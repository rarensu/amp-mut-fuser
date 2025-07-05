//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.
///
/// A session can be run synchronously in the current thread using `run()`, spawned into a
/// background thread using `spawn()`, or run in a single-threaded mode that handles
/// both FUSE requests and poll notifications using `run_single_threaded()`.

use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use nix::unistd::geteuid;
use std::fmt;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::{io, ops::DerefMut};

use crate::ll::fuse_abi as abi;
use crate::request::Request;
use crate::{Filesystem, FsStatus};
use crate::MountOption;
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::{channel::ChannelSender, notify::Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Sender, Receiver};


/// The max size of write requests from the kernel. The absolute minimum is 4k,
/// FUSE recommends at least 128k, max 16M. The FUSE default is 16M on macOS
/// and 128k on other systems.
pub const MAX_WRITE_SIZE: usize = 16 * 1024 * 1024;

/// Size of the buffer for reading a request from the kernel. Since the kernel may send
/// up to MAX_WRITE_SIZE bytes in a write request, we use that value plus some extra space.
const BUFFER_SIZE: usize = MAX_WRITE_SIZE + 4096;

const SYNC_SLEEP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

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
    /// Communication channel to the kernel driver
    pub(crate) ch: Channel,
    /// Handle to the mount.  Dropping this unmounts.
    mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Whether to restrict access to owner, root + owner, or unrestricted
    /// Used to implement allow_root and auto_unmount
    pub(crate) allowed: SessionACL,
    /// User that launched the fuser process
    pub(crate) session_owner: u32,
    /// FUSE protocol major version
    pub(crate) proto_major: u32,
    /// FUSE protocol minor version
    pub(crate) proto_minor: u32,
    /// True if the filesystem is initialized (init operation done)
    pub(crate) initialized: bool,
    /// True if the filesystem was destroyed (destroy operation done)
    pub(crate) destroyed: bool,
    #[cfg(feature = "abi-7-11")]
    /// Whether this session currently has poll support
    pub(crate) poll_enabled: bool,
    /// Sender for poll events to the filesystem. It will be cloned and passed to Filesystem.
    #[cfg(feature = "abi-7-11")]
    pub(crate) poll_event_sender: Sender<(u64, u32)>,
    /// Receiver for poll events from the filesystem.
    #[cfg(feature = "abi-7-11")]
    pub(crate) poll_event_receiver: Receiver<(u64, u32)>,
}

