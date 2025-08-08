//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.

use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use nix::unistd::geteuid;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::io;
#[cfg(feature = "threaded")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "threaded")]
use std::fmt;

use crate::request::RequestHandler;
use crate::{channel, Filesystem, FsStatus};
use crate::MountOption;
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Sender, Receiver};


/// The max size of write requests from the kernel. The absolute minimum is 4k,
/// FUSE recommends at least 128k, max 16M. The FUSE default is 16M on macOS
/// and 128k on other systems.
pub const MAX_WRITE_SIZE: usize = 16 * 1024 * 1024;

/// Size of the buffer for reading a request from the kernel. Since the kernel may send
/// up to `MAX_WRITE_SIZE` bytes in a write request, we use that value plus some extra space.
const BUFFER_SIZE: usize = MAX_WRITE_SIZE + 4096;

/// This value is used to prevent a busy loop in the synchronous run with notification
use crate::channel::SYNC_SLEEP_INTERVAL;

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
    pub(crate) proto_major: u32,
    /// FUSE protocol minor version
    pub(crate) proto_minor: u32,
    /// True if the filesystem is initialized (init operation done)
    pub(crate) initialized: bool,
    /// True if the filesystem was destroyed (destroy operation done)
    pub(crate) destroyed: bool,
    #[cfg(feature = "abi-7-11")]
    /// Whether this session currently has notification support
    pub(crate) notify: bool,
}

/// The session data structure
#[derive(Debug)]
pub struct Session<FS: Filesystem> {
    /// Filesystem operation implementations
    pub(crate) filesystem: Arc<FS>,
    /// Communication channels to the kernel fuse driver
    pub(crate) chs: Vec<Channel>,
    /// Handle to the mount.  Dropping this unmounts.
    mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Whether to restrict access to owner, root + owner, or unrestricted
    /// Used to implement `allow_root` and `auto_unmount`
    meta: Arc<Mutex<SessionMeta>>,
    #[cfg(feature = "abi-7-11")]
    /// Sender for poll events to the filesystem. It will be cloned and passed to Filesystem.
    pub(crate) ns: Sender<Notification>,
    #[cfg(feature = "abi-7-11")]
    /// Receiver for poll events from the filesystem.
    pub(crate) nr: Receiver<Notification>,
}

/*
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
impl<FS: Filesystem> AsFd for Session<FS> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.chs[0].as_borrowed() // doesn't work
    }
}
*/

