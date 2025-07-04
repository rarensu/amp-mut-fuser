use crossbeam_channel::Sender;
use std::collections::{HashMap, HashSet};
// Arc and Mutex are no longer used directly within this module's PollData logic
// as PollData is typically owned directly or its sharing is managed by its owner.

/// `PollData` holds the state required for managing asynchronous poll notifications.
/// It is typically owned by a `Filesystem` implementation. The `Sender` end of its
/// MPMC channel (`ready_events_sender`) is provided by the `Session` to the
/// `Filesystem` (e.g., via a method like `init_poll_sender`).
#[derive(Debug)]
pub struct PollData {
    /// Sender part of the MPMC channel for (poll_handle, events_bitmask).
    /// This is used by the filesystem logic to send readiness events.
    /// Typically set via `PollData::new` or `PollData::set_sender`.
    pub ready_events_sender: Option<Sender<(u64, u32)>>,
    /// Stores registered poll handles.
    /// Maps a kernel poll handle (`u64`) to a tuple of (inode, requested_events).
    /// This allows us to know which inode and which events a poll handle is interested in.
    pub registered_poll_handles: HashMap<u64, (u64, u32)>,
    /// Stores active poll handles for a given inode.
    /// Maps an inode (`u64`) to a set of kernel poll handles (`u64`).
    /// This is useful to quickly find all poll handles interested in a particular inode
    /// when that inode's state changes.
    pub inode_poll_handles: HashMap<u64, HashSet<u64>>,
    /// Tracks inodes that are currently ready for I/O (e.g., POLLIN).
    /// This set is updated by filesystem operations.
    pub ready_inodes: HashSet<u64>,
}

impl PollData {
    /// Creates a new `PollData` instance, optionally with an initial sender.
    pub fn new(sender: Option<Sender<(u64, u32)>>) -> Self {
        PollData {
            ready_events_sender: sender,
            registered_poll_handles: HashMap::new(),
            inode_poll_handles: HashMap::new(),
            ready_inodes: HashSet::new(),
        }
    }

    /// Sets or updates the sender for ready events.
    /// This is typically called by the `Filesystem` implementation when the `Session` provides the sender.
    pub fn set_sender(&mut self, new_sender: Sender<(u64, u32)>) {
        self.ready_events_sender = Some(new_sender);
    }

    /// Registers a new poll request.
    ///
    /// Stores the kernel poll handle (`ph`) associated with an inode and the events
    /// it's interested in. If the inode is already ready for the requested events,
    /// an immediate notification is sent.
    ///
    /// # Arguments
    ///
    /// * `ph`: The kernel poll handle.
    /// * `ino`: The inode number being polled.
    /// * `events_requested`: The event bitmask the poll handle is interested in.
    ///
    /// # Returns
    ///
    /// * `Option<u32>`: An initial event mask if the file is already ready, otherwise `None`.
    pub fn register_poll_handle(
        &mut self,
        ph: u64,
        ino: u64,
        events_requested: u32,
    ) -> Option<u32> {
        self.registered_poll_handles
            .insert(ph, (ino, events_requested));
        self.inode_poll_handles.entry(ino).or_default().insert(ph);

        // Check if the file is already ready and send an initial notification if so.
        // We assume POLLIN for now as the primary readiness event.
        // More sophisticated event matching could be added here.
        if self.ready_inodes.contains(&ino) {
            if let Some(sender) = &self.ready_events_sender {
                // TODO: Determine actual_events based on file state and events_requested.
                // For now, just signaling with POLLIN if requested.
                // TODO: Determine actual_events based on file state and events_requested more accurately.
                #[cfg(feature = "abi-7-21")]
                let actual_events = events_requested & libc::POLLIN as u32; // Example: only care about POLLIN
                #[cfg(not(feature = "abi-7-21"))]
                let actual_events = libc::POLLIN as u32; // Before abi-7-21, requested events is meaningless
                if actual_events != 0 {
                    log::debug!(
                        "PollData::register_poll_handle() sending initial event: ph={}, actual_events={:#x}",
                        ph, actual_events
                    );
                    if sender.send((ph, actual_events)).is_err() {
                        log::warn!("PollData: Failed to send initial poll readiness event for ph {}. Channel might be disconnected.", ph);
                    }
                    return Some(actual_events);
                }
            }
        }
        None
    }

