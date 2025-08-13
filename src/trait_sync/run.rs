use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::io;
#[cfg(feature = "abi-7-11")]
use std::sync::atomic::Ordering::Relaxed;

use crate::FsStatus;
use crate::any::AnyFS;
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
use crate::request::RequestHandler;
use crate::session::{BUFFER_SIZE, SYNC_SLEEP_INTERVAL, Session};
use crate::trait_async::Filesystem as AsyncFS;
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{RecvError, TryRecvError};

impl<L, S, A> Session<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    /// Run the session in a single thread. TODO: multithreaded.
    /// Handles requests, and also sends notifications and/or heartbeats (if enabled).
    pub fn run_sync(mut self) -> io::Result<()> {
        let init_fs_status = match &mut self.filesystem {
            AnyFS::Sync(fs) => fs.heartbeat(),
            _ => FsStatus::Default,
        };
        /*
        // a sketch of a possible multithreaded implementation
        #cfg[(feature = "threaded")]
        {
            // TODO: empty join handles
            for ch_idx in 0..self.chs.len() {
                // TODO: join_handle = std::?::spawn(
                    if init_fs_status != FsStatus::Default || self.meta.notify.load(Relaxed) {
                        self.do_all_events_sync(ch_idx)
                    } else {
                        self.do_requests_sync(ch_idx)
                    }
                )
                // TODO: add the join handle
            }
            // TODO: gather the join handles
        }
        #cfg[not((feature = "threaded"))]
        */
        #[cfg(not(feature = "abi-7-11"))]
        let notify = false;
        #[cfg(feature = "abi-7-11")]
        let notify = self.meta.notify.load(Relaxed);
        // ch_idx=0 for the single-threaded case
        if init_fs_status != FsStatus::Default || notify {
            self.do_all_events_sync(0)
        } else {
            self.do_requests_sync(0)
        }
    }

    /// Process requests, blocking a single thread.
    pub fn do_requests_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!(
            "Starting request loop on channel {ch_idx} with fd {}",
            &self.chs[ch_idx].raw_fd
        );
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            // Read a FUSE request (blocks until read succeeds)
            match self.chs[ch_idx].receive(&mut buffer) {
                Ok(data) => {
                    // Kernel sent data
                    if !self.handle_one_request_sync(ch_idx, data) {
                        // false means invalid data; stop the loop
                        break;
                    }
                }
                Err(err) => match err.raw_os_error() {
                    // Operation interrupted. Accordingly to FUSE, this is safe to retry
                    Some(ENOENT) => continue,
                    // Interrupted system call, retry
                    Some(EINTR) => continue,
                    // Explicitly try again
                    Some(EAGAIN) => continue,
                    // Filesystem was unmounted,
                    // Stop the loop.
                    Some(ENODEV) => break,
                    // Unhandled error
                    _ => return Err(err),
                },
            }
        }
        Ok(())
    }

    fn handle_one_request_sync(self: &mut Session<L, S, A>, ch_idx: usize, data: Vec<u8>) -> bool {
        // Parse data
        match RequestHandler::new(self.chs[ch_idx].clone(), data) {
            // Request is valid
            Some(req) => {
                debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                match &mut self.filesystem {
                    AnyFS::Sync(fs) => {
                        // Dispatch request
                        req.dispatch_sync(fs, &self.meta);
                        // Return signal to continue
                        true
                    }
                    _ => panic!("Attempted to call Sync run method on non-Sync Filesystem"),
                }
            }
            // Illegal request
            // Return the signal to break
            None => false,
        }
    }

    /// Process notifications, blocking a single thread.
    #[cfg(feature = "abi-7-11")]
    #[allow(unused)] // this function is reserved for future multithreaded implementations
    pub fn do_notifications_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        let sender = self.get_ch(ch_idx);
        info!(
            "Starting notification loop on channel {ch_idx} with fd {}",
            &sender.raw_fd
        );
        let notifier = Notifier::new(sender);
        loop {
            if self.meta.destroyed.load(Relaxed) {
                break;
            } else if self.meta.notify.load(Relaxed) {
                // Fetch one notification; recv() blocks until one is available
                match self.nr.recv() {
                    Ok(notification) => {
                        // Process the notification; blocks until sent
                        self.handle_one_notification_sync(notification, &notifier, ch_idx)?
                    }
                    Err(RecvError) => {
                        // Filesystem's Notification Sender disconnected.
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed.
                        warn!("Notification channel disconnected.");
                        self.meta.notify.store(false, Relaxed);
                    }
                }
            } else {
                // Maybe break; instead of sleeping this thread?
                std::thread::sleep(SYNC_SLEEP_INTERVAL);
            }
        }
        Ok(())
    }

    #[cfg(feature = "abi-7-11")]
    fn handle_one_notification_sync(
        self: &mut Session<L, S, A>,
        notification: Notification,
        notifier: &Notifier,
        ch_idx: usize,
    ) -> io::Result<()> {
        debug!(
            "Notification {:?} on channel {ch_idx}",
            &notification.label()
        );
        if let Notification::Stop = notification {
            // Filesystem says no more notifications.
            info!("Disabling notifications.");
            self.meta.notify.store(false, Relaxed);
            Ok(())
        } else if let Err(e) = notifier.notify(notification) {
            error!("Failed to send notification.");
            // TODO. Decide if error is fatal. ENODEV might mean unmounted.
            Err(e)
        } else {
            Ok(())
        }
    }

    /// Process heartbeats, blocking a single thread.
    /// This variant executes sleep() to prevent busy loops.
    #[allow(unused)] // this function is reserved for future multithreaded implementations
    pub fn do_heartbeats_sync(self: &mut Session<L, S, A>) -> io::Result<()> {
        info!("Starting heartbeat loop");
        loop {
            std::thread::sleep(SYNC_SLEEP_INTERVAL);
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &mut self.filesystem {
                AnyFS::Sync(fs) => fs.heartbeat(),
                _ => panic!("Attempted to run SyncFS method on non-SyncFS filesystem"),
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

    /// Run the session loop in a single thread.
    /// Alternates between processing requests, notifications, and heartbeats, without blocking.
    /// This variant executes sleep() to prevent busy loops.
    pub fn do_all_events_sync(self: &mut Session<L, S, A>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        #[cfg(feature = "abi-7-11")]
        let notifier = Notifier::new(self.get_ch(ch_idx));

        info!("Starting full task loop on channel {ch_idx}");
        loop {
            #[cfg(feature = "abi-7-11")]
            if self.meta.notify.load(Relaxed) {
                // Check for outgoing notifications
                // try_recv() returns immediately
                match self.nr.try_recv() {
                    Ok(notification) => {
                        debug!(
                            "Notification {:?} on channel {ch_idx}",
                            &notification.label()
                        );
                        self.handle_one_notification_sync(notification, &notifier, ch_idx)?;
                        // skip checking for incoming FUSE requests,
                        // to prioritize checking for additional outgoing messages
                        continue;
                    }
                    Err(TryRecvError::Empty) => {
                        // No poll events pending, proceed to requests
                    }
                    Err(TryRecvError::Disconnected) => {
                        warn!("Notification channel disconnected.");
                        self.meta.notify.store(false, Relaxed);
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed. Proceed.
                    }
                }
            }
            // Check for incoming FUSE requests; (non-blocking)
            match self.chs[ch_idx].try_receive(&mut buffer) {
                Err(err) => {
                    if err.raw_os_error() == Some(EINTR) {
                        debug!("FUSE fd connection interrupted, will retry.");
                        continue;
                    //TODO: handle additional cases
                    } else {
                        warn!("FUSE fd: {err}");
                        // Unhandled error. Stop the run.
                        return Err(err);
                    }
                }
                Ok(Some(data)) => {
                    if self.handle_one_request_sync(ch_idx, data) {
                        // Skip the heartbeat to prioritize processing other pending requests
                        continue;
                    } else {
                        // Invalid request, assuming the state cannot be recovered
                        break;
                    }
                }
                Ok(None) => {
                    // request not ready, proceed to heartbeat.
                }
            }
            // No events were found during this loop iteration.
            // Sleep to prevent a busy loop
            std::thread::sleep(SYNC_SLEEP_INTERVAL);
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &mut self.filesystem {
                AnyFS::Sync(fs) => fs.heartbeat(),
                _ => panic!("Attempted to run SyncFS method on non-SyncFS filesystem"),
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
}