impl<FS: Filesystem> AsFd for Session<FS> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ch.as_fd()
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
        // If AutoUnmount is requested, but not AllowRoot or AllowOther we enforce the ACL
        // ourself and implicitly set AllowOther because fusermount needs allow_root or allow_other
        // to handle the auto_unmount option
        let (file, mount) = if options.contains(&MountOption::AutoUnmount)
            && !(options.contains(&MountOption::AllowRoot)
                || options.contains(&MountOption::AllowOther))
        {
            warn!("Given auto_unmount without allow_root or allow_other; adding allow_other, with userspace permission handling");
            let mut modified_options = options.to_vec();
            modified_options.push(MountOption::AllowOther);
            Mount::new(mountpoint, &modified_options)?
        } else {
            Mount::new(mountpoint, options)?
        };
        // Create the channel for fuse messages
        let ch = Channel::new(file);
        let allowed = if options.contains(&MountOption::AllowRoot) {
            SessionACL::RootAndOwner
        } else if options.contains(&MountOption::AllowOther) {
            SessionACL::All
        } else {
            SessionACL::Owner
        };
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (pxs, pxr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        let mut filesystem = filesystem;
        // Pass the sender to the filesystem.
        #[cfg(feature = "abi-7-11")]
        let poll_enabled = match filesystem.init_poll_sender(pxs.clone()) {
            Err(e) => {
                // Log an error if the filesystem explicitely states it does not support polling.
                // ENOSYS is the default from the trait if not implemented.
                if e != crate::Errno::ENOSYS {
                    warn!("Filesystem failed to initialize poll sender: {:?}. Channel-based polling might not work as expected.", e);
                } else {
                    info!("Filesystem does not implement init_poll_sender (ENOSYS). Assuming no channel-based poll support or uses legacy poll.");
                }
                // Proceeding even if init_poll_sender fails, as FS might use legacy poll or no poll.
                // The poll_event_loop will still be spawned if abi-7-11 is enabled,
                // but it might not receive anything if FS doesn't use the sender.
                false
            },
            Ok(()) => true
        };
        let new_session = Session {
            filesystem,
            ch,
            mount: Arc::new(Mutex::new(Some((mountpoint.to_owned(), mount)))),
            allowed,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            poll_enabled,
            #[cfg(feature = "abi-7-11")]
            poll_event_sender: pxs,
            #[cfg(feature = "abi-7-11")]
            poll_event_receiver: pxr,
        };
        Ok(new_session)
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: FS, fd: OwnedFd, acl: SessionACL) -> Self {
        // Create the channel for fuse messages
        let ch = Channel::new(Arc::new(fd.into()));
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (pxs, pxr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        let mut filesystem = filesystem;
        #[cfg(feature = "abi-7-11")]
        let poll_enabled = match filesystem.init_poll_sender(pxs.clone()) {
            Err(e) => {
                // Log an error if the filesystem explicitely states it does not support polling.
                // ENOSYS is the default from the trait if not implemented.
                if e != crate::Errno::ENOSYS {
                    warn!("Filesystem failed to initialize poll sender: {:?}. Channel-based polling might not work as expected.", e);
                } else {
                    info!("Filesystem does not implement init_poll_sender (ENOSYS). Assuming no channel-based poll support or uses legacy poll.");
                }
                // Proceeding even if init_poll_sender fails, as FS might use legacy poll or no poll.
                // The poll_event_loop will still be spawned if abi-7-11 is enabled,
                // but it might not receive anything if FS doesn't use the sender.
                false
            },
            Ok(()) => true
        };
        Session {
            filesystem,
            ch,
            mount: Arc::new(Mutex::new(None)),
            allowed: acl,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            poll_enabled,
            #[cfg(feature = "abi-7-11")]
            poll_event_sender: pxs,
            #[cfg(feature = "abi-7-11")]
            poll_event_receiver: pxr,
        }
    }

    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub fn run(&mut self) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = aligned_sub_buf(
            buffer.deref_mut(),
            std::mem::align_of::<abi::fuse_in_header>(),
        );
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            match self.ch.receive(buf) {
                Ok(size) => match Request::new(self.ch.sender(), &buf[..size]) {
                    // Dispatch request
                    Some(req) => req.dispatch(self),
                    // Quit loop on illegal request
                    None => break,
                },
                Err(err) => match err.raw_os_error() {
                    // Operation interrupted. Accordingly to FUSE, this is safe to retry
                    Some(ENOENT) => continue,
                    // Interrupted system call, retry
                    Some(EINTR) => continue,
                    // Explicitly try again
                    Some(EAGAIN) => continue,
                    // Filesystem was unmounted, quit the loop
                    Some(ENODEV) => break,
                    // Unhandled error
                    _ => return Err(err),
                },
            }
        }
        Ok(())
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
    pub fn notifier(&self) -> Notifier {
        Notifier::new(self.ch.sender())
    }

    /// Returns an object that can be used to send poll event notifications
    #[cfg(feature = "abi-7-11")]
    pub fn get_poll_sender(&self) -> Sender<(u64, u32)> {
        self.poll_event_sender.clone()
    }

    /// Run the session loop in a single thread, same as run(), but additionally
    /// processing both FUSE requests and poll events without blocking.
    pub fn run_single_threaded(&mut self) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = aligned_sub_buf(
            buffer.deref_mut(),
            std::mem::align_of::<abi::fuse_in_header>(),
        );

        info!("Running FUSE session in single-threaded mode");

        loop {
            let mut work_done = false;
            // 1. Check for and process pending poll events (non-blocking)
            #[cfg(feature = "abi-7-11")]
            if self.poll_enabled {
                match self.poll_event_receiver.try_recv() {
                    Ok((kh, events)) => {
                        // Note: Original plan mentioned calling self.notifier().poll(kh).
                        // The existing poll loop in BackgroundSession directly calls notifier.poll(ph).
                        // We'll replicate that behavior.
                        // The `events` variable from `poll_event_receiver` is not directly used by `Notifier::poll`,
                        // as `Notifier::poll` only takes `kh`. This matches existing behavior.
                        debug!("Processing poll event for kh: {}, events: {:x}", kh, events);
                        if let Err(e) = self.notifier().poll(kh) {
                            error!("Failed to send poll notification for kh {}: {}", kh, e);
                            // Decide if error is fatal. ENODEV might mean unmounted.
                            if e.raw_os_error() == Some(libc::ENODEV) {
                                warn!("FUSE device not available for poll notification, likely unmounted. Exiting.");
                                break;
                            }
                        }
                        work_done = true;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        // No poll events pending, proceed to check FUSE FD
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        // Filesystem's poll event sender side dropped.
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed.
                        warn!("Poll event channel disconnected by sender. No more poll events will be processed.");
                        self.poll_enabled=false;
                    }
                }
            }
            // Check for incoming FUSE requests (non-blocking)
            if work_done {
                // skip checking for incoming FUSE requests, for now,
                // to prioritize checking for additional outgoing messages
                continue;
            }
            match self.ch.ready() {
                Err(err) => {
                    if err.raw_os_error() == Some(EINTR) {
                        debug!("Poll interrupted, will retry.");
                    } else {
                        error!("Error polling FUSE FD: {}", err);
                        // Assume very bad. Stop the run. TODO: maybe some handling. 
                        return Err(err);
                    }
                },
                Ok( ready) => {
                    if ready {
                        // Read a FUSE request (blocks until read succeeds)
                        match self.ch.receive(buf) {
                            Ok(size) => {
                                if size == 0 {
                                    // Read of 0 bytes on FUSE FD typically means it was closed (unmounted)
                                    info!("FUSE channel read 0 bytes, session ending.");
                                    break;
                                }
                                match Request::new(self.ch.sender(), &buf[..size]) {
                                    Some(req) => req.dispatch(self),
                                    None => {
                                        warn!("Failed to parse FUSE request, session ending.");
                                        break; // Illegal request, quit loop
                                    }
                                }
                                work_done=true;
                            }
                            Err(err) => match err.raw_os_error() {
                                Some(ENOENT) => {
                                    debug!("FUSE channel receive ENOENT, retrying.");
                                    continue;
                                }
                                Some(EINTR) => {
                                    debug!("FUSE channel receive EINTR, retrying.");
                                    continue;
                                }
                                Some(EAGAIN) => {
                                    debug!("FUSE channel receive EAGAIN, retrying.");
                                    continue;
                                }
                                Some(ENODEV) => {
                                    info!("FUSE device not available (ENODEV), session ending.");
                                    break; // Filesystem was unmounted
                                }
                                _ => {
                                    error!("Error receiving FUSE request: {}", err);
                                    return Err(err); // Unhandled error
                                }
                            },
                        }
                    }

                }
            }
            if !work_done {
                // No actions taken this loop iteration.
                // Sleep briefly to yield CPU.
                std::thread::sleep(SYNC_SLEEP_INTERVAL);
                // Do a heartbeat to let the Filesystem know that some time has passed. 
                match FS::heartbeat(&mut self.filesystem) {
                    Ok(status) => {
                        match status {
                            FsStatus::Stopped => { break; }
                            _ => { }
                        }
                    }
                    Err(e) => {
                        warn!("Heartbeat error: {:?}", e);
                    }
                }
            }
        }
        Ok(())
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

