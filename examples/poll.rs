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

#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Receiver, Sender};
use fuser::{
    consts::{FOPEN_DIRECT_IO, FOPEN_NONSEEKABLE, FUSE_POLL_SCHEDULE_NOTIFY},
    Attr,
    DirEntry,
    Entry,
    Errno,
    FileAttr,
    FileType,
    Filesystem, // Removed SharedPollData
    MountOption,
    Open,
    PollData,
    RequestMeta,
    FUSE_ROOT_ID,
}; // For PollData initialization and test setup

const NUMFILES: u8 = 16;
const MAXBYTES: u64 = 10;

struct FSelData {
    bytecnt: [u64; NUMFILES as usize],
    open_mask: u16,
}

struct FSelFS {
    data: Arc<Mutex<FSelData>>, // This remains for original example's byte counting logic
    poll_handler: Arc<Mutex<PollData>>, //  Helper functions for handling polls
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
    fn get_poll_handler(&self) -> std::sync::MutexGuard<'_, PollData> {
        log::debug!("Accessing poll handler (PollData)");
        self.poll_handler.lock().unwrap()
    }
}

impl Filesystem for FSelFS { 
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
        // if cnt is now equal to zero, mark the node as not ready. 
        if *cnt == 0 {
            self.get_poll_handler().mark_inode_not_ready(FSelData::idx_to_ino(idx));
        }
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
        _flags: u32,
    ) -> Result<u32, Errno> {
        log::info!("poll() called: fh={fh}, ph={ph}, events={_events}");
        let ino = FSelData::idx_to_ino(fh.try_into().expect("fh should be a valid index"));
        if let Some(initial_events) = self.get_poll_handler().register_poll_handle(ph, ino, _events) {
            log::debug!("poll(): Registered poll handle {ph} for ino {ino}, initial_events={initial_events}");
            Ok(initial_events)
        } else {
            log::debug!("poll(): Registered poll handle {ph} for ino {ino}, no initial events");
            Ok(0)
        }
    }

    #[cfg(feature = "abi-7-11")]
    fn init_poll_sender(&mut self, sender: Sender<(u64, u32)>) -> Result<(), Errno> {
        log::info!("init_poll_sender() called");
        self.get_poll_handler().set_sender(sender);
        Ok(())
    }
}

// Producer now takes a mutable reference to PollData (if called from same thread context)
// or needs its own way to access it if threaded separately from FS owner.
// For this example, assuming it can get a mutable reference or PollData is Arc<Mutex> internally if producer is separate.
// Given PollData is now owned by FSelFS, producer needs access to FSelFS or its PollData.
// Let's pass a clone of Arc<Mutex<FSelData>> and Arc<Mutex<PollData>> if FSelFS held PollData that way,
// but FSelFS now owns PollData directly.
// For simplicity in example, let producer take &mut PollData. This implies producer logic might need rethinking
// if it's truly concurrent with FUSE operations modifying PollData.
// However, PollData methods are internally consistent. The main issue is concurrent access to the PollData struct itself.
// Let's assume FSelFS's PollData is what producer needs to modify.
// The producer function will need to be adapted to how it gets access to PollData.
// For this iteration, let's assume producer gets a Arc<Mutex<PollData>> or similar,
// or the example structure changes for producer to be part of FSelFS or have shared access.
// Plan: Producer will take direct mutable access to FSelFS.poll_data for simplicity of change.
// This is not ideal for true concurrency but matches the spirit of the example's direct manipulation.
// A better way: producer takes Arc<Mutex<PollData>> if PollData were shared.
// Since FSelFS owns PollData, and producer is a separate thread, producer must get PollData through FSelFS.

// Let's refine: Producer will take direct mutable access to FSelFS's poll_data.
// This means FSelFS itself needs to be shareable, e.g. Arc<Mutex<FSelFS>>.
// Or, producer gets a clone of an Arc<Mutex<PollData>> if PollData itself was wrapped.
// Since FSelFS now owns PollData directly, the simplest way for the example producer
// is to operate on an Arc<Mutex<FSelFS>>.

