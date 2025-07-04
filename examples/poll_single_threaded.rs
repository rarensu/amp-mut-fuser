// Based on libfuse's example/poll.c and fuser's examples/poll.rs
// This version is adapted to use the single-threaded session runner.
//
//    Copyright (C) 2008       SUSE Linux Products GmbH
//    Copyright (C) 2008       Tejun Heo <teheo@suse.de>
//
// Translated to Rust/fuser by Zev Weiss <zev@bewilderbeest.net>
// Adapted for single-threaded example by AI.
//
// Due to the above provenance, unlike the rest of fuser this file is
// licensed under the terms of the GNU GPLv2.

// Requires feature = "abi-7-11"

use std::{
    convert::TryInto,
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, UNIX_EPOCH},
};

#[cfg(feature = "abi-7-11")]
use crossbeam_channel::Sender;
use fuser::{
    consts::FOPEN_DIRECT_IO, // FOPEN_NONSEEKABLE removed as it might conflict with some poll tests if not handled carefully
    Attr,
    DirEntry,
    Entry,
    Errno,
    FileAttr,
    FileType,
    Filesystem,
    MountOption,
    Open,
    PollData,
    RequestMeta,
    FUSE_ROOT_ID,
};

const NUMFILES: u8 = 16;
const MAXBYTES: u64 = 10;

struct FSelData {
    bytecnt: [u64; NUMFILES as usize],
    open_mask: u16,
}

struct FSelFS {
    data: Arc<Mutex<FSelData>>,
    poll_handler: Arc<Mutex<PollData>>, // PollData for managing poll state
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
            _ => return Err(Errno::ENOENT),
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
                size: 0, blocks: 0, atime: UNIX_EPOCH, mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH, crtime: UNIX_EPOCH, kind: FileType::Directory,
                perm: 0o555, nlink: 2, uid: 0, gid: 0, rdev: 0, flags: 0, blksize: 0,
            };
            return Ok(Attr { ttl: Duration::ZERO, attr: a });
        }
        let idx = FSelData::ino_to_idx(ino);
        if idx < NUMFILES {
            Ok(Attr { attr: self.get_data().filestat(idx), ttl: Duration::ZERO })
        } else {
            Err(Errno::ENOENT)
        }
    }

    fn readdir(
        &mut self, _req: RequestMeta, ino: u64, _fh: u64, offset: i64,
        _max_bytes: u32, // Added missing parameter to match trait
    ) -> Result<Vec<DirEntry>, Errno> {
        if ino != FUSE_ROOT_ID { return Err(Errno::ENOTDIR); }
        let Ok(start_offset): Result<u8, _> = offset.try_into() else { return Err(Errno::EINVAL); };
        let mut entries = Vec::new();
        for idx in start_offset..NUMFILES {
            let ascii_char_val = match idx {
                0..=9 => b'0' + idx,
                10..=15 => b'A' + idx - 10,
                _ => panic!("idx out of range"),
            };
            let name = OsString::from_vec(vec![ascii_char_val]);
            entries.push(DirEntry {
                ino: FSelData::idx_to_ino(idx),
                offset: (idx + 1).into(),
                kind: FileType::RegularFile,
                name,
            });
        }
        Ok(entries)
    }

    fn open(&mut self, _req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        let idx = FSelData::ino_to_idx(ino);
        if idx >= NUMFILES { return Err(Errno::ENOENT); }
        if (flags & libc::O_ACCMODE) != libc::O_RDONLY { return Err(Errno::EACCES); }
        {
            let mut d = self.get_data();
            if d.open_mask & (1 << idx) != 0 { return Err(Errno::EBUSY); }
            d.open_mask |= 1 << idx;
        }
        Ok(Open { fh: idx.into(), flags: FOPEN_DIRECT_IO })
    }

    fn release(
        &mut self, _req: RequestMeta, _ino: u64, fh: u64, _flags: i32,
        _lock_owner: Option<u64>, _flush: bool,
    ) -> Result<(), Errno> {
        let idx = fh;
        if idx >= NUMFILES.into() { return Err(Errno::EBADF); }
        self.get_data().open_mask &= !(1 << idx);
        Ok(())
    }

    fn read(
        &mut self, _req: RequestMeta, _ino: u64, fh: u64, _offset: i64,
        max_size: u32, _flags: i32, _lock_owner: Option<u64>,
    ) -> Result<Vec<u8>, Errno> {
        let Ok(idx): Result<u8, _> = fh.try_into() else { return Err(Errno::EINVAL); };
        if idx >= NUMFILES { return Err(Errno::EBADF); }

        let size;
        { // Scope for fsel_data_guard
            let mut fsel_data_guard = self.get_data();
            let cnt = &mut fsel_data_guard.bytecnt[idx as usize];
            size = (*cnt).min(max_size.into());
            log::info!("READ   {:X} transferred={} cnt={}", idx, size, *cnt);
            *cnt -= size;
            if *cnt == 0 {
                // If count is zero, mark inode as not ready for reading (POLLIN).
                // This is crucial for poll to work correctly.
                log::debug!("READ: Marking ino {} as NOT ready for POLLIN (bytecnt is 0)", FSelData::idx_to_ino(idx));
                self.get_poll_handler().mark_inode_not_ready(FSelData::idx_to_ino(idx), libc::POLLIN as u32);
            }
        } // fsel_data_guard is dropped here

        let elt = match idx {
            0..=9 => b'0' + idx,
            10..=15 => b'A' + idx - 10,
            _ => panic!("idx out of range"),
        };
        let data = vec![elt; size.try_into().unwrap()];
        Ok(data)
    }

    #[cfg(feature = "abi-7-11")]
    fn poll(
        &mut self, _req: RequestMeta, _ino: u64, fh: u64, ph: u64,
        events: u32, _flags: u32, // _flags (FUSE_POLL_SCHEDULE_NOTIFY) is implicitly handled by PollData
    ) -> Result<u32, Errno> {
        log::info!("poll() called: fh={}, ino={}, ph={}, events={:x}", fh, _ino, ph, events);
        let file_idx: u8 = fh.try_into().expect("fh should be a valid index for poll");
        let ino_for_poll = FSelData::idx_to_ino(file_idx);

        // Check current readiness before registering.
        let initial_revents = if self.get_data().bytecnt[file_idx as usize] > 0 {
            log::debug!("poll(): File {:X} (ino {}) has data, initially ready for POLLIN.", file_idx, ino_for_poll);
            libc::POLLIN as u32 // Ready for reading
        } else {
            log::debug!("poll(): File {:X} (ino {}) has no data, initially not ready for POLLIN.", file_idx, ino_for_poll);
            0 // Not ready for reading
        };

        // Register the poll handle. PollData will manage sending notifications if readiness changes.
        self.get_poll_handler().register_poll_handle(ph, ino_for_poll, events);
        log::debug!("poll(): Registered poll handle {} for ino {}, requested events {:x}. Initial revents: {:x}", ph, ino_for_poll, events, initial_revents);

        Ok(initial_revents & events) // Return only requested events that are ready
    }

    #[cfg(feature = "abi-7-11")]
    fn init_poll_sender(&mut self, sender: Sender<(u64, u32)>) -> Result<(), Errno> {
        log::info!("init_poll_sender() called for FSelFS");
        self.get_poll_handler().set_sender(sender);
        Ok(())
    }
}

