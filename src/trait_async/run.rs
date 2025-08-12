use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::io;
use std::sync::Arc;
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
    L: LegacyFS + Send + Sync + 'static,
    S: SyncFS + Send + Sync + 'static,
    A: AsyncFS + 'static, // implied Send + Sync
{
    /// Run the session in a single thread with a single channel. TODO: multithreaded.
    /// Sequentially handles requests, notifications, and/or heartbeats (if enabled).
    pub async fn run_async(self) -> io::Result<()> {
        let init_fs_status = match &self.filesystem {
            AnyFS::Async(fs) => fs.heartbeat().await,
            _ => FsStatus::Default,
        };
        let se = Arc::new(self);
        // ch_idx=0 for the single-threaded case
        #[cfg(not(feature = "abi-7-11"))]
        let notify = false;
        #[cfg(feature = "abi-7-11")]
        let notify = se.meta.notify.load(Relaxed);
        if init_fs_status != FsStatus::Default || notify {
            Session::do_all_events_async(se.clone(), 0).await
        } else {
            Session::do_requests_async(se.clone(), 0).await
        }
    }

    /// Starts one tokio task per channel, each of which is a full, sequential session loop.
    /// NOTE: Single-threaded concurrency is experimental.
    /// WARNING. Deadlocks can occur when using multiple concurrent loops, and the conditions are not well-documented.
    #[cfg(feature = "tokio")]
    pub async fn run_concurrent_full(self) -> io::Result<()> {
        let channel_count = self.chs.len();
        if channel_count > 1 {
            warn!("multiple full task loops! possible deadlocks");
        }
        let se = Arc::new(self);
        let mut futures = Vec::new();
        for ch_idx in 0..(channel_count) {
            futures.push(tokio::spawn(Session::do_all_events_async(
                se.clone(),
                ch_idx,
            )))
        }
        for future in futures {
            if let Err(e) = future.await.expect("Unable to await task?") {
                return Err(e);
            }
        }
        Ok(())
    }

    /// Starts one tokio task on per channel, each of which either a request or notification loop.
    /// Additionally, starts one tokio task for heartbeats.
    /// NOTE: Single-threaded concurrency is experimental.
    /// WARNING. Deadlocks can occur when using multiple concurrent loops, and the conditions are not well-documented.
    #[cfg(feature = "tokio")]
    pub async fn run_concurrently_parts(self) -> io::Result<()> {
        let channel_count = self.chs.len();
        if channel_count > 0 {
            error!("too many tasks loops! possible deadlocks");
        }
        let se = Arc::new(self);
        let mut futures = Vec::new();
        // Alternate between request channel and notification channel.
        // TODO: more fine-grained control.
        for ch_idx in (0..(channel_count)).filter(|i| i % 2 == 0) {
            futures.push(tokio::spawn(Session::do_requests_async(se.clone(), ch_idx)));
        }
        #[cfg(feature = "abi-7-11")]
        for ch_idx in (0..(channel_count)).filter(|i| i % 2 == 1) {
            futures.push(tokio::spawn(Session::do_notifications_async(
                se.clone(),
                ch_idx,
            )));
        }
        futures.push(tokio::spawn(Session::do_heartbeats_async(se.clone())));
        for future in futures {
            if let Err(e) = future.await.expect("Unable to await task?") {
                return Err(e);
            }
        }
        Ok(())
    }

    /// Process requests in a single task.
    pub async fn do_requests_async(se: Arc<Session<L, S, A>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!(
            "Starting request loop on channel {ch_idx} with fd {}",
            &se.chs[ch_idx].raw_fd
        );
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            #[cfg(not(feature = "tokio"))]
            let mut updated_buffer = buffer;
            #[cfg(not(feature = "tokio"))]
            // Read a FUSE request (blocks until read succeeds)
            let result = se.chs[ch_idx].receive(&mut updated_buffer);
            #[cfg(feature = "tokio")]
            // Read a FUSE request (await until read succeeds)
            // Move buffer into the helper function, receive the buffer back if it succeeds.
            let (result, updated_buffer) = se.chs[ch_idx].receive_async(buffer).await;
            if updated_buffer.len() > 0 {
                // Re-use the buffer for the next loop iteration
                buffer = updated_buffer;
            } else {
                break;
            }
            match result {
                Ok(data) => {
                    if !Session::handle_one_request_async(&se, ch_idx, data).await {
                        // false means invalid data; stop the loop
                        break;
                    }
                }
                Err(err) => match err.raw_os_error() {
                    // Operation interrupted. Accordingly to FUSE, this is safe to retry
                    Some(ENOENT) => continue,
                    // Interrupted system call, retry
                    // Return the buffer for re-use
                    Some(EINTR) => continue,
                    // Explicitly try again
                    Some(EAGAIN) => continue,
                    // Filesystem was unmounted,
                    // Return an empty buffer to quit the loop
                    Some(ENODEV) => break,
                    // Unhandled error
                    _ => return Err(err),
                },
            }
        }
        Ok(())
    }

    async fn handle_one_request_async(
        se: &Arc<Session<L, S, A>>,
        ch_idx: usize,
        data: Vec<u8>,
    ) -> bool {
        // Parse data
        match RequestHandler::new(se.chs[ch_idx].clone(), data) {
            // Request is valid
            Some(req) => {
                debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                match &se.filesystem {
                    AnyFS::Async(fs) => {
                        // Dispatch request
                        req.dispatch_async(fs, &se.meta).await;
                        // Return signal to continue
                        true
                    }
                    _ => panic!("Attempted to call Async run method on non-Async Filesystem"),
                }
            }
            // Illegal request
            // Return the signal to break
            None => false,
        }
    }

    /// Process notifications in a single task.
    /// This variant executes sleep() to prevent busy loops.
    #[cfg(all(feature = "abi-7-11"))]
    #[allow(unused)] // this function is reserved for future multithreaded implementations
    pub async fn do_notifications_async(
        se: Arc<Session<L, S, A>>,
        ch_idx: usize,
    ) -> io::Result<()> {
        let sender = se.get_ch(ch_idx);
        info!(
            "Starting notification loop on channel {ch_idx} with fd {}",
            &sender.raw_fd
        );
        let notifier = Notifier::new(sender);
        loop {
            if se.meta.destroyed.load(Relaxed) {
                break;
            } else if se.meta.notify.load(Relaxed) {
                // Fetch one notification; recv() blocks until one is available
                // TODO: switch to a receiver variant that awaits
                match se.nr.recv() {
                    Ok(notification) => {
                        // Process the notification; blocks until sent
                        Session::handle_one_notification_async(&se, notification, &notifier, ch_idx)
                            .await?
                    }
                    Err(RecvError) => {
                        // Filesystem's Notification Sender disconnected.
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed.
                        warn!("Notification channel disconnected.");
                        se.meta.notify.store(false, Relaxed);
                    }
                }
            } else {
                // TODO: maybe: `break;` instead of sleeping this task?
                #[cfg(not(feature = "tokio"))]
                std::thread::sleep(SYNC_SLEEP_INTERVAL);
                #[cfg(feature = "tokio")]
                tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            }
        }
        Ok(())
    }

    #[cfg(all(feature = "abi-7-11"))]
    async fn handle_one_notification_async(
        se: &Session<L, S, A>,
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
            se.meta.notify.store(false, Relaxed);
            Ok(())
        } else {
            #[cfg(not(feature = "tokio"))]
            let res = notifier.notify(notification);
            #[cfg(feature = "tokio")]
            let notifier_copy = notifier.clone();
            #[cfg(feature = "tokio")]
            let res =
                tokio::task::spawn_blocking(async move || notifier_copy.notify(notification)).await;
            if let Err(e) = res {
                error!("Failed to send notification.");
                // TODO. Decide if error is fatal. ENODEV might mean unmounted.
                Err(e.into())
            } else {
                Ok(())
            }
        }
    }

    /// Process heartbeats in a single task.
    /// This variant executes sleep() to prevent busy loops.
    #[allow(unused)] // this function is reserved for future multithreaded implementations
    pub async fn do_heartbeats_async(se: Arc<Session<L, S, A>>) -> io::Result<()> {
        info!("Starting heartbeat loop");
        loop {
            #[cfg(not(feature = "tokio"))]
            std::thread::sleep(SYNC_SLEEP_INTERVAL);
            #[cfg(feature = "tokio")]
            tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &se.filesystem {
                AnyFS::Async(fs) => fs.heartbeat().await,
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

    /// Run the session loop in a single task.
    /// Alternates between processing requests, notifications, and heartbeats, without blocking.
    /// This variant executes sleep() to prevent busy loops.
    pub async fn do_all_events_async(se: Arc<Session<L, S, A>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        #[cfg(feature = "abi-7-11")]
        let notifier = Notifier::new(se.get_ch(ch_idx));
        info!("Starting full task loop on channel {ch_idx}");
        loop {
            // Check for outgoing Notifications (non-blocking)
            #[cfg(feature = "abi-7-11")]
            if se.meta.notify.load(Relaxed) {
                match se.nr.try_recv() {
                    Ok(notification) => {
                        Session::handle_one_notification_async(
                            &se,
                            notification,
                            &notifier,
                            ch_idx,
                        )
                        .await?;
                        // skip checking for incoming FUSE requests,
                        // to prioritize checking for additional outgoing messages
                        continue;
                    }
                    Err(TryRecvError::Empty) => {
                        // No poll events pending, proceed to check FUSE FD
                    }
                    Err(TryRecvError::Disconnected) => {
                        warn!("Notification channel disconnected.");
                        se.meta.notify.store(false, Relaxed);
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed.
                    }
                }
            }
            // Check for incoming FUSE requests (non-blocking)
            #[cfg(not(feature = "tokio"))]
            let mut updated_buffer = buffer;
            #[cfg(not(feature = "tokio"))]
            let result = se.chs[ch_idx].try_receive(&mut updated_buffer);
            #[cfg(feature = "tokio")]
            let updated_buffer = buffer;
            #[cfg(feature = "tokio")]
            let (result, updated_buffer) = se.chs[ch_idx].try_receive_async(updated_buffer).await;
            if updated_buffer.len() > 0 {
                // Re-use the buffer for the next loop iteration
                buffer = updated_buffer;
            } else {
                error!("Buffer corrupted on channel {ch_idx}");
                break;
            }
            // Decide what to do about the FUSE request
            match result {
                Err(err) => {
                    if err.raw_os_error() == Some(EINTR) {
                        debug!("FUSE fd connection interrupted, will retry.");
                    } else {
                        warn!("FUSE fd connection: {err}");
                        // Assume very bad. Stop the run. TODO: consider additional handling.
                        return Err(err);
                    }
                }
                Ok(Some(data)) => {
                    // Parse data
                    if Session::handle_one_request_async(&se, ch_idx, data).await {
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
            // Sleep to prevent a busy loop, blocks the thread.
            #[cfg(not(feature = "tokio"))]
            std::thread::sleep(SYNC_SLEEP_INTERVAL);
            #[cfg(feature = "tokio")]
            tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &se.filesystem {
                AnyFS::Async(fs) => fs.heartbeat().await,
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
