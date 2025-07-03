// Translated from libfuse's example/poll.c:
//    Copyright (C) 2008       SUSE Linux Products GmbH
//    Copyright (C) 2008       Tejun Heo <teheo@suse.de>
//
// Translated to Rust/fuser by Zev Weiss <zev@bewilderbeest.net>
//
// Due to the above provenance, unlike the rest of fuser this file is
// licensed under the terms of the GNU GPLv2.

// Requires feature = "abi-7-11"

use std::{
    convert::TryInto,
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt}, // for converting to and from
    sync::{
        atomic::{AtomicU64, Ordering::SeqCst},
        Arc, Mutex,
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

use fuser::{
    consts::{FOPEN_DIRECT_IO, FOPEN_NONSEEKABLE, FUSE_POLL_SCHEDULE_NOTIFY},
    FileAttr, FileType, MountOption, RequestMeta, Entry, Attr, DirEntry, Open, Errno, FUSE_ROOT_ID,
    PollData, SharedPollData, Filesystem, // Added PollData, SharedPollData, Filesystem
};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Sender, Receiver}; // For PollData initialization and test setup

const NUMFILES: u8 = 16;
const MAXBYTES: u64 = 10;

struct FSelData {
    bytecnt: [u64; NUMFILES as usize],
    open_mask: u16,
    notify_mask: u16,
    poll_handles: [u64; NUMFILES as usize],
}

struct FSelFS {
    data: Arc<Mutex<FSelData>>, // This remains for original example's byte counting logic
    poll_data_arc: SharedPollData, // New field for channel-based polling
}

impl FSelData {
    fn idx_to_ino(idx: u8) -> u64 {
        let idx: u64 = idx.into();
        FUSE_ROOT_ID + idx + 1
    }

    fn ino_to_idx(ino: u64) -> u8 {
        (ino - (FUSE_ROOT_ID + 1))
            .try_into()
            .expect("out-of-range inode number")
    }

    fn filestat(&self, idx: u8) -> FileAttr {
        assert!(idx < NUMFILES);
        FileAttr {
            ino: Self::idx_to_ino(idx),
            size: self.bytecnt[idx as usize],
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::RegularFile,
            perm: 0o444,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
            blksize: 0,
        }
    }
}

impl FSelFS {
    fn get_data(&self) -> std::sync::MutexGuard<'_, FSelData> {
        self.data.lock().unwrap()
    }
}

