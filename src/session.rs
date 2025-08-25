//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use nix::unistd::geteuid;
use core::panic;
#[cfg(feature = "threaded")]
use std::fmt;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{
    AtomicBool, AtomicU32,
    Ordering::{Acquire, Relaxed},
};
use std::sync::{Arc, Mutex};
#[cfg(feature = "threaded")]
use std::thread::{self, JoinHandle};
use std::time::Duration;
use crate::any::{_Nl, _Ns, _Na};
/*
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
*/
use crate::{AnyFS, MountOption};
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::notify::{Queues, Notifier};
#[cfg(feature = "abi-7-40")]
use crate::passthrough::BackingHandler;
use crate::trait_async::Filesystem as AsyncFS;
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
#[cfg(feature = "abi-7-11")]
use crate::LegacyNotifier;

pub const SYNC_SLEEP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

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
    #[cfg(feature = "abi-7-11")]
    /// Whether this session currently has notification support
    pub(crate) notify: AtomicBool,
    /// Whether this session does heartbeats, if nonzero
    pub(crate) heartbeat_interval: Duration,
}

/// The session data structure
#[derive(Debug)]
pub struct Session<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    // TODO: if threaded, filesystem: Mutex<AnyFS<...>>
    /// Filesystem operation implementations
    pub(crate) filesystem: AnyFS<L, S, A>,
    /// Main communication channel to the kernel fuse driver
    pub(crate) ch_main: Channel,
    #[cfg(feature = "side_channel")]
    /// Side communication channel to the kernel fuse driver
    pub(crate) ch_side: Channel,
    #[cfg(feature = "abi-7-11")]
    /// Side communication with the filesystem.
    pub(crate) queues: Queues,
    /// Handle to the mount.  Dropping this unmounts.
    pub(crate) mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Describes the configurable state of the Session
    pub(crate) meta: SessionMeta,
}

impl AsFd for Session<'_ , '_, '_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ch_main.as_fd()
    }
}

