use fuser::Notifier;
use std::collections::{HashMap, HashSet};

/// `PollData` holds the state required for managing asynchronous poll notifications.
/// It is typically owned by a `Filesystem` implementation. The `Sender` end of its
/// MPMC channel (`ready_events_sender`) is provided by the `Session` to the
/// `Filesystem` (e.g., via a method like `init_poll_sender`).
#[derive(Debug)]
pub struct PollData {
    /// Sender part of the MPMC channel for (`poll_handle`, `events_bitmask`).
    /// This is used by the filesystem logic to send readiness events.
    /// Typically set via `PollData::new` or `PollData::set_sender`.
    pub ready_events_sender: Notifier,
    /// Stores registered poll handles.
    /// Maps a kernel poll handle (`u64`) to a tuple of (inode, `requested_events`).
    /// This allows us to know which inode and which events a poll handle is interested in.
    pub registered_poll_handles: HashMap<u64, (u64, u32)>,
    /// Stores active poll handles for a given inode.
    /// Maps an inode (`u64`) to a set of kernel poll handles (`u64`).
    /// This is useful to quickly find all poll handles interested in a particular inode
    /// when that inode's state changes.
    pub inode_poll_handles: HashMap<u64, HashSet<u64>>,
    /// Tracks inodes that are currently ready for I/O (e.g., POLLIN).
    /// This set is updated by filesystem operations.
    /// Stores the actual current readiness mask for each inode.
    pub ready_inodes: HashMap<u64, u32>,
}

impl PollData {
    /// Creates a new `PollData` instance, optionally with an initial sender.
    pub fn new(sender: Notifier) -> Self {
        PollData {
            ready_events_sender: sender,
            registered_poll_handles: HashMap::new(),
            inode_poll_handles: HashMap::new(),
            ready_inodes: HashMap::new(),
        }
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
        // Check if the file is already ready for any of the requested events.
        if let Some(current_readiness_mask) = self.ready_inodes.get(&ino) {
            let initial_events_to_send = events_requested & *current_readiness_mask;
            if initial_events_to_send != 0 {
                log::debug!(
                    "PollData::register_poll_handle() sending initial event: handle={ph}, initial_events_to_send={initial_events_to_send:#x}"
                );
                // Return the subset of requested events that are currently ready.
                return Some(initial_events_to_send);
            }
        }
        // If events are not ready, then register the poll handler for later.
        self.registered_poll_handles
            .insert(ph, (ino, events_requested));
        self.inode_poll_handles.entry(ino).or_default().insert(ph);
        None
    }

    // NOTE: the example does not currently process poll cancellations.
    // If it did, it would use this convenience function.
    #[allow(unused)]
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

    /// Marks an inode as ready for I/O and notifies registered poll handles.
    ///
    /// # Arguments
    ///
    /// * `ino`: The inode number that has become ready.
    /// * `ready_events`: The bitmask of events that are now active for the inode (e.g., `libc::POLLIN`).
    pub fn mark_inode_ready(&mut self, ino: u64, ready_events_mask: u32) {
        log::info!(
            "PollData::mark_inode_ready() called: ino={ino}, ready_events_mask={ready_events_mask:#x}"
        );
        // Update the readiness state for the inode or insert it if new.
        // If an inode becomes ready for POLLIN, then later for POLLOUT,
        // its readiness mask should reflect both (POLLIN | POLLOUT).
        let current_mask = self.ready_inodes.entry(ino).or_insert(0);
        *current_mask |= ready_events_mask;

        let mut handles_to_unregister = Vec::new();
        if let Some(poll_handles) = self.inode_poll_handles.get(&ino) {
            for &ph in poll_handles {
                if let Some((_ino_of_ph, requested_events_for_ph)) =
                    self.registered_poll_handles.get(&ph)
                {
                    // Notify if any of the newly ready events are requested by this handle.
                    let events_to_send = *requested_events_for_ph & ready_events_mask;
                    if events_to_send != 0 {
                        self.ready_events_sender.poll(ph);
                        handles_to_unregister.push(ph);
                    }
                }
            }
        }
        for ph in handles_to_unregister {
            self.unregister_poll_handle(ph);
        }
    }

