use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::io;


use crate::session::{Session, BUFFER_SIZE, SYNC_SLEEP_INTERVAL};
use crate::request::RequestHandler;
use crate::FsStatus;
#[cfg(feature = "abi-7-11")]
use crate::notify::{Notification, Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::TryRecvError;
use crate::any::AnyFS;
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

impl<L, S, A> Session<L, S, A> where 
    L: LegacyFS + Send + Sync + 'static,
    S: SyncFS + Send + Sync +'static,
    A: AsyncFS + 'static
{ 
    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub async fn run_async(self) -> io::Result<()> {
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
                tokio::spawn(Session::do_requests_async(se.clone(), ch_idx))
            );
        }
        for ch_idx in (0..(channel_count)).filter(|i| i%2==1) {
            #[cfg(feature = "abi-7-11")]
            futures.push(
                tokio::spawn(Session::do_notifications_async(se.clone(), ch_idx))
            );
            #[cfg(not(feature = "abi-7-11"))]
            futures.push(
                tokio::spawn(Session::do_requests_async(se.clone(), ch_idx))
            );
        }
        futures.push(
            tokio::spawn(Session::do_heartbeats_async(se.clone()))
        );
        for future in futures {
            if let Err(e) = future.await.expect("Unable to await task?") {
                return Err(e);
            }
        }
        Ok(())
    }


    async fn do_requests_async(se: Arc<Session<L, S, A>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];

        info!("Starting request loop on channel {ch_idx} with fd {}", &se.chs[ch_idx].raw_fd);
        loop {
            // Move buffer into the helper function
            let updated_buffer = Session::handle_one_request_async(&se, ch_idx, buffer).await?;
            if updated_buffer.len() > 0 {
                // Re-use the buffer
                buffer = updated_buffer;
            } else {
                break;
            }
            // TODO: maybe add a heartbeat?
        }
        Ok(())
    }

    async fn handle_one_request_async(se: &Arc<Session<L, S, A>>, ch_idx: usize, buffer: Vec<u8>) -> io::Result<Vec<u8>> {
        // Read the next request from the given channel to kernel driver
        // The kernel driver makes sure that we get exactly one request per read
        #[cfg(not(feature = "tokio"))]
        let mut updated_buffer = buffer;
        #[cfg(not(feature = "tokio"))]
        // Read a FUSE request (blocks until read succeeds)
        let result = se.chs[ch_idx].receive(&mut updated_buffer);
        #[cfg(feature = "tokio")]
        // Read a FUSE request (await until read succeeds)
        let (result, mut updated_buffer) = se.chs[ch_idx].receive_later(buffer).await;
        match result {
            // Kernel sent data
            Ok(data) => {
                // Parse data
                match RequestHandler::new(se.chs[ch_idx].clone(), data) {
                    // Request is valid
                    Some(req) => {
                        debug!("Request {} on channel {ch_idx}.", req.meta.unique);
                        match  &se.filesystem {
                            AnyFS::Async(fs) => {
                                // Dispatch request
                                req.dispatch_async(fs, &se.meta).await;
                                // Return the buffer for re-use
                                Ok(updated_buffer)
                            },
                            _ => panic!("Attempted to call Async run method on non-Async Filesystem")
                        }
                    },
                    // Illegal request
                    // Return an empty buffer to quit the loop
                    None => Ok(Vec::new())
                }
            },
            Err(err) => match err.raw_os_error() {
                // Operation interrupted. Accordingly to FUSE, this is safe to retry
                // Return the buffer for re-use
                Some(ENOENT) => Ok(updated_buffer),
                // Interrupted system call, retry
                // Return the buffer for re-use
                Some(EINTR) => Ok(updated_buffer),
                // Explicitly try again
                // Return the buffer for re-use
                Some(EAGAIN) => Ok(updated_buffer),
                // Filesystem was unmounted,
                // Return an empty buffer to quit the loop
                Some(ENODEV) => Ok(Vec::new()),
                // Unhandled error
                _ => return Err(err),
            },
        }
    }

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    #[cfg(all(feature = "abi-7-11"))]
    async fn do_notifications_async(se: Arc<Session<L, S, A>>, ch_idx: usize) -> io::Result<()> {
        let sender = se.get_ch(ch_idx);
        info!("Starting notification loop on channel {ch_idx} with fd {}", &sender.raw_fd);
        let notifier = Notifier::new(sender);
        loop {
            if se.meta.destroyed.load(Relaxed) {
                break;
            } else if se.meta.notify.load(Relaxed) {
                if !Session::handle_one_notification_async(&se, &notifier, ch_idx).await? {
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

    #[cfg(all(feature = "abi-7-11"))]
    async fn handle_one_notification_async(se: &Session<L, S, A>, notifier: &Notifier, ch_idx: usize) -> io::Result<bool> {
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

    /// Run the session loop in a single thread, same as `run()`, but additionally
    /// processing both FUSE requests and poll events without blocking.
    async fn do_heartbeats_async(se: Arc<Session<L, S, A>>) -> io::Result<()> {
        info!("Starting heartbeat loop");
        loop {
            tokio::time::sleep(SYNC_SLEEP_INTERVAL).await;
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &se.filesystem {
                AnyFS::Async(fs) => fs.heartbeat().await,
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
    async fn do_all_events(se: Arc<Session<L, S, A>>, ch_idx: usize) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        #[cfg(feature = "abi-7-11")]
        let notifier = Notifier::new(se.get_ch(ch_idx));
        info!("Starting full task loop on channel {ch_idx}");
        loop {
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
                            if Session::handle_one_notification_async(&se, &notifier, ch_idx).await? {
                                // skip checking for incoming FUSE requests,
                                // to prioritize checking for additional outgoing messages
                                continue;
                            }
                        }
                    }
                }
            }
            // Check for incoming FUSE requests (non-blocking)
            #[cfg(not(feature = "tokio"))]
            let poll = se.chs[ch_idx].ready_read();
            #[cfg(feature = "tokio")]
            // Tokio variant doesn't need to do this check.
            const poll: io::Result<bool> = Ok(true);
            //#[allow(match)]
            match poll {
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
                        // Move buffer into the helper function
                        let updated_buffer = Session::handle_one_request_async(&se, ch_idx, buffer).await?;
                        if updated_buffer.len() > 0 {
                            // Re-use the buffer
                            buffer = updated_buffer;
                        } else {
                            break;
                        }
                    }
                    // if not ready, do nothing.
                }
            }
            // Do a heartbeat to let the Filesystem know that some time has passed.
            let fs_status = match &se.filesystem {
                AnyFS::Async(fs) => fs.heartbeat().await,
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

}