// Producer function needs to be rethought.
// Original producer directly manipulated FSelData and called Notifier.
// New producer needs to manipulate FSelData and tell PollData (owned by FSelFS) that an Inode is ready.
// The `poll_data_arc: SharedPollData` argument for producer is removed.
// It will now operate on `fsel_fs_arc: Arc<Mutex<FSelFS>>`.

fn producer(fsel_data_arc: Arc<Mutex<FSelData>>, poll_handler_arc: Arc<Mutex<PollData>>) {
    let mut current_file_idx_producer: u8 = 0;
    let mut nr = 1;
    loop {
        {
            // fsel_data_guard is for the byte counts
            let mut fsel_data_guard = fsel_data_arc.lock().unwrap();
            
            let mut t = current_file_idx_producer;
            for _ in 0..nr {
                let tidx = t as usize;
                if fsel_data_guard.bytecnt[tidx] < MAXBYTES {
                    fsel_data_guard.bytecnt[tidx] += 1;
                    log::info!("PRODUCER: Increased bytecnt for file {:X} to {}", t, fsel_data_guard.bytecnt[tidx]);
                    {
                        let mut poll_handler_guard = poll_handler_arc.lock().unwrap();
                        log::debug!("PRODUCER: Marking ino {} as ready via poll handler", FSelData::idx_to_ino(t));
                        poll_handler_guard.mark_inode_ready(FSelData::idx_to_ino(t), libc::POLLIN as u32);
                        log::info!("PRODUCER: Marked ino {} as ready", FSelData::idx_to_ino(t));
                    }
                }
                t = (t + NUMFILES / nr) % NUMFILES;
            }
            current_file_idx_producer = (current_file_idx_producer + 1) % NUMFILES;
            if current_file_idx_producer == 0 {
                nr = (nr % (NUMFILES / 2)) + 1;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn main() {
    let options = vec![MountOption::RO, MountOption::FSName("fsel_chan".to_string())];
    env_logger::init();
    log::info!("Starting fsel_chan example with poll support");
    let data_arc = Arc::new(Mutex::new(FSelData { // For byte counts
        bytecnt: [0; NUMFILES as usize],
        open_mask: 0,
    }));
    let poll_handler_arc = Arc::new(Mutex::new(PollData::new(None)));

    // FSelFS now creates its own PollData. Sender will be set by Session.
    let fsel_fs = FSelFS {
        data: Arc::clone(&data_arc),
        poll_handler: Arc::clone(&poll_handler_arc), 
    };

    let mntpt = std::env::args().nth(1).expect("Expected mountpoint argument");

    let session = fuser::Session::new(
        fsel_fs,
        &mntpt,
        &options
    ).unwrap_or_else(|e| {
        panic!("Failed to create FUSE session on {}: {}", mntpt, e);
    });

    let bg = session.spawn().unwrap_or_else(|e| {
        panic!("Failed to spawn FUSE session: {}", e);
    });


    thread::spawn(move || {
        producer(data_arc, poll_handler_arc);
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
    use fuser::{Filesystem, RequestMeta, PollData}; // Ensure PollData is in scope
    use std::sync::{Arc, Mutex};
    use crossbeam_channel::unbounded;

    // Helper to create FSelFS and a channel pair for its PollData for tests
    fn setup_test_fs_with_channel() -> (FSelFS, Sender<(u64, u32)>, Receiver<(u64,u32)>) {
        log::debug!("Setting up test FS with poll channel");
        let (tx, rx) = unbounded();
        let fsel_data_arc = Arc::new(Mutex::new(FSelData {
            bytecnt: [0; NUMFILES as usize],
            open_mask: 0,
        }));
        // PollData with None sender.
        let poll_handler_arc = Arc::new(Mutex::new(PollData::new(None)));
        let fs = FSelFS {
            data: fsel_data_arc,
            poll_handler: poll_handler_arc,
        };
        (fs, tx, rx)
    }

    #[test]
    fn test_fs_poll_registers_handle_no_initial_event() {
        log::info!("test_fs_poll_registers_handle_no_initial_event: starting");
        let (mut fs, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        fs.init_poll_sender(tx_to_fs).unwrap(); // Link FS's PollData to our test sender

        let req = RequestMeta { unique: 0, uid: 0, gid: 0, pid: 0 };
        let idx: u8 = 0;
        let fh = idx as u64;
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 12345;
        let events = libc::POLLIN as u32;

        fs.get_data().bytecnt[idx as usize] = 0;
        fs.get_poll_handler().mark_inode_not_ready(ino); // Ensure PollData also knows it's not ready

        let result = fs.poll(req, ino, fh, ph, events, 0);
        log::debug!("test_fs_poll_registers_handle_no_initial_event: poll result = {:?}", result);
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(result.unwrap(), 0, "Should return 0 as no initial event is expected");

        assert!(fs.get_poll_handler().registered_poll_handles.contains_key(&ph));
        assert_eq!(fs.get_poll_handler().registered_poll_handles.get(&ph), Some(&(ino, events)));
        assert!(fs.get_poll_handler().inode_poll_handles.get(&ino).unwrap().contains(&ph));

        assert!(rx_from_fs.try_recv().is_err());
    }

    #[test]
    fn test_fs_poll_registers_handle_with_initial_event() {
        log::info!("test_fs_poll_registers_handle_with_initial_event: starting");
        let (mut fs, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        fs.init_poll_sender(tx_to_fs).unwrap();

        let req = RequestMeta { unique: 0, uid: 0, gid: 0, pid: 0 };
        let idx: u8 = 1;
        let fh = idx as u64;
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 54321;
        let events = libc::POLLIN as u32;

        fs.get_poll_handler().mark_inode_ready(ino, libc::POLLIN as u32);
        // Clear the channel from the mark_inode_ready call if any (no handle registered yet, so it shouldn't send)
        while rx_from_fs.try_recv().is_ok() {}

        let result = fs.poll(req, ino, fh, ph, events, 0);
        log::debug!("test_fs_poll_registers_handle_with_initial_event: poll result = {:?}", result);
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(result.unwrap(), libc::POLLIN as u32, "Should return POLLIN as an initial event");

        assert!(fs.get_poll_handler().registered_poll_handles.contains_key(&ph));

        match rx_from_fs.try_recv() {
            Ok((ph_recv, ev_recv)) => {
                assert_eq!(ph_recv, ph);
                assert_eq!(ev_recv, libc::POLLIN as u32);
            }
            Err(_) => panic!("Expected an initial event on the channel"),
        }
    }

    #[test]
    fn test_producer_marks_inode_ready_triggers_event() {
        log::info!("test_producer_marks_inode_ready_triggers_event: starting");
        // For this test, we need an Arc<Mutex<FSelFS>> because producer runs in a separate thread.
        let (mut fs_instance, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        fs_instance.init_poll_sender(tx_to_fs).unwrap();

        let idx_to_test: u8 = 2;
        let ino_to_test = FSelData::idx_to_ino(idx_to_test);
        let ph_to_test: u64 = 67890;
        let events_to_test = libc::POLLIN as u32;

        // Simulate a poll request being registered by directly accessing PollData via the Arc
        fs_instance.get_poll_handler().register_poll_handle(ph_to_test, ino_to_test, events_to_test);
        while rx_from_fs.try_recv().is_ok() {} // Clear channel

        // Manually simulate one iteration of the producer logic for a specific file
        fs_instance.get_data().bytecnt[idx_to_test as usize] = 1;
        fs_instance.get_poll_handler().mark_inode_ready(ino_to_test, libc::POLLIN as u32);
        log::debug!("test_producer_marks_inode_ready_triggers_event: marked inode ready");

        match rx_from_fs.try_recv() {
            Ok((ph_recv, ev_recv)) => {
                assert_eq!(ph_recv, ph_to_test);
                assert_eq!(ev_recv, libc::POLLIN as u32);
            }
            Err(_) => panic!("Producer marking inode ready should have triggered an event on the channel"),
        }
    }
}