    /// Marks an inode as no longer ready for specific I/O events.
    ///
    /// This function clears the specified event bits from the inode's readiness mask.
    /// If the resulting readiness mask is zero, the inode is removed from the set of
    /// ready inodes.
    ///
    /// # Arguments
    ///
    /// * `ino`: The inode number.
    /// * `no_longer_ready_events_mask`: A bitmask of events that are no longer ready for the inode.
    pub fn mark_inode_not_ready(&mut self, ino: u64, no_longer_ready_events_mask: u32) {
        if let Some(current_mask) = self.ready_inodes.get_mut(&ino) {
            *current_mask &= !no_longer_ready_events_mask; // Clear the bits
            if *current_mask == 0 {
                self.ready_inodes.remove(&ino);
            }
        }
        // Note: FUSE usually doesn't have explicit "not ready anymore" notifications for poll,
        // other than timeout. Applications will re-poll if needed.
        // However, managing this state internally is important for subsequent poll registrations
        // and for correctly reporting initial readiness.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::{Arc, Mutex};
    use fuser::NotificationKind;

    #[test]
    fn test_poll_data_new() {
        let (tx, rx) = unbounded();
        let poll_data = PollData::new(Notifier::new(tx));
        assert!(poll_data.registered_poll_handles.is_empty());
        assert!(poll_data.inode_poll_handles.is_empty());
        assert!(poll_data.ready_inodes.is_empty());
        drop(rx); // ensure channel is dropped
    }

    #[test]
    fn test_register_and_unregister_poll_handle() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Notifier::new(tx))));
        let ph1: u64 = 1001;
        let ino1: u64 = 1;
        let events1: u32 = libc::POLLIN as u32;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            poll_data.register_poll_handle(ph1, ino1, events1);

            assert_eq!(
                poll_data.registered_poll_handles.get(&ph1),
                Some(&(ino1, events1))
            );
            assert!(
                poll_data
                    .inode_poll_handles
                    .get(&ino1)
                    .unwrap()
                    .contains(&ph1)
            );
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
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Notifier::new(tx))));

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
            Ok(NotificationKind::Poll(poll)) => {
                assert_eq!(poll, ph1);
            }
            Ok(_) => panic!("Unexpected notification type"),
            Err(e) => panic!("Expected to receive a poll event, but got error: {}", e),
        }
    }

    #[test]
    fn test_mark_inode_ready_sends_event_only_for_requested_events() {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Notifier::new(tx))));

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
        assert!(
            rx.try_recv().is_err(),
            "Should not receive event if not requested"
        );

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark inode ready for POLLIN. ph1 should be notified.
            poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);
        }
        match rx.try_recv() {
            Ok(NotificationKind::Poll(poll)) => {
                assert_eq!(poll, ph1);
            }
            Ok(_) => panic!("Unexpected notification type"),
            Err(e) => panic!("Expected to receive a POLLIN event, but got error: {}", e),
        }
    }

    #[test]
    fn test_no_initial_notification_if_already_ready() {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Notifier::new(tx))));

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

        assert_eq!(
            initial_event_mask,
            Some(libc::POLLIN as u32),
            "Initial event mask should be POLLIN"
        );

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_mark_inode_not_ready() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Notifier::new(tx))));
        let ino1: u64 = 5;
        let poll_in_event = libc::POLLIN as u32;
        let poll_out_event = libc::POLLOUT as u32;

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark ready for POLLIN and POLLOUT
            poll_data.mark_inode_ready(ino1, poll_in_event | poll_out_event);
            assert_eq!(
                poll_data.ready_inodes.get(&ino1),
                Some(&(poll_in_event | poll_out_event))
            );
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark no longer ready for POLLIN
            poll_data.mark_inode_not_ready(ino1, poll_in_event);
            // Should still be ready for POLLOUT
            assert_eq!(poll_data.ready_inodes.get(&ino1), Some(&poll_out_event));
        }

        {
            let mut poll_data = poll_data_arc.lock().unwrap();
            // Mark no longer ready for POLLOUT
            poll_data.mark_inode_not_ready(ino1, poll_out_event);
            // Should not be ready for anything, so removed from map
            assert!(!poll_data.ready_inodes.contains_key(&ino1));
        }
    }

    #[test]
    fn test_check_replies() {
        let (tx, rx) = unbounded();
        let mut poll_data = PollData::new(Notifier::new(tx));
        let ph1: u64 = 2001;
        let ino1: u64 = 6;
        let events1: u32 = libc::POLLIN as u32;

        poll_data.register_poll_handle(ph1, ino1, events1);
        poll_data.mark_inode_ready(ino1, libc::POLLIN as u32);

        // Simulate receiving a notification and sending a reply
        if let Ok(NotificationKind::Poll(message)) = rx.try_recv() {
            assert_eq!(message, ph1);
        } else {
            panic!("Failed to receive notification");
        }
    }
}
