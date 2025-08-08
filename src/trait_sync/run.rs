use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use nix::unistd::geteuid;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::io;
#[cfg(feature = "threaded")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "threaded")]
use std::fmt;

use crate::session::{Session, BackgroundSession, FsStatus, BUFFER_SIZE};
// This value is used to prevent a busy loop in the synchronous run with notification
use crate::channel::SYNC_SLEEP_INTERVAL;
use crate::request::RequestHandler;
use crate::{channel};
use crate::MountOption;
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::TryRecvError;
use crate::any::{_Nl, _Na};
use super::Filesystem as SyncFilesystem;

impl<FS: SyncFilesystem> Session<_Nl, FS, _Na> {    
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
        for ch_idx in 0..(channel_count) {
            futures.push(
                tokio::spawn(Session::do_all_events(se.clone(), ch_idx))
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
        for ch_idx in (0..(channel_count)).filter(|i| i%2==0) {
            futures.push(
                tokio::spawn(Session::do_requests(se.clone(), ch_idx))
            );
        }
        for ch_idx in (0..(channel_count)).filter(|i| i%2==1) {
            #[cfg(feature = "abi-7-11")]
            futures.push(
                tokio::spawn(Session::do_notifications(se.clone(), ch_idx))
            );
            #[cfg(not(feature = "abi-7-11"))]
            futures.push(
                tokio::spawn(Session::do_requests(se.clone(), ch_idx))
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


    async fn do_requests(se: Arc<Session<FS>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!("Starting request loop on channel {ch_idx} with fd {}", &se.chs[ch_idx].raw_fd);
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            let (result, new_buffer) = se.chs[ch_idx].receive_later(buffer).await;
            buffer = new_buffer;
            match result {
                Ok(size) => {
                    let buf = channel::aligned_sub_buf(&mut buffer, channel::FUSE_HEADER_ALIGNMENT);
                    let data = Vec::from(&buf[..size]);
                    buf[..size].fill(0);
                    match RequestHandler::new(se.chs[ch_idx].clone(), data) {
                        // Dispatch request
                        Some(req) => {
                            debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                            req.dispatch_async(&se.filesystem, &se.meta).await
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
    async fn do_notifications(se: Arc<Session<FS>>, ch_idx: usize) -> io::Result<()> {
        let sender = se.get_ch(ch_idx);
        info!("Starting notification loop on channel {ch_idx} with fd {}", &sender.raw_fd);
        let notifier = Notifier::new(sender);
        loop {
            if se.meta.destroyed.load(Relaxed) {
                break;
            } else if se.meta.notify.load(Relaxed) {
                if !Session::handle_one_notification(&se, &notifier, ch_idx).await? {
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
    async fn do_all_events(se: Arc<Session<FS>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = channel::aligned_sub_buf(
            &mut buffer,
            channel::FUSE_HEADER_ALIGNMENT,
        );
        let sender = se.get_ch(ch_idx);
        #[cfg(feature = "abi-7-11")]
        let notifier = Notifier::new(se.get_ch(ch_idx));

        info!("Starting full task loop on channel {ch_idx}");
        loop {
            let mut work_done = false;
            // Check for outgoing Notifications (non-blocking)
            #[cfg(feature = "abi-7-11")]
            if se.meta.notify.load(Relaxed) {
                // Note: this seems useless
                match se.chs[ch_idx].ready_write() {
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
                            if Session::handle_one_notification(&se, &notifier, ch_idx).await? {
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
            match se.chs[ch_idx].ready_read() {
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
                        let result = se.chs[ch_idx].receive(buf);
                        match result {
                            Ok(size) => {
                                if size == 0 {
                                    // Read of 0 bytes on FUSE FD typically means it was closed (unmounted)
                                    info!("FUSE channel read 0 bytes, session ending.");
                                    break;
                                }
                                let data = Vec::from(&buf[..size]);
                                if let Some(req) = RequestHandler::new(sender.clone(), data) {
                                    debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                                    req.dispatch_async(&se.filesystem, &se.meta).await;
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
    async fn handle_one_notification(se: &Session<FS>, notifier: &Notifier, ch_idx: usize) -> io::Result<bool> {
        match se.nr.try_recv() {
            Ok(notification) => {
                debug!("Notification {:?} on channel {ch_idx}", &notification.label());
                if let Notification::Stop = notification {
                    // Filesystem says no more notifications.
                    info!("Disabling notifications.");
                    se.meta.notify.store(false, Relaxed);
                }
                if let Err(_e) = notifier.notify(notification).await {
                    error!("Failed to send notification.");
                    // TODO. Decide if error is fatal. ENODEV might mean unmounted.
                }
                Ok(true)
            }
            Err(TryRecvError::Empty) => {
                // No poll events pending, proceed to check FUSE FD
                Ok(false)
            }
            Err(TryRecvError::Disconnected) => {
                // Filesystem's Notification Sender disconnected.
                // This is not necessarily a fatal error for the session itself,
                // as FUSE requests can still be processed.
                warn!("Notification channel disconnected.");
                se.meta.notify.store(false, Relaxed);  
                Ok(false)
            }
        }
    }
}