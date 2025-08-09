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

use crate::FsStatus;
use crate::session::{Session, BUFFER_SIZE};
use crate::channel::{aligned_sub_buf, SYNC_SLEEP_INTERVAL, FUSE_HEADER_ALIGNMENT};
use crate::request::RequestHandler;
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::TryRecvError;
use crate::any::{AnyFS};
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS
{
    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub fn run_sync(mut self) -> io::Result<()> {
        // TODO: future multi-threaded feature
        /*
            for ch_idx in 0..self.chs.len() {
            } 
        */
        // ch_idx=0 for the single-threaded case
        let init_fs_status = match &mut self.filesystem {
            AnyFS::Sync(fs) => fs.heartbeat(),
            _ => FsStatus::Default
        };
        if init_fs_status != FsStatus::Default || self.meta.notify.load(Relaxed) {
            self.do_all_events_sync(0)
        } else {
            self.do_requests_sync(0)
        }
    }

    fn do_requests_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!("Starting request loop on channel {ch_idx} with fd {}", &self.chs[ch_idx].raw_fd);
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            let result = self.chs[ch_idx].receive(&mut buffer);
            match result {
                Ok(size) => {
                    let buf = aligned_sub_buf(&mut buffer, FUSE_HEADER_ALIGNMENT);
                    let data = Vec::from(&buf[..size]);
                    buf[..size].fill(0);
                    match RequestHandler::new(self.chs[ch_idx].clone(), data) {
                        // Dispatch request
                        Some(req) => {
                            debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                            match &mut self.filesystem {
                                AnyFS::Sync(fs) => {
                                    req.dispatch_sync(fs, &self.meta);
                                } 
                                _ => {
                                    panic!("tried to execute a SyncFS function on a non-SyncFS filestem")
                                }
                            }
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
    #[cfg(all(feature = "abi-7-11", ))]
    fn do_notifications_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        let sender = self.get_ch(ch_idx);
        info!("Starting notification loop on channel {ch_idx} with fd {}", &sender.raw_fd);
        let notifier = Notifier::new(sender);
        loop {
            if self.meta.destroyed.load(Relaxed) {
                break;
            } else if self.meta.notify.load(Relaxed) {
                if !self.handle_one_notification_sync(&notifier, ch_idx)? {
                    // If no more notifications, 
                    // sleep to make sure that other tasks get attention.
                    std::thread::sleep(SYNC_SLEEP_INTERVAL);
                }
            } else {
                // TODO: await on notify instead of sleeping
                std::thread::sleep(SYNC_SLEEP_INTERVAL);
            }
        }
        Ok(())
    }

    #[cfg(all(feature = "abi-7-11", ))]
    fn handle_one_notification_sync(self: &mut Session<L, S, A>, notifier: &Notifier, ch_idx: usize) -> io::Result<bool> {
        match self.nr.try_recv() {
            Ok(notification) => {
                debug!("Notification {:?} on channel {ch_idx}", &notification.label());
                if let Notification::Stop = notification {
                    // Filesystem says no more notifications.
                    info!("Disabling notifications.");
                    self.meta.notify.store(false, Relaxed);
                }
                if let Err(_e) = futures::executor::block_on(
                    notifier.notify(notification)
                ) {
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
                self.meta.notify.store(false, Relaxed);  
                Ok(false)
            }
        }
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    fn do_heartbeats_sync(self: &mut Session<L, S, A>) -> io::Result<()> {
        info!("Starting heartbeat loop");
        loop {
            std::thread::sleep(SYNC_SLEEP_INTERVAL);
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &mut self.filesystem {
                AnyFS::Sync(fs) => fs.heartbeat(),
                _ => panic!("Attempted to run SyncFS method on non-SyncFS filesystem")
            };
            match fs_status {
                FsStatus::Stopped => {
                    break;
                }
                _ => {
                    // TODO: handle other cases
                }
            }
        }
        Ok(())
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    fn do_all_events_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = aligned_sub_buf(
            &mut buffer,
            FUSE_HEADER_ALIGNMENT,
        );
        let sender = self.get_ch(ch_idx);
        #[cfg(all(feature = "abi-7-11", ))]
        let notifier = Notifier::new(self.get_ch(ch_idx));

        info!("Starting full task loop on channel {ch_idx}");
        loop {
            let mut work_done = false;
            // Check for outgoing Notifications (non-blocking)
            #[cfg(all(feature = "abi-7-11", ))]
            if self.meta.notify.load(Relaxed) {
                // Note: this seems useless
                match self.chs[ch_idx].ready_write() {
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
                            if self.handle_one_notification_sync(&notifier, ch_idx)? {
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
            match self.chs[ch_idx].ready_read() {
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
                        let result = self.chs[ch_idx].receive(buf);
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
                                    match &mut self.filesystem {
                                        AnyFS::Sync(fs) => {
                                            req.dispatch_sync(fs, &self.meta);
                                        } 
                                        _ => {
                                            panic!("tried to execute a SyncFS function on a non-SyncFS filestem")
                                        }
                                    }
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
                std::thread::sleep(SYNC_SLEEP_INTERVAL);
                // Do a heartbeat to let the Filesystem know that some time has passed.
                let fs_status = match &mut self.filesystem {
                    AnyFS::Sync(fs) => fs.heartbeat(),
                    _ => panic!("Attempted to run SyncFS method on non-SyncFS filesystem")
                };
                match fs_status {
                    FsStatus::Stopped => {
                        break;
                    }
                    _ => {
                        // TODO: handle other cases
                    }
                }
            }
        }
        Ok(())
    }
}