impl Filesystem for FSelFS { // Changed from fuser::Filesystem to just Filesystem to pick up local trait
    fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsString) -> Result<Entry, Errno> {
        if parent != FUSE_ROOT_ID || name.len() != 1 {
            return Err(Errno::ENOENT);
        }

        let name_bytes = name.as_bytes();

        let idx = match name_bytes[0] {
            b'0'..=b'9' => name_bytes[0] - b'0',
            b'A'..=b'F' => name_bytes[0] - b'A' + 10,
            _ => {
                return Err(Errno::ENOENT);
            }
        };

        Ok(Entry {
            attr: self.get_data().filestat(idx),
            ttl: Duration::ZERO,
            generation: 0,
        })
    }

    fn getattr(&mut self, _req: RequestMeta, ino: u64, _fh: Option<u64>) -> Result<Attr, Errno> {
        if ino == FUSE_ROOT_ID {
            let a = FileAttr {
                ino: FUSE_ROOT_ID,
                size: 0,
                blocks: 0,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind: FileType::Directory,
                perm: 0o555,
                nlink: 2,
                uid: 0,
                gid: 0,
                rdev: 0,
                flags: 0,
                blksize: 0,
            };
            return Ok(Attr { ttl: Duration::ZERO, attr: a });
        }
        let idx = FSelData::ino_to_idx(ino);
        if idx < NUMFILES {
            Ok(Attr {
                attr: self.get_data().filestat(idx),
                ttl: Duration::ZERO,
            })
        } else {
            Err(Errno::ENOENT)
        }
    }

    fn readdir(
        &mut self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<Vec<DirEntry>, Errno> {
        if ino != FUSE_ROOT_ID {
            return Err(Errno::ENOTDIR);
        }

        let Ok(start_offset): Result<u8, _> = offset.try_into() else {
            return Err(Errno::EINVAL);
        };

        let mut entries = Vec::new();
        for idx in start_offset..NUMFILES {
            let ascii_char_val = match idx {
                0..=9 => b'0' + idx,
                10..=15 => b'A' + idx - 10, // Corrected range to 15 for NUMFILES = 16
                _ => panic!("idx out of range for NUMFILES"),
            };
            let name_bytes = vec![ascii_char_val]; // Byte vector (but just one byte)
            let name = OsString::from_vec(name_bytes);
            entries.push(DirEntry {
                ino: FSelData::idx_to_ino(idx),
                offset: (idx + 1).into(),
                kind: FileType::RegularFile,
                name,
            });
            // TODO: compare to _max_bytes; stop if full.
        }
        Ok(entries)
    }

    fn open(&mut self, _req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        let idx = FSelData::ino_to_idx(ino);
        if idx >= NUMFILES {
            return Err(Errno::ENOENT);
        }

        if (flags & libc::O_ACCMODE) != libc::O_RDONLY {
            return Err(Errno::EACCES);
        }

        {
            let mut d = self.get_data();

            if d.open_mask & (1 << idx) != 0 {
                return Err(Errno::EBUSY);
            }
            d.open_mask |= 1 << idx;
        }

        Ok(Open {
            fh: idx.into(), // Using idx as file handle
            flags: FOPEN_DIRECT_IO | FOPEN_NONSEEKABLE,
        })
    }

    fn release(
        &mut self,
        _req: RequestMeta,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
    ) -> Result<(), Errno> {
        let idx = fh; // fh is the idx from open()
        if idx >= NUMFILES.into() {
            return Err(Errno::EBADF);
        }
        self.get_data().open_mask &= !(1 << idx);
        Ok(())
    }

    fn read(
        &mut self,
        _req: RequestMeta,
        _ino: u64,
        fh: u64,
        _offset: i64, // offset is ignored due to FOPEN_NONSEEKABLE
        max_size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Vec<u8>, Errno> {
        let Ok(idx): Result<u8, _> = fh.try_into() else {
            return Err(Errno::EINVAL);
        };
        if idx >= NUMFILES {
            return Err(Errno::EBADF);
        }
        let cnt = &mut self.get_data().bytecnt[idx as usize];
        let size = (*cnt).min(max_size.into());
        println!("READ   {:X} transferred={} cnt={}", idx, size, *cnt);
        *cnt -= size;
        let elt = match idx {
            0..=9 => b'0' + idx,
            10..=15 => b'A' + idx - 10, // Corrected range
            _ => panic!("idx out of range for NUMFILES"),
        };
        let data = vec![elt; size.try_into().unwrap()];
        Ok(data)
    }

    #[cfg(feature = "abi-7-11")]
    fn poll(
        &mut self,
        _req: RequestMeta,
        _ino: u64,
        fh: u64,
        ph: u64,
        _events: u32,
        _flags: u32, // flags (like FUSE_POLL_SCHEDULE_NOTIFY) are handled by PollData registration logic implicitly if needed
    ) -> Result<u32, Errno> {
        let ino = FSelData::idx_to_ino(fh.try_into().expect("fh should be a valid index"));
        let mut poll_data_guard = self.poll_data_arc.lock().map_err(|_| Errno::EIO)?;

        // Register the poll handle. PollData's register_poll_handle will check if
        // the file is already ready (based on its internal state, which producer updates)
        // and arrange for an immediate notification if so via the channel.
        if let Some(initial_events) = poll_data_guard.register_poll_handle(ph, ino, _events) {
            Ok(initial_events)
        } else {
            Ok(0) // No initial events, async notification will follow if/when ready
        }
    }

    #[cfg(feature = "abi-7-11")]
    fn poll_data(&self) -> Option<SharedPollData> {
        Some(Arc::clone(&self.poll_data_arc))
    }
}

// Producer now takes SharedPollData to mark inodes as ready
fn producer(data: Arc<Mutex<FSelData>>, poll_data_arc: SharedPollData) {
    let mut current_file_idx_producer: u8 = 0; // Renamed to avoid conflict if FSelData had 'idx'
    let mut nr = 1; // Number of files to update per iteration
    loop {
        {
            // Lock FSelData to update byte counts (original example logic)
            let mut d = data.lock().unwrap();
            // Lock PollData to mark inodes ready
            let mut poll_data_guard = poll_data_arc.lock().unwrap();

            let mut t = current_file_idx_producer;
            for _ in 0..nr {
                let tidx = t as usize;
                if d.bytecnt[tidx] < MAXBYTES { // Check against MAXBYTES
                    d.bytecnt[tidx] += 1;
                    println!("PRODUCER: Increased bytecnt for file {:X} to {}", t, d.bytecnt[tidx]);
                    // If bytecnt indicates readiness, mark it in PollData
                    // FSelData::idx_to_ino converts producer's file index `t` to an inode number
                    poll_data_guard.mark_inode_ready(FSelData::idx_to_ino(t), libc::POLLIN as u32);
                    println!("PRODUCER: Marked ino {} as ready", FSelData::idx_to_ino(t));
                }
                t = (t + NUMFILES / nr) % NUMFILES;
            }
            current_file_idx_producer = (current_file_idx_producer + 1) % NUMFILES;
            if current_file_idx_producer == 0 {
                nr = (nr % (NUMFILES / 2)) + 1; // Cycle nr, ensure it's at least 1 and not too large
            }
        } // Locks are released here
        thread::sleep(Duration::from_millis(250));
    }
}

fn main() {
    let options = vec![MountOption::RO, MountOption::FSName("fsel_chan".to_string())];

    // This is where the sender for PollData would be created if PollData needed one directly.
    // However, Session creates the channel and gives PollData the sender.
    // So, FSelFS's PollData will be initialized with None initially,
    // and Session will populate the sender.
    let poll_data_arc = Arc::new(Mutex::new(PollData::new(None)));

    let fsel_data_arc = Arc::new(Mutex::new(FSelData {
        bytecnt: [0; NUMFILES as usize],
        open_mask: 0,
        notify_mask: 0, // notify_mask is no longer used by producer for direct notification
        poll_handles: [0; NUMFILES as usize], // poll_handles stored in PollData now
    }));

    let fs = FSelFS {
        data: Arc::clone(&fsel_data_arc),
        poll_data_arc: Arc::clone(&poll_data_arc),
    };

    let mntpt = std::env::args().nth(1).expect("Expected mountpoint argument");
    // Session::new will call fs.poll_data() to get the poll_data_arc,
    // create a channel, and set the sender in the PollData instance.
    let session = fuser::Session::new(fs, &mntpt, &options).unwrap_or_else(|e| {
        panic!("Failed to create FUSE session on {}: {}", mntpt, e);
    });

    let bg = session.spawn().unwrap_or_else(|e| {
        panic!("Failed to spawn FUSE session: {}", e);
    });

    // Start the producer thread
    // It now needs access to the original FSelData (for byte counts)
    // and the SharedPollData (to mark files as ready).
    let producer_poll_data_arc = Arc::clone(&poll_data_arc);
    thread::spawn(move || {
        producer(fsel_data_arc, producer_poll_data_arc);
    });

    // Keep the main thread alive to keep the filesystem mounted.
    // bg.join() would block until unmount.
    // For an example that runs indefinitely:
    println!("FUSE filesystem 'fsel_chan' mounted on {}. Press Ctrl-C to unmount.", mntpt);
    let (_tx_shutdown, rx_shutdown) = crossbeam_channel::bounded::<()>(1);
    ctrlc::set_handler(move || {
        println!("\nCtrl-C pressed, shutting down...");
        // Dropping bg should trigger unmount.
        // If more explicit shutdown is needed, bg.join() or specific unmount call.
        // For this example, allowing main to exit will drop bg.
        // tx_shutdown.send(()).unwrap(); // Signal main to exit if it were waiting on rx_shutdown
        std::process::exit(0); // Exit directly for simplicity in example
    }).expect("Error setting Ctrl-C handler");

    // Wait indefinitely, or until Ctrl-C handler exits.
    // rx_shutdown.recv().unwrap(); // Would wait for signal if not exiting directly
    // bg.join(); // This would also work if we want main to wait for unmount
    loop {
        thread::park();
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use fuser::{Filesystem, RequestMeta, Errno, PollData}; // Ensure PollData is in scope
    use std::sync::{Arc, Mutex};
    use crossbeam_channel::unbounded;

    // Helper to create FSelFS with its PollData for tests
    fn setup_test_fs_with_poll_data() -> (FSelFS, Sender<(u64, u32)>, Receiver<(u64,u32)>) {
        let (tx, rx) = unbounded();
        let poll_data_arc = Arc::new(Mutex::new(PollData::new(Some(tx.clone()))));
        let fsel_data_arc = Arc::new(Mutex::new(FSelData {
            bytecnt: [0; NUMFILES as usize],
            open_mask: 0,
            notify_mask: 0,
            poll_handles: [0; NUMFILES as usize],
        }));
        let fs = FSelFS {
            data: fsel_data_arc,
            poll_data_arc,
        };
        (fs, tx, rx)
    }

    #[test]
    fn test_fs_poll_registers_handle_no_initial_event() {
        let (mut fs, _tx, rx) = setup_test_fs_with_poll_data();
        let req = RequestMeta { unique: 0, uid: 0, gid: 0, pid: 0 };
        let idx: u8 = 0;
        let fh = idx as u64; // fh is idx in this example
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 12345; // Kernel poll handle
        let events = libc::POLLIN as u32;

        // Ensure bytecnt is 0, so no initial event
        fs.data.lock().unwrap().bytecnt[idx as usize] = 0;
        // Also ensure PollData's ready_inodes is clear for this ino
        fs.poll_data_arc.lock().unwrap().mark_inode_not_ready(ino);


        let result = fs.poll(req, ino, fh, ph, events, 0);
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(result.unwrap(), 0, "Should return 0 as no initial event is expected");

        // Check if PollData registered the handle
        let poll_data_guard = fs.poll_data_arc.lock().unwrap();
        assert!(poll_data_guard.registered_poll_handles.contains_key(&ph));
        assert_eq!(poll_data_guard.registered_poll_handles.get(&ph), Some(&(ino, events)));
        assert!(poll_data_guard.inode_poll_handles.get(&ino).unwrap().contains(&ph));

        // No event should be on the channel yet
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_fs_poll_registers_handle_with_initial_event() {
        let (mut fs, _tx, rx) = setup_test_fs_with_poll_data();
        let req = RequestMeta { unique: 0, uid: 0, gid: 0, pid: 0 };
        let idx: u8 = 1;
        let fh = idx as u64;
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 54321;
        let events = libc::POLLIN as u32;

        // Make the file ready in PollData *before* calling fs.poll
        fs.poll_data_arc.lock().unwrap().mark_inode_ready(ino, libc::POLLIN as u32);
         // Clear the channel from the mark_inode_ready call if any (though no handle was registered yet)
        while rx.try_recv().is_ok() {}


        let result = fs.poll(req, ino, fh, ph, events, 0);
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(result.unwrap(), libc::POLLIN as u32, "Should return POLLIN as an initial event");

        // Check if PollData registered the handle
        let poll_data_guard = fs.poll_data_arc.lock().unwrap();
        assert!(poll_data_guard.registered_poll_handles.contains_key(&ph));

        // An event should be on the channel due to initial readiness
        match rx.try_recv() {
            Ok((ph_recv, ev_recv)) => {
                assert_eq!(ph_recv, ph);
                assert_eq!(ev_recv, libc::POLLIN as u32);
            }
            Err(_) => panic!("Expected an initial event on the channel"),
        }
    }

    #[test]
    fn test_producer_marks_inode_ready_triggers_event() {
        let (_fs, tx_pd, rx_pd) = setup_test_fs_with_poll_data(); // fs not directly used, but sets up poll_data_arc
        let poll_data_arc_for_producer = Arc::clone(&_fs.poll_data_arc);
        let fsel_data_for_producer = Arc::clone(&_fs.data);

        let idx_to_test: u8 = 2;
        let ino_to_test = FSelData::idx_to_ino(idx_to_test);
        let ph_to_test: u64 = 67890;
        let events_to_test = libc::POLLIN as u32;

        // Simulate a poll request being registered
        _fs.poll_data_arc.lock().unwrap().register_poll_handle(ph_to_test, ino_to_test, events_to_test);
        // Clear channel from registration if it sent something (it shouldn't if not initially ready)
        while rx_pd.try_recv().is_ok() {}


        // Manually simulate one iteration of the producer logic for a specific file
        {
            let mut fsel_data_guard = fsel_data_for_producer.lock().unwrap();
            let mut poll_data_guard = poll_data_arc_for_producer.lock().unwrap();

            fsel_data_guard.bytecnt[idx_to_test as usize] = 1; // Update byte count
            poll_data_guard.mark_inode_ready(ino_to_test, libc::POLLIN as u32); // Mark ready
        }

        // Check if an event was sent on the channel
        match rx_pd.try_recv() {
            Ok((ph_recv, ev_recv)) => {
                assert_eq!(ph_recv, ph_to_test);
                assert_eq!(ev_recv, libc::POLLIN as u32);
            }
            Err(_) => panic!("Producer marking inode ready should have triggered an event on the channel"),
        }
    }
}
