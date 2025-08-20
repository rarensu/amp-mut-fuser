//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use nix::unistd::geteuid;
use std::fmt;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{
    AtomicBool, AtomicU32,
    Ordering::{Acquire, Relaxed},
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::io;

use crate::Filesystem;
use crate::MountOption;
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::notify::NotificationHandler;

/// The max size of write requests from the kernel. The absolute minimum is 4k,
/// FUSE recommends at least 128k, max 16M. The FUSE default is 16M on macOS
/// and 128k on other systems.
pub const MAX_WRITE_SIZE: usize = 16 * 1024 * 1024;

/// Size of the buffer for reading a request from the kernel. Since the kernel may send
/// up to MAX_WRITE_SIZE bytes in a write request, we use that value plus some extra space.
pub const BUFFER_SIZE: usize = MAX_WRITE_SIZE + 4096;

#[derive(Default, Debug, Eq, PartialEq)]
/// How requests should be filtered based on the calling UID.
pub enum SessionACL {
    /// Allow requests from any user. Corresponds to the `allow_other` mount option.
    All,
    /// Allow requests from root. Corresponds to the `allow_root` mount option.
    RootAndOwner,
    /// Allow requests from the owning UID. This is FUSE's default mode of operation.
    #[default]
    Owner,
}

/// The session data structure
#[derive(Debug)]
pub struct Session<FS: Filesystem> {
    /// Filesystem operation implementations
    pub(crate) filesystem: FS,
    /// Main communication channel to the kernel fuse driver
    pub(crate) ch_main: Channel,
    #[cfg(feature = "side-channel")]
    /// Side communication channel to the kernel fuse driver
    pub(crate) ch_side: Channel,
    /// Handle to the mount.  Dropping this unmounts.
    pub(crate) mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Session metadata
    pub(crate) meta: SessionMeta,
}

/// Session metadata
#[derive(Debug)]
pub(crate) struct SessionMeta {
    /// Whether to restrict access to owner, root + owner, or unrestricted
    /// Used to implement allow_root and auto_unmount
    pub(crate) allowed: SessionACL,
    /// User that launched the fuser process
    pub(crate) session_owner: u32,
    /// FUSE protocol major version
    pub(crate) proto_major: AtomicU32,
    /// FUSE protocol minor version
    pub(crate) proto_minor: AtomicU32,
    /// True if the filesystem is initialized (init operation done)
    pub(crate) initialized: AtomicBool,
    /// True if the filesystem was destroyed (destroy operation done)
    pub(crate) destroyed: AtomicBool,
}

impl<FS: Filesystem> AsFd for Session<FS> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ch_main.as_fd()
    }
}

impl<FS: Filesystem> Session<FS> {
    /// Create a new session by mounting the given filesystem to the given mountpoint
    pub fn new<P: AsRef<Path>>(
        filesystem: FS,
        mountpoint: P,
        options: &[MountOption],
    ) -> io::Result<Session<FS>> {
        let mountpoint = mountpoint.as_ref();
        info!("Mounting {}", mountpoint.display());
        // Create the channel for fuse messages
        let (ch, mount) = if options.contains(&MountOption::AutoUnmount)
            && !(options.contains(&MountOption::AllowRoot)
                || options.contains(&MountOption::AllowOther))
        {
            // If AutoUnmount is requested, but not AllowRoot or AllowOther we enforce the ACL
            // ourself and implicitly set AllowOther because fusermount needs allow_root or allow_other
            // to handle the auto_unmount option
            warn!(
                "Given auto_unmount without allow_root or allow_other; adding allow_other, with userspace permission handling"
            );
            let mut modified_options = options.to_vec();
            modified_options.push(MountOption::AllowOther);
            Mount::new(mountpoint, &modified_options)?
        } else {
            Mount::new(mountpoint, options)?
        };
        let allowed = if options.contains(&MountOption::AllowRoot) {
            SessionACL::RootAndOwner
        } else if options.contains(&MountOption::AllowOther) {
            SessionACL::All
        } else {
            SessionACL::Owner
        };
        let mount = Some((mountpoint.to_owned(), mount));
        Ok(Session::from_mount(filesystem, ch, allowed, mount))
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: FS, fd: OwnedFd, acl: SessionACL) -> Self {
        // Create the channel for fuse messages
        let ch = Channel::new(fd.into());
        Session::from_mount(filesystem, ch, acl, None)
    }