    /// Unregisters a poll handle.
    ///
    /// This is typically called when the poll request is cancelled or the associated
    /// file descriptor is closed.
    ///
    /// # Arguments
    ///
    /// * `ph`: The kernel poll handle to unregister.
    pub fn unregister_poll_handle(&mut self, ph: u64) {
        if let Some((ino, _)) = self.registered_poll_handles.remove(&ph) {
            if let Some(handles) = self.inode_poll_handles.get_mut(&ino) {
                handles.remove(&ph);
                if handles.is_empty() {
                    self.inode_poll_handles.remove(&ino);
                }
            }
        }
    }

    // Removed set_ready_events_sender as set_sender serves this purpose.

    /// Marks an inode as ready for I/O and notifies registered poll handles.
    ///
    /// # Arguments
    ///
    /// * `ino`: The inode number that has become ready.
    /// * `ready_events`: The bitmask of events that are now active for the inode (e.g., `libc::POLLIN`).
    pub fn mark_inode_ready(&mut self, ino: u64, ready_events_mask: u32) {
        log::info!(
            "PollData::mark_inode_ready() called: ino={}, ready_events_mask={:#x}",
            ino, ready_events_mask
        );
        self.ready_inodes.insert(ino);
        if let Some(sender) = &self.ready_events_sender {
            if let Some(poll_handles) = self.inode_poll_handles.get(&ino) {
                for &ph in poll_handles {
                    if let Some((_ino_of_ph, _requested_events)) = self.registered_poll_handles.get(&ph) {
                        #[cfg(feature = "abi-7-21")]
                        let actual_events = _requested_events & libc::POLLIN as u32; // Example: only care about POLLIN
                        #[cfg(not(feature = "abi-7-21"))]
                        let actual_events = libc::POLLIN as u32; // Before abi-7-21, requested events is meaningless
                        if actual_events != 0 {
                            log::debug!(
                                "PollData::mark_inode_ready() sending event: ino={}, ph={}, actual_events={:#x}",
                                ino, ph, actual_events
                            );
                            if sender.send((ph, actual_events)).is_err() {
                                log::warn!("PollData: Failed to send poll readiness event for ino {}, ph {}. Channel might be disconnected.", ino, ph);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Marks an inode as no longer ready for I/O.
    ///
    /// # Arguments
    ///
    /// * `ino`: The inode number that is no longer ready.
    pub fn mark_inode_not_ready(&mut self, ino: u64) {
        self.ready_inodes.remove(&ino);
        // Note: FUSE usually doesn't have explicit "not ready anymore" notifications for poll,
        // other than timeout. Applications will re-poll if needed.
        // However, clearing the state is important for subsequent poll registrations.
    }
}

// Example of how it might be integrated into a Filesystem struct
//
// pub struct MyFs {
//     poll_data: Arc<Mutex<PollData>>,
//     // other fs data
// }
//
// impl MyFs {
//     pub fn new(poll_data_sender: Option<Sender<(u64, u32)>>) -> Self {
//         Self {
//             poll_data: Arc::new(Mutex::new(PollData::new(poll_data_sender))),
//             // ...
//         }
//     }
// }
//
// // In the Filesystem::poll implementation:
// // let mut poll_data_guard = self.poll_data.lock().unwrap();
// // poll_data_guard.register_poll_handle(ph, ino, events);
//
// // In an operation that makes a file ready (e.g., write):
// // let mut poll_data_guard = self.poll_data.lock().unwrap();
// // poll_data_guard.mark_inode_ready(ino, libc::POLLIN as u32);

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_poll_data_new() {
        let (_tx, rx) = unbounded();
        let poll_data = PollData::new(Some(_tx));
        assert!(poll_data.ready_events_sender.is_some());
        assert!(poll_data.registered_poll_handles.is_empty());
        assert!(poll_data.inode_poll_handles.is_empty());
        assert!(poll_data.ready_inodes.is_empty());
        drop(rx); // ensure channel is dropped
    }

    #[test]
    fn test_register_and_unregister_poll_handle() {
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(None)));
        let ph1: u64 = 1001;
        let ino1: u64 = 1;
        let events1: u32 = libc::POLLIN as u32;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.register_poll_handle(ph1, ino1, events1);

            assert_eq!(poll_data.registered_poll_handles.get(&ph1), Some(&(ino1, events1)));
            assert!(poll_data.inode_poll_handles.get(&ino1).unwrap().contains(&ph1));
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.unregister_poll_handle(ph1);
            assert!(!poll_data.registered_poll_handles.contains_key(&ph1));
            assert!(!poll_data.inode_poll_handles.contains_key(&ino1));
        }
    }

    #[test]
    fn test_mark_inode_ready_sends_event() {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Some(tx))));

        let ph1: u64 = 1002;
        let ino1: u64 = 2;
        let events1: u32 = libc::POLLIN as u32;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.register_poll_handle(ph1, ino1, events1);
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);
        }

        match rx.try_recv() {
            Ok((ph_recv, events_recv)) => {
                assert_eq!(ph_recv, ph1);
                assert_eq!(events_recv, libc::POLLIN as u32);
            }
            Err(e) => panic!("Expected to receive a poll event, but got error: {}", e),
        }
    }

    #[test]
    fn test_mark_inode_ready_sends_event_only_for_requested_events() {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Some(tx))));

        let ph1: u64 = 1003;
        let ino1: u64 = 3;
        let requested_events_in: u32 = libc::POLLIN as u32;
        let _requested_events_out: u32 = libc::POLLOUT as u32; // Prefixed with underscore

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // ph1 is interested in POLLIN
            poll_data.register_poll_handle(ph1, ino1, requested_events_in);
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark inode ready for POLLOUT. ph1 should not be notified.
            poll_data.mark_inode_ready(ino1, libc::POLLOUT as u32);
        }
        assert!(rx.try_recv().is_err(), "Should not receive event if not requested");

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark inode ready for POLLIN. ph1 should be notified.
            poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);
        }
        match rx.try_recv() {
            Ok((ph_recv, events_recv)) => {
                assert_eq!(ph_recv, ph1);
                assert_eq!(events_recv, libc::POLLIN as u32);
            }
            Err(e) => panic!("Expected to receive a POLLIN event, but got error: {}", e),
        }
    }

    #[test]
    fn test_initial_notification_if_already_ready() {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Some(tx))));

        let ph1: u64 = 1004;
        let ino1: u64 = 4;
        let events1: u32 = libc::POLLIN as u32;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark ino1 as ready *before* registering the poll handle
            poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);
        }

        // Clear any messages from the mark_inode_ready call (which should be none as no handle was registered yet)
        while rx.try_recv().is_ok() {}

        let initial_event_mask: Option<u32>;
        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Now register the poll handle
            initial_event_mask = poll_data.register_poll_handle(ph1, ino1, events1);
        }

        assert_eq!(initial_event_mask, Some(libc::POLLIN as u32), "Initial event mask should be POLLIN");

        match rx.try_recv() {
            Ok((ph_recv, events_recv)) => {
                assert_eq!(ph_recv, ph1);
                assert_eq!(events_recv, libc::POLLIN as u32);
            }
            Err(e) => panic!("Expected to receive an initial poll event, but got error: {}", e),
        }
    }

    #[test]
    fn test_mark_inode_not_ready() {
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(None)));
        let ino1: u64 = 5;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);
            assert!(poll_data.ready_inodes.contains(&ino1));
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.mark_inode_not_ready(ino1);
            assert!(!poll_data.ready_inodes.contains(&ino1));
        }
    }

    #[test]
    fn test_set_sender() { // Renamed test function
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(None)));
        assert!(poll_data_arc.lock().unwrap().ready_events_sender.is_none());

        let (tx, _rx) = unbounded();
        poll_data_arc.lock().unwrap().set_sender(tx); // Use set_sender
        assert!(poll_data_arc.lock().unwrap().ready_events_sender.is_some());
    }
}