impl<L, S, A> Session<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    /// Create a new session by mounting the given filesystem to the given mountpoint
    pub fn new<P: AsRef<Path>>(
        filesystem: AnyFS<L, S, A>,
        mountpoint: P,
        options: &[MountOption],
    ) -> io::Result<Self> {
        let mut se = SessionBuilder::new();
        se.set_filesystem(filesystem);
        se.mount_path(mountpoint, options)?;
        Ok(se.build())
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: AnyFS<L, S, A>, fd: OwnedFd, acl: SessionACL) -> io::Result<Self> {
        // Create the channel for fuse messages
        let mut se = SessionBuilder::new();
        se.set_filesystem(filesystem);
        se.set_fuse_fd(fd, acl)?;
        Ok(se.build())
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> NotificationHandler {
        #[cfg(not(feature = "side-channel"))]
        let this_ch = self.ch_main.clone();
        #[cfg(feature = "side-channel")]
        let this_ch = self.ch_side.clone();
        NotificationHandler::new(this_ch)
    }

    /// Returns an object that can be used to queue notifications for the Session to process
    #[cfg(feature = "abi-7-11")]
    pub fn get_notification_sender(&self) -> Notifier {
        // Sync/Async notification method
        self.meta.notify.store(true, Relaxed);
        Notifier::new(self.queues.sender.clone())
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

impl<L, S, A> Session<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    /// This function starts the main Session loop of a Legacy or Sync Filesystem in the current thread.
    /// # Panics
    /// Panics if the filesystem is Async. Hint: try `run_blocking()` or `run_async()`
    pub fn run(self) -> io::Result<()> {
        match &self.filesystem {
            AnyFS::Legacy(_) => self.run_legacy(),
            AnyFS::Sync(_) => self.run_sync(),
            AnyFS::Async(_) => {
                panic!("Attempted to use Legacy/Sync method on an Async filesystem.")
            }
        }
    }
}

impl<L, S, A> Session<L, S, A>
where
    L: LegacyFS + Send + Sync + 'static,
    S: SyncFS + Send + Sync + 'static,
    A: AsyncFS + Send + Sync + 'static,
{
    /// This function starts the main Session loop of Any Filesystem, blocking the current thread.
    pub fn run_blocking(self) -> io::Result<()> {
        match &self.filesystem {
            AnyFS::Legacy(_) => self.run_legacy(),
            AnyFS::Sync(_) => self.run_sync(),
            AnyFS::Async(_) => futures::executor::block_on(self.run_async()),
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
impl<L, S, A> Session<L, S, A>
where
    L: LegacyFS + Send + Sync + 'static,
    S: SyncFS + Send + Sync + 'static,
    A: AsyncFS + Send + Sync + 'static,
{
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}

impl<L, S, A> Drop for Session<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    fn drop(&mut self) {
        if !self.meta.destroyed.load(Acquire) {
            match &mut self.filesystem {
                AnyFS::Legacy(fs) => fs.destroy(),
                AnyFS::Sync(fs) => fs.destroy(),
                AnyFS::Async(fs) => futures::executor::block_on(fs.destroy()),
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
    pub guard: JoinHandle<io::Result<()>>,
    /// Object for creating Notifiers for client use
    /*#[cfg(feature = "abi-7-11")]
    sender: Channel,*/
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Option<Mount>,
}

#[cfg(feature = "threaded")]
impl BackgroundSession {
    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<L, S, A>(se: Session<L, S, A>) -> io::Result<BackgroundSession>
    where
        L: LegacyFS + Send + Sync + 'static,
        S: SyncFS + Send + Sync + 'static,
        A: AsyncFS + Send + Sync + 'static,
    {
        #[cfg(all(feature = "abi-7-11", not(feature = "side-channel")))]
        let sender = se.ch_main.clone();
        #[cfg(all(feature = "abi-7-11", feature = "side-channel"))]
        let sender = se.ch_side.clone();
        // Take the fuse_session, so that we can unmount it
        let mount = std::mem::take(&mut *se.mount.lock().unwrap()).map(|(_, mount)| mount);
        // The main session (se) is moved into this thread.
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
        // Unmount the filesystem
        drop(_mount);
        // Stop the background thread
        let res = guard
            .join()
            .expect("Failed to join the background thread");
        // An error is expected, since the thread was active when the unmount occured.
        info!("Session loop end with result {res:?}.");
    }

    /*
    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> Notifier {
        Notifier::new(self.sender.clone())
    }
    */
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
#[cfg(feature = "threaded")]
impl fmt::Debug for BackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut builder = f.debug_struct("BackgroundSession");
        builder.field("main_loop_guard", &self.guard);
        /*#[cfg(feature = "abi-7-11")]
        {
            builder.field("sender", &self.sender);
        }*/
        builder.field("_mount", &self._mount);
        builder.finish()
    }
}

#[derive(Debug)]
/// SessionBuilder provides an alternative interface for 
pub struct SessionBuilder<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    // TODO: if threaded, filesystem: Mutex<AnyFS<...>>
    /// Filesystem operation implementations
    pub(crate) filesystem: Option<AnyFS<L, S, A>>,
    /// Main communication channel to the kernel fuse driver
    pub(crate) ch_main: Option<Channel>,
    #[cfg(feature = "side_channel")]
    /// Side communication channel to the kernel fuse driver
    pub(crate) ch_side: Option<Channel>,
    #[cfg(feature = "abi-7-11")]
    /// Side communication with the filesystem.
    pub(crate) queues: Queues,
    /// Handle to the mount.  Dropping this unmounts.
    pub(crate) mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Describes the configurable state of the Session
    pub(crate) meta: SessionMeta,
}

impl<L, S, A> SessionBuilder<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    /// Create a new SessionBuilder with no Filesystem, no Mount, and default options.
    pub fn new() -> Self {
        SessionBuilder {
            filesystem: None,
            ch_main: None,
            #[cfg(feature = "side_channel")]
            ch_side: None,
            #[cfg(feature = "abi-7-11")]
            queues: Queues::new(),
            mount: Arc::new(Mutex::new(None)),
            meta: SessionMeta {
                allowed: SessionACL::Owner,
                session_owner: geteuid().as_raw(),
                proto_major: AtomicU32::new(0),
                proto_minor: AtomicU32::new(0),
                initialized: AtomicBool::new(false),
                destroyed: AtomicBool::new(false),
                #[cfg(feature = "abi-7-11")]
                notify: AtomicBool::new(false),
                heartbeat_interval: Duration::from_secs(0),
            }
        }
    }
    /// Open a new fuse file descriptor and use it to mount this Session at the target path.
    pub fn mount_path<P: AsRef<Path>>(
        &mut self,
        mountpoint: P,
        options: &[MountOption],
    ) -> io::Result<()> {
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
        self.set_ch(ch)?;
        let mount = Some((mountpoint.to_owned(), mount));
        let mut guard = self.mount.lock().unwrap();
        *guard = mount;
        self.meta.allowed = allowed;
        Ok(())
    }
    /// Set a Filesystem (of Any type) for this Session.
    pub fn set_filesystem(&mut self, fs: AnyFS<L, S, A>) {
        self.filesystem = Some(fs);
    }
    /// Set a heartbeat interval for this Session. 
    /// If zero (default), session will not send heartbeats.
    pub fn set_heartbeat_interval(&mut self, interval: Duration) {
        self.meta.heartbeat_interval = interval;
    }
    /// Get a Notifier for this Session.
    /// Can be used to send Notifications.
    /// Enables Notifications for this session.
    #[cfg(feature = "abi-7-11")]
    pub fn get_notification_sender(&mut self) -> Notifier {
        self.meta.notify.store(true, Relaxed);
        // Sync/Async notification method
        Notifier::new(self.queues.sender.clone())
    }
    /// Get a BackingHandler for this Session.
    /// Can be used to open and close BackingId.
    /// Enables Notifications for this session.
    #[cfg(feature = "abi-7-40")]
    pub fn get_backing_handler(&mut self) -> BackingHandler {
        if let Some(ch) = &self.ch_main {
            self.meta.notify.store(true, Relaxed);
            BackingHandler::new(
                ch.clone(),
                self.queues.sender.clone(),
            )
        } else {
            panic!("No fuse channel! Did you forget to mount?");
        } 
    }
    /// Set a fuse file descriptor without mounting the Session.
    /// If this method is used, mounting must be done elsewhere.
    pub fn set_fuse_fd(&mut self, fd: OwnedFd, acl: SessionACL) -> io::Result<()> {
        let ch = Channel::new(fd.into());
        self.set_ch(ch)?;
        let mut guard = self.mount.lock().unwrap();
        *guard = None;
        self.meta.allowed = acl;
        Ok(())
    }
    /// Sets the main and side fuse channels. Internal use only.
    fn set_ch(&mut self, ch: Channel) -> io::Result<()> {
        // Create the channel for fuse messages
        #[cfg(feature = "side_channel")]
        {
            self.ch_side = Some(ch.fork()?);
        }
        self.ch_main = Some(ch);
        Ok(())
    }
    /// Assemble the parts of the SessionBuilder into a Session ready for use.
    pub fn build(self) -> Session<L, S, A> {
        Session {
            filesystem: self.filesystem
                .expect("No filesystem! Did you forget to set one?"),
            ch_main: self.ch_main
                .expect("No fuse channel! Did you forget to mount?"),
            #[cfg(feature = "side_channel")]
            ch_side: self.ch_side
                .expect("No fuse channel! Did you forget to mount?"),
            mount: self.mount,
            #[cfg(feature = "abi-7-11")]
            queues: self.queues,
            meta: self.meta,
        }
    }
}
// Additional, trait-specific build functions.
impl<L> SessionBuilder<L, _Ns, _Na>
where
    L: LegacyFS,
{
    /// Set a Legacy Filesystem for this Session.
    pub fn set_filesystem_legacy(&mut self, fs: L) {
        let any = AnyFS::<L, _Ns, _Na>::Legacy(fs);
        self.filesystem = Some(any);
    }
}
impl<S> SessionBuilder<_Nl, S, _Na>
where
    S: SyncFS,
{
    /// Set a Synchronous Filesystem for this Session.
    pub fn set_filesystem_sync(&mut self, fs: S) {
        let any = AnyFS::<_Nl, S, _Na>::Sync(fs);
        self.filesystem = Some(any);
    }
}
impl<A> SessionBuilder<_Nl, _Ns, A>
where
    A: AsyncFS,
{
    /// Set an Asynchronous Filesystem for this Session.
    pub fn set_filesystem_async(&mut self, fs: A) {
        let any = AnyFS::<_Nl, _Ns, A>::Async(fs);
        self.filesystem = Some(any);
    }
}