fn aligned_sub_buf(buf: &mut [u8], alignment: usize) -> &mut [u8] {
    let off = alignment - (buf.as_ptr() as usize) % alignment;
    if off == alignment {
        buf
    } else {
        &mut buf[off..]
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
        if !self.destroyed {
            self.filesystem.destroy();
            self.destroyed = true;
        }

        if let Some((mountpoint, _mount)) = std::mem::take(&mut *self.mount.lock().unwrap()) {
            info!("unmounting session at {}", mountpoint.display());
        }
    }
}

/// The background session data structure
pub struct BackgroundSession {
    /// Thread guard of the main session loop
    pub main_loop_guard: JoinHandle<io::Result<()>>,
    /// Thread guard for the poll event notification loop
    #[cfg(feature = "abi-7-11")]
    pub poll_event_loop_guard: Option<JoinHandle<()>>,
    /// Object for creating Notifiers for client use
    #[cfg(feature = "abi-7-11")]
    sender: ChannelSender,
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Option<Mount>,
}

impl BackgroundSession {
    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<FS: Filesystem + Send + 'static>(mut se: Session<FS>) -> io::Result<BackgroundSession> {
        #[cfg(feature = "abi-7-11")]
        let poll_event_receiver_for_loop = se.poll_event_receiver.clone(); // Receiver is copied into the poll_event_loop_guard's thread
        #[cfg(feature = "abi-7-11")]
        let notifier_for_poll_loop = Notifier::new(se.ch.sender().clone()); // Notifier needs its own sender clone
        #[cfg(feature = "abi-7-11")]
        let extra_sender_clone = se.ch.sender().clone();

