//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.

#[allow(unused_imports)]
use log::{debug, info, warn, error};
use nix::unistd::geteuid;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::{Relaxed, Acquire}};
use std::sync::{Arc, Mutex};
use std::io;
#[cfg(feature = "threaded")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "threaded")]
use std::fmt;

use crate::{MountOption, AnyFS};
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Sender, Receiver};

use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

/// The max size of write requests from the kernel. The absolute minimum is 4k,
/// FUSE recommends at least 128k, max 16M. The FUSE default is 16M on macOS
/// and 128k on other systems.
pub const MAX_WRITE_SIZE: usize = 16 * 1024 * 1024;

/// Size of the buffer for reading a request from the kernel. Since the kernel may send
/// up to `MAX_WRITE_SIZE` bytes in a write request, we use that value plus some extra space.
pub const BUFFER_SIZE: usize = MAX_WRITE_SIZE + 4096;

#[derive(Default, Eq, PartialEq, Debug)]
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

/// Session metadata
#[derive(Debug)]
pub(crate) struct SessionMeta {
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
    #[cfg(feature = "abi-7-11")]
    /// Whether this session currently has notification support
    pub(crate) notify: AtomicBool,
}

/// The session data structure
#[derive(Debug)]
pub struct Session<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS
{
    /// Filesystem operation implementations
    pub(crate) filesystem: AnyFS<L, S, A>,
    /// Communication channels to the kernel fuse driver
    pub(crate) chs: Vec<Channel>,
    /// Handle to the mount.  Dropping this unmounts.
    pub(crate) mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Whether to restrict access to owner, root + owner, or unrestricted
    /// Used to implement `allow_root` and `auto_unmount`
    pub(crate) meta: SessionMeta,
    #[cfg(feature = "abi-7-11")]
    /// Sender for poll events to the filesystem. It will be cloned and passed to Filesystem.
    pub(crate) ns: Sender<Notification>,
    #[cfg(feature = "abi-7-11")]
    /// Receiver for poll events from the filesystem.
    pub(crate) nr: Receiver<Notification>,
}

impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS 
{
    /// Create a new session by mounting the given filesystem to the given mountpoint
    pub fn new_mounted<P: AsRef<Path>>(
        filesystem: AnyFS<L, S, A>,
        mountpoint: P,
        options: &[MountOption],
    ) -> io::Result<Session<L, S, A>> {
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
            warn!("Given auto_unmount without allow_root or allow_other; adding allow_other, with userspace permission handling");
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
        Ok(Session::new(filesystem, ch, allowed, mount))
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: AnyFS<L, S, A>, fd: OwnedFd, allowed: SessionACL) -> Self {
        // Create the channel for fuse messages
        let ch = Channel::new(fd.into());
        Session::new(filesystem, ch, allowed, None)
    }

    /// Assemble a Session from raw parts. Not recommended for regular users. 
    pub(crate) fn new(mut filesystem: AnyFS<L, S, A>, ch: Channel, allowed: SessionACL, mount: Option<(PathBuf, Mount)>) -> Self {
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (ns, nr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        // Pass the sender to the filesystem.
        let notify = match &mut filesystem {
            AnyFS::Legacy(lfs) => lfs.init_notification_sender(ns.clone()),
            AnyFS::Sync(sfs) => sfs.init_notification_sender(ns.clone()),
            AnyFS::Async(afs) => afs.init_notification_sender(ns.clone())
        };
        let meta = SessionMeta {
            allowed,
            session_owner: geteuid().as_raw(),
            proto_major: AtomicU32::new(0),
            proto_minor: AtomicU32::new(0),
            initialized: AtomicBool::new(false),
            destroyed: AtomicBool::new(false),
            #[cfg(feature = "abi-7-11")]
            notify: AtomicBool::new(notify),
        };
        Session {
            filesystem,
            chs: vec![ch],
            mount: Arc::new(Mutex::new(mount)),
            meta,
            #[cfg(feature = "abi-7-11")]
            ns,
            #[cfg(feature = "abi-7-11")]
            nr,
        }
    }

    /// Returns an object that can be used to send notifications directly to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> Notifier {
        // Legacy notification method
        Notifier::new(self.chs[0].clone())
    }

    /// Returns an object that can be used to queue notifications for the Session to process
    #[cfg(feature = "abi-7-11")]
    pub fn get_notification_sender(&self) -> Sender<Notification> {
        // Sync/Async notification method
        self.ns.clone()
    }

    /// Creates additional worker fuse file descriptors until the
    /// Session has at least `channel_count` communication channels.
    /// Reports the number of channels that were created.
    /// # Errors
    /// Propagates underlying errors. 
    pub fn set_channels(&mut self, channel_count: u32) -> Result<u32, io::Error> {
        let main_fuse_fd = self.chs[0].raw_fd as u32;
        let mut new_channels = 0;
        while self.chs.len() < channel_count as usize {
            let fuse_device_name = "/dev/fuse";
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fuse_device_name)?;
            let ch = Channel::new(file);
            ch.new_fuse_worker(main_fuse_fd)?;
            self.chs.push(ch);
            new_channels += 1;
        }
        Ok(new_channels)
    }

    /// returns a copy of the channel associated with a specific channel index.
    pub(crate) fn get_ch(&self, ch_idx: usize) -> Channel {
        self.chs[ch_idx].clone()
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
}

impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS
{
    /// This function starts the main Session loop of a Legacy or Sync Filesystem in the current thread. 
    /// # Panics
    /// Panics if the filesystem is Async. Hint: try `run_blocking()` or `run_async()`
    pub fn run(self) -> io::Result<()> {
        match &self.filesystem {
            AnyFS::Legacy(_) => self.run_legacy(),
            AnyFS::Sync(_) => self.run_sync(),
            AnyFS::Async(_) => panic!("Attempted to use Legacy/Sync method on an Async filesystem.")
        }
    }
}

impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS + Send + Sync,
    S: SyncFS + Send + Sync,
    A: AsyncFS + Send + Sync
{
    /// This function starts the main Session loop of Any Filesystem, blocking the current thread.
    pub fn run_blocking(self) -> io::Result<()> {
        match &self.filesystem {
            AnyFS::Legacy(_) => self.run_legacy(),
            AnyFS::Sync(_) => self.run_sync(),
            AnyFS::Async(_) => futures::executor::block_on(self.run_async())
        }
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

/// A session can be run synchronously in the current thread using `run()` or spawned into a
/// background thread using `spawn()`.
#[cfg(feature = "threaded")]
impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS + Send + Sync + 'static,
    S: SyncFS + Send + Sync + 'static,
    A: AsyncFS + Send + Sync + 'static
{
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}

impl<L, S, A> Drop for Session<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{    
    fn drop(&mut self) {
        if !self.meta.destroyed.load(Acquire) {
            match &mut self.filesystem {
                AnyFS::Legacy(fs) => fs.destroy(),
                AnyFS::Sync(fs) => fs.destroy(),
                AnyFS::Async(fs) => {
                    futures::executor::block_on(fs.destroy())
                }
            }
            self.meta.destroyed.store(true, Relaxed);
        }

        if let Some((mountpoint, _mount)) = std::mem::take(&mut *self.mount.lock().unwrap()) {
            info!("unmounting session at {}", mountpoint.display());
        }
    }
}

/// The background session data structure
#[cfg(feature = "threaded")]
pub struct BackgroundSession {
    /// Thread guard of the main session loop
    pub main_loop_guard: JoinHandle<io::Result<()>>,
    /// Object for creating Notifiers for client use
    #[cfg(feature = "abi-7-11")]
    extra_notification_sender: Sender<Notification>,
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Option<Mount>,
}

#[cfg(feature = "threaded")]
impl BackgroundSession {

    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<L, S, A>(se: Session<L, S, A>) -> io::Result<BackgroundSession> where 
        L: LegacyFS + Send + Sync + 'static,
        S: SyncFS + Send + Sync +'static,
        A: AsyncFS + Send + Sync + 'static
    {
        #[cfg(feature = "abi-7-11")]
        let extra_notification_sender = se.ns.clone();

        let mount = std::mem::take(&mut *se.mount.lock().unwrap()).map(|(_, mount)| mount);

        #[cfg(not(feature = "abi-7-11"))]
        // The main session (se) is moved into this thread.
        let main_loop_guard = thread::spawn(move || {
            futures::executor::block_on(se.run())
        });
        #[cfg(feature = "abi-7-11")]
        let main_loop_guard = thread::spawn(move || {
            futures::executor::block_on(se.run_concurrently_sequential_full())
        });

        Ok(BackgroundSession {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            extra_notification_sender,
            _mount: mount,
        })
    }
    /// Unmount the filesystem and join the background thread.
    pub fn join(self) {
        let Self {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            extra_notification_sender: _,
            _mount,
        } = self;
        // Unmount the filesystem
        drop(_mount);
        // Stop the background thread
        let res = main_loop_guard.join()
            .expect("Failed to join the background thread");
        // An error is expected, since the thread was active when the unmount occured.
        info!("Session loop end with result {res:?}.");
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    #[must_use]
    pub fn get_notification_sender(&self) -> Sender<Notification> {
       self.extra_notification_sender.clone()
    }
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
#[cfg(feature = "threaded")]
impl fmt::Debug for BackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut builder = f.debug_struct("BackgroundSession");
        builder.field("main_loop_guard", &self.main_loop_guard);
        #[cfg(feature = "abi-7-11")]
        {
            builder.field("sender", &self.extra_notification_sender);
        }
        builder.field("_mount", &self._mount);
        builder.finish()
    }
}