fn producer_thread_main(fsel_data_arc: Arc<Mutex<FSelData>>, poll_handler_arc: Arc<Mutex<PollData>>) {
    let mut current_file_idx_producer: u8 = 0;
    let mut nr = 1; // Number of files to update per iteration
    loop {
        thread::sleep(Duration::from_millis(500)); // Slower production for easier observation
        {
            let mut fsel_data_guard = fsel_data_arc.lock().unwrap();
            let mut poll_handler_guard = poll_handler_arc.lock().unwrap();

            let mut t = current_file_idx_producer;
            for _ in 0..nr {
                let tidx = t as usize;
                if fsel_data_guard.bytecnt[tidx] < MAXBYTES {
                    fsel_data_guard.bytecnt[tidx] += 1;
                    log::info!("PRODUCER: Increased bytecnt for file {:X} (ino {}) to {}", t, FSelData::idx_to_ino(t), fsel_data_guard.bytecnt[tidx]);
                    // Important: Mark inode as ready AFTER updating bytecount
                    poll_handler_guard.mark_inode_ready(FSelData::idx_to_ino(t), libc::POLLIN as u32);
                }
                t = (t + NUMFILES / nr) % NUMFILES;
            }
            current_file_idx_producer = (current_file_idx_producer + 1) % NUMFILES;
            if current_file_idx_producer == 0 {
                nr = (nr % (NUMFILES / 2)) + 1;
            }
        } // Mutex guards are dropped
    }
}

fn main() {
    let options = vec![MountOption::RO, MountOption::FSName("fsel_single_threaded".to_string())];
    env_logger::init();
    log::info!("Starting fsel_single_threaded example");

    let data_arc = Arc::new(Mutex::new(FSelData {
        bytecnt: [0; NUMFILES as usize],
        open_mask: 0,
    }));
    // PollData is created here. The sender will be set by Session via init_poll_sender.
    let poll_handler_arc = Arc::new(Mutex::new(PollData::new(None)));

    let fsel_fs = FSelFS {
        data: Arc::clone(&data_arc),
        poll_handler: Arc::clone(&poll_handler_arc),
    };

    let mntpt_arg = std::env::args().nth(1).expect("Expected mountpoint argument");
    let mntpt = std::path::Path::new(&mntpt_arg);

    // Spawn the producer thread
    let producer_data_arc = Arc::clone(&data_arc);
    let producer_poll_handler_arc = Arc::clone(&poll_handler_arc);
    thread::spawn(move || {
        producer_thread_main(producer_data_arc, producer_poll_handler_arc);
    });

    println!("FUSE filesystem 'fsel_single_threaded' mounting on {}. Press Ctrl-C to unmount.", mntpt.display());

    // Create and run the session in single-threaded mode
    let mut session = fuser::Session::new(fsel_fs, mntpt, &options)
        .unwrap_or_else(|e| panic!("Failed to create FUSE session on {}: {}", mntpt.display(), e));

    // Setup Ctrl-C handler to gracefully unmount and exit
    // This needs to be done carefully as session.unmount() might be called from another thread
    // For simplicity in an example, we'll rely on the main thread exiting to drop the session,
    // which should trigger unmount. A more robust solution might involve a signal channel.
    let unmounter = session.unmount_callable(); // Get a callable unmounter
    ctrlc::set_handler(move || {
        println!("\nCtrl-C pressed, attempting to unmount {} and shut down...", mntpt_arg);
        // It's tricky to call unmounter.unmount() directly here as it might block
        // or session might be busy. The simplest is to trigger main thread exit.
        // For this example, we'll just exit, relying on Drop of Session.
        // In a real app, signal the main loop to break and then unmount.
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");

    if let Err(e) = session.run_single_threaded() {
        log::error!("Session run_single_threaded failed: {}", e);
    }

    log::info!("Session ended. Filesystem unmounted (implicitly on drop or explicitly if unmount was called).");
}