    /// Assemble a Session that has been mounted. Not recommended for regular users.
    pub(crate) fn from_mount(
        filesystem: FS,
        ch: Channel,
        allowed: SessionACL,
        mount: Option<(PathBuf, Mount)>,
    ) -> Self {
        let meta = SessionMeta {
            allowed,
            session_owner: geteuid().as_raw(),
            proto_major: AtomicU32::new(0),
            proto_minor: AtomicU32::new(0),
            initialized: AtomicBool::new(false),
            destroyed: AtomicBool::new(false),
        };
        Session {
            filesystem,
            ch_main: ch,
            #[cfg(feature = "side-channel")]
            ch_side: ch,
            mount: Arc::new(Mutex::new(mount)),
            meta,
        }
    }

    /// This function starts the main Session loop of a Legacy Filesystem in the current thread.
    pub fn run(mut self) -> io::Result<()> {
        self.run_legacy()
    }

    /// Unmount the filesystem
    pub fn unmount(&mut self) {
        drop(std::mem::take(&mut *self.mount.lock().unwrap()));
    }

    /// Returns a thread-safe object that can be used to unmount the Filesystem
    pub fn unmount_callable(&mut self) -> SessionUnmounter {
        SessionUnmounter {
            mount: self.mount.clone(),
        }
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> NotificationHandler {
        NotificationHandler::new(self.ch.clone())
    }
}

#[derive(Debug)]
/// A thread-safe object that can be used to unmount a Filesystem
pub struct SessionUnmounter {
    mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
}

impl SessionUnmounter {
    /// Unmount the filesystem
    pub fn unmount(&mut self) -> io::Result<()> {
        drop(std::mem::take(&mut *self.mount.lock().unwrap()));
        Ok(())
    }
}

impl<FS: 'static + Filesystem + Send> Session<FS> {
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}

impl<FS: Filesystem> Drop for Session<FS> {
    fn drop(&mut self) {
        if !self.meta.destroyed.load(Acquire) {
            self.filesystem.destroy();
            self.meta.destroyed.store(true, Relaxed);
        }

        if let Some((mountpoint, _mount)) = std::mem::take(&mut *self.mount.lock().unwrap()) {
            info!("unmounting session at {}", mountpoint.display());
        }
    }
}

/// The background session data structure
pub struct BackgroundSession {
    /// Thread guard of the background session
    pub guard: JoinHandle<io::Result<()>>,
    /// Object for creating Notifiers for client use
    #[cfg(feature = "abi-7-11")]
    sender: Channel,
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Option<Mount>,
}

impl BackgroundSession {
    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<FS: Filesystem + Send + 'static>(se: Session<FS>) -> io::Result<BackgroundSession> {
        #[cfg(feature = "abi-7-11")]
        let sender = se.ch.clone();
        // Take the fuse_session, so that we can unmount it
        let mount = std::mem::take(&mut *se.mount.lock().unwrap()).map(|(_, mount)| mount);
        let guard = thread::spawn(move || {
            //let mut se = se;
            se.run()
        });
        Ok(BackgroundSession {
            guard,
            #[cfg(feature = "abi-7-11")]
            sender,
            _mount: mount,
        })
    }
    /// Unmount the filesystem and join the background thread.
    pub fn join(self) {
        let Self {
            guard,
            #[cfg(feature = "abi-7-11")]
                sender: _,
            _mount,
        } = self;
        drop(_mount);
        guard.join().unwrap().unwrap();
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> NotificationHandler {
        NotificationHandler::new(self.sender.clone())
    }
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
impl fmt::Debug for BackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "BackgroundSession {{ guard: JoinGuard<()> }}",)
    }
}