impl<FS: Filesystem + 'static> Session<FS> {
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
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (ns, nr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        // Pass the sender to the filesystem.
        let notify = futures::executor::block_on(
            filesystem.init_notification_sender(ns.clone())
        );
        let meta = SessionMeta {
            allowed,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            notify,
        };
        let new_session = Session {
            filesystem: Arc::new(filesystem),
            chs: vec![ch],
            mount: Arc::new(Mutex::new(Some((mountpoint.to_owned(), mount)))),
            meta: Arc::new(Mutex::new(meta)),
            #[cfg(feature = "abi-7-11")]
            ns,
            #[cfg(feature = "abi-7-11")]
            nr,
        };
        Ok(new_session)
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: FS, fd: OwnedFd, acl: SessionACL) -> Self {
        // Create the channel for fuse messages
        let ch = Channel::new(fd.into());
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (ns, nr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        // Pass the sender to the filesystem.
        let notify = futures::executor::block_on(
            filesystem.init_notification_sender(ns.clone())
        );
        let meta = SessionMeta {
            allowed: acl,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            notify,
        };
        Session {
            filesystem: Arc::new(filesystem),
            chs: vec![ch],
            mount: Arc::new(Mutex::new(None)),
            meta: Arc::new(Mutex::new(meta)),
            #[cfg(feature = "abi-7-11")]
            ns,
            #[cfg(feature = "abi-7-11")]
            nr,
        }
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
        // OLD notification method
        Notifier::new(self.chs[0].clone())
    }

    /// Returns an object that can be used to send poll event notifications
    #[cfg(feature = "abi-7-11")]
    pub fn get_notification_sender(&self) -> Sender<Notification> {
        // NEW notification method
        self.ns.clone()
    }

    /// returns a copy of the channel associated with a specific channel id.
    fn get_ch(&self, channel_id: usize) -> Channel {
        self.chs[channel_id].clone()
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

    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub async fn run(self) -> io::Result<()> {
        let se = Arc::new(self);
        Session::do_all_events(se.clone(), 0).await
    }

    /// Starts a number of tokio tasks each of which is an independant sequential full event and notification loop.
    pub async fn run_concurrently_sequential_full(self) -> io::Result<()> {
        let channel_count = self.chs.len();
        let se = Arc::new(self);
        let mut futures = Vec::new();
        for channel_id in 0..(channel_count) {
            futures.push(
                tokio::spawn(Session::do_all_events(se.clone(), channel_id))
            )
        }
        for future in futures {
            if let Err(e) = future.await.expect("Unable to await task?") {
                return Err(e);
            }
        }
        Ok(())
    }

    /// Starts a number of tokio tasks each of which is an independant sequential event, heartbeat, or notification loop.
    pub async fn run_concurrently_sequential_parts(self) -> io::Result<()> {
        let channel_count = self.chs.len();
        let se = Arc::new(self);
        let mut futures = Vec::new();
        #[cfg(feature = "abi-7-11")]
        for channel_id in (0..(channel_count)).filter(|i| i%2==0) {
            futures.push(
                tokio::spawn(Session::do_requests(se.clone(), channel_id))
            );
        }
        for channel_id in (0..(channel_count)).filter(|i| i%2==1) {
            #[cfg(feature = "abi-7-11")]
            futures.push(
                tokio::spawn(Session::do_notifications(se.clone(), channel_id))
            );
            #[cfg(not(feature = "abi-7-11"))]
            futures.push(
                tokio::spawn(Session::do_requests(se.clone(), channel_id))
            );
        }
        futures.push(
            tokio::spawn(Session::do_heartbeats(se.clone()))
        );
        for future in futures {
            if let Err(e) = future.await.expect("Unable to await task?") {
                return Err(e);
            }
        }
        Ok(())
    }


    async fn do_requests(se: Arc<Session<FS>>, channel_id: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!("Starting request loop on channel {channel_id} with fd {}", &se.chs[channel_id].raw_fd);
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            let (result, new_buffer) = se.chs[channel_id].receive_later(buffer).await;
            buffer = new_buffer;
            match result {
                Ok(size) => {
                    let buf = channel::aligned_sub_buf(&mut buffer, channel::FUSE_HEADER_ALIGNMENT);
                    let data = Vec::from(&buf[..size]);
                    buf[..size].fill(0);
                    match RequestHandler::new(se.chs[channel_id].clone(), data) {
                        // Dispatch request
                        Some(req) => {
                            debug!("Request {} on channel {channel_id}.", req.meta.unique);
                            req.dispatch(se.filesystem.clone(), se.meta.clone()).await
                        },
                        // Quit loop on illegal request
                        None => break,
                    }
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
            // TODO: maybe add a heartbeat?
        }
        Ok(())
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    #[cfg(feature = "abi-7-11")]
    async fn do_notifications(se: Arc<Session<FS>>, channel_id: usize) -> io::Result<()> {
        let sender = se.get_ch(channel_id);
        info!("Starting notification loop on channel {channel_id} with fd {}", &sender.raw_fd);
        let notifier = Notifier::new(sender);
        loop {
            let notify = match se.meta.lock() {
                Ok(meta) => meta.notify,
                Err(e) => {
                    error!("Notification loop on channel {channel_id}: {e:?}");
                    break;
                }
            };
            if notify {
                if !Session::handle_one_notification(&se, &notifier, channel_id).await? {
                    // If no more notifications, 
                    // sleep to make sure that other tasks get attention.
                    tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
                }
            } else {
                // TODO: await on notify instead of sleeping
                tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            }
        }
        Ok(())
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    async fn do_heartbeats(se: Arc<Session<FS>>) -> io::Result<()> {
        info!("Starting heartbeat loop");
        loop {
            tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            // Do a heartbeat to let the Filesystem know that some time has passed.
            match FS::heartbeat(&se.filesystem).await {
                Ok(status) => {
                    if let FsStatus::Stopped = status {
                        break;
                    }
                    // TODO: handle other cases
                }
                Err(e) => {
                    warn!("Heartbeat error: {e:?}");
                }
            }
        }
        Ok(())
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    async fn do_all_events(se: Arc<Session<FS>>, channel_id: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = channel::aligned_sub_buf(
            &mut buffer,
            channel::FUSE_HEADER_ALIGNMENT,
        );
        let sender = se.get_ch(channel_id);
        #[cfg(feature = "abi-7-11")]
        let notifier = Notifier::new(se.get_ch(channel_id));

        info!("Starting full task loop on channel {channel_id}");
        loop {
            let mut work_done = false;
            // Check for outgoing Notifications (non-blocking)
            #[cfg(feature = "abi-7-11")]
            let notify = match se.meta.lock() {
                Ok(meta) => meta.notify,
                Err(_) => {
                    // TODO: something smarter than this
                    false
                }
            };
            #[cfg(feature = "abi-7-11")]
            if notify {
                match se.chs[channel_id].ready_write() {
                    Err(err) => {
                        if err.raw_os_error() == Some(EINTR) {
                            debug!("FUSE fd connection interrupted, will retry.");
                        } else {
                            warn!("FUSE fd connection: {err}");
                            // Assume very bad. Stop the run. TODO: maybe some handling.
                            return Err(err);
                        }
                    }
                    Ok(ready) => {
                        if ready {    
                            if Session::handle_one_notification(&se, &notifier, channel_id).await? {
                                work_done = true;
                            }
                        }
                    }
                }
            }
            if work_done {
                // skip checking for incoming FUSE requests,
                // to prioritize checking for additional outgoing messages
                continue;
            }
            // Check for incoming FUSE requests (non-blocking)
            match se.chs[channel_id].ready_read() {
                Err(err) => {
                    if err.raw_os_error() == Some(EINTR) {
                        debug!("FUSE fd connection interrupted, will retry.");
                    } else {
                        warn!("FUSE fd connection: {err}");
                        // Assume very bad. Stop the run. TODO: maybe some handling.
                        return Err(err);
                    }
                }
                Ok(ready) => {
                    if ready {
                        // Read a FUSE request (blocks until read succeeds)
                        let result = se.chs[channel_id].receive(buf);
                        match result {
                            Ok(size) => {
                                if size == 0 {
                                    // Read of 0 bytes on FUSE FD typically means it was closed (unmounted)
                                    info!("FUSE channel read 0 bytes, session ending.");
                                    break;
                                }
                                let data = Vec::from(&buf[..size]);
                                if let Some(req) = RequestHandler::new(sender.clone(), data) {
                                    let fs = se.filesystem.clone();
                                    let meta = se.meta.clone();
                                    debug!("Request {} on channel {channel_id}.", req.meta.unique);
                                    req.dispatch(fs, meta).await;
                                } else {
                                    // Illegal request, quit loop
                                    warn!("Failed to parse FUSE request, session ending.");
                                    break;
                                }
                                work_done = true;
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
                                    error!("Error receiving FUSE request: {err}");
                                    return Err(err); // Unhandled error
                                }
                            },
                        }
                    }
                    // if not ready, do nothing.
                }
            }
            if !work_done {
                // No actions taken this loop iteration.
                // Sleep briefly to yield CPU.
                tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
                // Do a heartbeat to let the Filesystem know that some time has passed.
                match FS::heartbeat(&se.filesystem).await {
                    Ok(status) => {
                        if let FsStatus::Stopped = status {
                            break;
                        }
                        // TODO: handle other cases
                    }
                    Err(e) => {
                        warn!("Heartbeat error: {e:?}");
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "abi-7-11")]
    async fn handle_one_notification(se: &Arc<Session<FS>>, notifier: &Notifier, channel_id: usize) -> io::Result<bool> {
        let notify = match se.meta.lock() {
            Ok(meta) => meta.notify,
            Err(_) => {
                // TODO: something smarter than this
                return Ok(false);
            }
        };
        if notify {
            match se.nr.try_recv() {
                Ok(notification) => {
                    debug!("Notification {:?} on channel {channel_id}", &notification.label());
                    if let Notification::Stop = notification {
                        // Filesystem says no more notifications.
                        info!("Disabling notifications.");
                        if let Ok(mut meta) = se.meta.lock() {
                            meta.notify = false;
                        }
                    }
                    if let Err(_e) = notifier.notify(notification).await {
                        error!("Failed to send notification.");
                        // TODO. Decide if error is fatal. ENODEV might mean unmounted.
                    }
                    Ok(true)
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    // No poll events pending, proceed to check FUSE FD
                    Ok(false)
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    // Filesystem's Notification Sender disconnected.
                    // This is not necessarily a fatal error for the session itself,
                    // as FUSE requests can still be processed.
                    warn!("Notification channel disconnected.");
                    if let Ok(mut meta) = se.meta.lock() {
                        meta.notify = false;
                    }                    
                    Ok(false)
                }
            }
        } else {
            Ok(false)
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
impl<FS: 'static + Filesystem + Send> Session<FS> {
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}

impl<FS: Filesystem> Drop for Session<FS> {
    fn drop(&mut self) {
        let mut meta = self.meta.lock().unwrap();
        if !meta.destroyed {
            futures::executor::block_on(
                self.filesystem.destroy()
            );
            meta.destroyed = true;
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
    pub fn new<FS: Filesystem + Send + 'static>(se: Session<FS>) -> io::Result<BackgroundSession> {
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