        let mount = std::mem::take(&mut *se.mount.lock().unwrap()).map(|(_, mount)| mount);

        #[cfg(not(feature = "abi-7-11"))]
        // The main session (se) is moved into this thread.
        let main_loop_guard = thread::spawn(move || {
            se.run()
        });
        #[cfg(feature = "abi-7-11")]
        let main_loop_guard = thread::spawn(move || {
            se.run_single_threaded()
        });

        #[cfg(feature = "abi-7-11")]
        let poll_event_loop_guard = {
            // Note: se.ch.sender() is used for the notifier, se.poll_event_receiver for this loop.
            info!("Spawning poll event notification thread.");
            Some(thread::spawn(move || {
                loop {
                    match poll_event_receiver_for_loop.recv() { // uses clone of receiver
                        Ok((ph, _events)) => {
                            if let Err(e) = notifier_for_poll_loop.poll(ph) {
                                log::error!("Failed to send poll notification for ph {}: {}", ph, e);
                                if e.kind() == io::ErrorKind::BrokenPipe || e.raw_os_error() == Some(libc::ENODEV) {
                                    warn!("Poll notification channel broken, exiting poll event loop.");
                                    break;
                                }
                            } else {
                                debug!("Sent poll notification for ph {}", ph);
                            }
                        }
                        Err(e) => {
                            info!("Poll event channel disconnected: {}. Exiting poll event loop.", e);
                            break;
                        }
                    }
                }
            }))
        };
        // No explicit poll_event_loop_guard = None for the else case, as the field itself is conditional in BackgroundSession

        Ok(BackgroundSession {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            poll_event_loop_guard,
            #[cfg(feature = "abi-7-11")]
            sender: extra_sender_clone, // This sender is for the Notifier method on BackgroundSession
            _mount: mount,
        })
    }
    /// Unmount the filesystem and join the background thread.
    pub fn join(self) {
        let Self {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            poll_event_loop_guard,
            #[cfg(feature = "abi-7-11")] // sender is conditionally present
            sender: _,
            _mount,
        } = self;
        drop(_mount); // Unmounts the filesystem
        main_loop_guard.join().unwrap().unwrap();
        #[cfg(feature = "abi-7-11")]
        if let Some(guard) = poll_event_loop_guard {
            guard.join().unwrap();
        }
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> Notifier {
        Notifier::new(self.sender.clone())
    }
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
impl fmt::Debug for BackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut builder = f.debug_struct("BackgroundSession");
        builder.field("main_loop_guard", &self.main_loop_guard);
        #[cfg(feature = "abi-7-11")]
        {
            builder.field("poll_event_loop_guard", &self.poll_event_loop_guard);
            builder.field("sender", &self.sender);
        }
        builder.field("_mount", &self._mount);
        builder.finish()
    }
}
