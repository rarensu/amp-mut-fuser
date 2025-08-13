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
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt}, // for converting to and from
    time::{Duration, UNIX_EPOCH},
};

#[cfg(feature = "abi-7-11")]
use crossbeam_channel::Sender;

mod poll_data;
use poll_data::PollData;

use async_trait::async_trait;
use bytes::Bytes;
use fuser::{
    Dirent, DirentList, Entry, Errno, FUSE_ROOT_ID, FileAttr, FileType, FsStatus, MountOption,
    Notification, Open, RequestMeta,
    consts::{FOPEN_DIRECT_IO, FOPEN_NONSEEKABLE, FUSE_POLL_SCHEDULE_NOTIFY},
    trait_async::Filesystem,
};

const NUMFILES: u8 = 16;
const MAXBYTES: u64 = 10;
const PRODUCER_INTERVAL: Duration = Duration::from_millis(250);

struct ProducerData {
    next_time: std::time::SystemTime,
    next_idx: u8,
    next_nr: u8,
}

impl ProducerData {
    fn advance(&mut self) {
        self.next_idx = (self.next_idx + 1) % NUMFILES;
        if self.next_idx == 0 {
            self.next_nr = (self.next_nr % (NUMFILES / 2)) + 1;
        }
        self.next_time += PRODUCER_INTERVAL;
    }
    fn is_ready(&self) -> bool {
        std::time::SystemTime::now() >= self.next_time
    }
}

struct FSelData {
    bytecnt: [u64; NUMFILES as usize],
}

use std::sync::{
    Mutex,
    atomic::{AtomicU16, Ordering as AtomicOrdering},
};

struct FSelFS {
    // Byte counts for each file
    data: Mutex<FSelData>,
    // Helper functions for handling polls
    poll_handler: Mutex<PollData>,
    // Helper functions for updating Byte counts
    producer: Mutex<ProducerData>,
    // open mask is heavily contended and fits in 16 bits
    open_mask: AtomicU16,
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
    fn produce_data(&self) {
        let producer = self.producer.lock().unwrap();
        let mut data = self.data.lock().unwrap();
        let mut poll_handler = self.poll_handler.lock().unwrap();
        let mut t = producer.next_idx;
        for _ in 0..producer.next_nr {
            let tidx = t as usize;
            if data.bytecnt[tidx] < MAXBYTES {
                data.bytecnt[tidx] += 1;
                log::info!(
                    "PRODUCER: Increased bytecnt for file {:X} to {}",
                    t,
                    data.bytecnt[tidx]
                );
                poll_handler.mark_inode_ready(FSelData::idx_to_ino(t), libc::POLLIN as u32);
            }
            t = (t + NUMFILES / producer.next_nr) % NUMFILES;
        }
    }
}

#[async_trait]
impl Filesystem for FSelFS {
    async fn heartbeat(&self) -> FsStatus {
        self.poll_handler.lock().unwrap().check_replies();
        if self.producer.lock().unwrap().is_ready() {
            self.produce_data();
            self.producer.lock().unwrap().advance();
        }
        FsStatus::Ready
    }

    async fn lookup(&self, _req: RequestMeta, parent: u64, name: &OsStr) -> Result<Entry, Errno> {
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
            ino: FSelData::idx_to_ino(idx),
            generation: None,
            file_ttl: Duration::ZERO,
            attr: self.data.lock().unwrap().filestat(idx),
            attr_ttl: Duration::ZERO,
        })
    }

    async fn getattr(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: Option<u64>,
    ) -> Result<(FileAttr, Duration), Errno> {
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
            return Ok((a, Duration::ZERO));
        }
        let idx = FSelData::ino_to_idx(ino);
        if idx < NUMFILES {
            Ok((self.data.lock().unwrap().filestat(idx), Duration::ZERO))
        } else {
            Err(Errno::ENOENT)
        }
    }

    async fn readdir(
        &self,
        _req: RequestMeta,
        ino: u64,
        _fh: u64,
        offset: i64,
        _max_bytes: u32,
    ) -> Result<DirentList, Errno> {
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
            // Create OsString from the single byte character
            let name_os_string = OsString::from_vec(vec![ascii_char_val]);
            let entry_data = Dirent {
                ino: FSelData::idx_to_ino(idx),
                offset: (idx + 1).into(),
                kind: FileType::RegularFile,
                // Convert the OsString back into an owned vector,
                // and then into an appropriate Bytes variant, in one step,
                // using the From trait.
                name: Bytes::from_owner(name_os_string.into_vec()),
            };
            entries.push(entry_data);
            // Fuser library will ensure that max_bytes is respected.
        }
        // convert the vector of entries into an appropriate DirentList variant,
        // using the Into trait
        Ok(entries.into())
    }

    async fn open(&self, _req: RequestMeta, ino: u64, flags: i32) -> Result<Open, Errno> {
        let idx = FSelData::ino_to_idx(ino);
        if idx >= NUMFILES {
            return Err(Errno::ENOENT);
        }

        if (flags & libc::O_ACCMODE) != libc::O_RDONLY {
            return Err(Errno::EACCES);
        }

        // atomically set bit, detect if already set
        let bit: u16 = 1 << idx;
        let prev = self.open_mask.fetch_or(bit, AtomicOrdering::AcqRel);
        if prev & bit != 0 {
            return Err(Errno::EBUSY);
        }

        Ok(Open {
            fh: idx.into(), // Using idx as file handle
            flags: FOPEN_DIRECT_IO | FOPEN_NONSEEKABLE,
            backing_id: None,
        })
    }

    async fn release(
        &self,
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
        let bit: u16 = 1 << (idx as u8);
        self.open_mask.fetch_and(!bit, AtomicOrdering::AcqRel);
        Ok(())
    }

    async fn read(
        &self,
        _req: RequestMeta,
        _ino: u64,
        fh: u64,
        _offset: i64, // offset is ignored due to FOPEN_NONSEEKABLE
        max_size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
    ) -> Result<Bytes, Errno> {
        let Ok(idx): Result<u8, _> = fh.try_into() else {
            return Err(Errno::EINVAL);
        };
        if idx >= NUMFILES {
            return Err(Errno::EBADF);
        }
        let mut data = self.data.lock().unwrap();
        let cnt = &mut data.bytecnt[idx as usize];
        let size = (*cnt).min(max_size.into());
        println!("READ   {:X} transferred={} cnt={}", idx, size, *cnt);
        *cnt -= size;
        // if cnt is now equal to zero, mark the node as not ready.
        if *cnt == 0 {
            // Mark the inode as no longer ready for POLLIN events specifically
            self.poll_handler
                .lock()
                .unwrap()
                .mark_inode_not_ready(FSelData::idx_to_ino(idx), libc::POLLIN as u32);
        }
        let elt = match idx {
            0..=9 => b'0' + idx,
            10..=15 => b'A' + idx - 10, // Corrected range
            _ => panic!("idx out of range for NUMFILES"),
        };
        let data = vec![elt; size.try_into().unwrap()];
        // example of converting to an explicit Owned Bytes variant
        Ok(Bytes::from_owner(data))
    }

    #[cfg(feature = "abi-7-11")]
    async fn poll(
        &self,
        _req: RequestMeta,
        _ino: u64,
        fh: u64,
        ph: u64,
        events: u32,
        flags: u32,
    ) -> Result<u32, Errno> {
        log::info!("poll() called: fh={fh}, ph={ph}, events={events}, flags={flags}");
        if flags & FUSE_POLL_SCHEDULE_NOTIFY == 0 {
            // TODO: handle this unexpected case.
        }
        let Ok(idx): Result<u8, _> = fh.try_into() else {
            return Err(Errno::EINVAL);
        };
        if idx >= NUMFILES {
            return Err(Errno::EBADF);
        }
        let ino = FSelData::idx_to_ino(idx);
        if let Some(initial_events) = self
            .poll_handler
            .lock()
            .unwrap()
            .register_poll_handle(ph, ino, events)
        {
            log::debug!(
                "poll(): Registered poll handle {ph} for ino {ino}, initial_events={initial_events}"
            );
            Ok(initial_events)
        } else {
            log::debug!("poll(): Registered poll handle {ph} for ino {ino}, no initial events");
            Ok(0)
        }
    }

    #[cfg(feature = "abi-7-11")]
    fn init_notification_sender(&mut self, sender: Sender<Notification>) -> bool {
        log::info!("init_poll_sender() called");
        self.poll_handler.lock().unwrap().set_sender(sender);
        true
    }
}

fn main() {
    let options = vec![MountOption::RO, MountOption::FSName("fsel".to_string())];
    env_logger::init();
    log::info!("Starting fsel example with poll support.");

    let data = FSelData {
        bytecnt: [0; NUMFILES as usize],
    };
    let poll_handler = PollData::new(None);
    let producer = ProducerData {
        next_time: std::time::SystemTime::now() + Duration::from_millis(1000),
        next_idx: 0,
        next_nr: 1,
    };
    let fs = FSelFS {
        data: Mutex::new(data),
        poll_handler: Mutex::new(poll_handler),
        producer: Mutex::new(producer),
        open_mask: AtomicU16::new(0),
    };
    let mntpt = std::env::args()
        .nth(1)
        .expect("Expected mountpoint argument");
    let session = fuser::Session::new_mounted(fs.into(), &mntpt, &options)
        .expect("Failed to create FUSE session.");
    println!("FUSE filesystem 'fsel_chan' mounted on {mntpt}. Press Ctrl-C to unmount.");

    // Drive the async session loop with a Tokio runtime, matching ioctl.rs style.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(session.run_async()) {
        Ok(()) => log::info!("Session ended safely"),
        Err(e) => log::info!("Session ended with error: {e:?}"),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crossbeam_channel::{Receiver, unbounded};

    // Helper to create FSelFS and a channel pair for its PollData for tests
    fn setup_test_fs_with_channel() -> (FSelFS, Sender<Notification>, Receiver<Notification>) {
        log::debug!("Setting up test FS with poll channel");
        let (tx, rx) = unbounded();
        let data = FSelData {
            bytecnt: [0; NUMFILES as usize],
        };
        // PollData with None sender.
        let poll_handler = PollData::new(None);
        let fs = FSelFS {
            data: Mutex::new(data),
            poll_handler: Mutex::new(poll_handler),
            producer: Mutex::new(ProducerData {
                next_time: UNIX_EPOCH,
                next_idx: 0,
                next_nr: 1,
            }),
            open_mask: AtomicU16::new(0),
        };
        (fs, tx, rx)
    }

    #[test]
    fn test_fs_poll_registers_handle_no_initial_event() {
        log::info!("test_fs_poll_registers_handle_no_initial_event: starting");
        let (mut fs, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        assert!(fs.init_notification_sender(tx_to_fs)); // Link FS's PollData to our test sender

        let req = RequestMeta {
            unique: 0,
            uid: 0,
            gid: 0,
            pid: 0,
        };
        let idx: u8 = 0;
        let fh = idx as u64;
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 12345;
        let events = libc::POLLIN as u32;

        fs.data.lock().unwrap().bytecnt[idx as usize] = 0;
        fs.poll_handler
            .lock()
            .unwrap()
            .mark_inode_not_ready(ino, libc::POLLIN as u32); // Ensure PollData also knows it's not ready

        let result = futures::executor::block_on(fs.poll(
            req,
            ino,
            fh,
            ph,
            events,
            FUSE_POLL_SCHEDULE_NOTIFY,
        ));
        log::debug!(
            "test_fs_poll_registers_handle_no_initial_event: poll result = {:?}",
            result
        );
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(
            result.unwrap(),
            0,
            "Should return 0 as no initial event is expected"
        );

        assert!(
            fs.poll_handler
                .lock()
                .unwrap()
                .registered_poll_handles
                .contains_key(&ph)
        );
        assert_eq!(
            fs.poll_handler
                .lock()
                .unwrap()
                .registered_poll_handles
                .get(&ph),
            Some(&(ino, events))
        );
        assert!(
            fs.poll_handler
                .lock()
                .unwrap()
                .inode_poll_handles
                .get(&ino)
                .unwrap()
                .contains(&ph)
        );

        assert!(rx_from_fs.try_recv().is_err());
    }

    #[test]
    fn test_fs_poll_registers_handle_with_initial_event() {
        log::info!("test_fs_poll_registers_handle_with_initial_event: starting");
        let (mut fs, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        assert!(fs.init_notification_sender(tx_to_fs));

        let req = RequestMeta {
            unique: 0,
            uid: 0,
            gid: 0,
            pid: 0,
        };
        let idx: u8 = 1;
        let fh = idx as u64;
        let ino = FSelData::idx_to_ino(idx);
        let ph: u64 = 54321;
        let events = libc::POLLIN as u32;

        fs.poll_handler
            .lock()
            .unwrap()
            .mark_inode_ready(ino, libc::POLLIN as u32);
        // Clear the channel from the mark_inode_ready call if any (no handle registered yet, so it shouldn't send)
        while rx_from_fs.try_recv().is_ok() {}

        let result = futures::executor::block_on(fs.poll(
            req,
            ino,
            fh,
            ph,
            events,
            FUSE_POLL_SCHEDULE_NOTIFY,
        ));
        log::debug!(
            "test_fs_poll_registers_handle_with_initial_event: poll result = {:?}",
            result
        );
        assert!(result.is_ok(), "FS poll method should succeed");
        assert_eq!(
            result.unwrap(),
            libc::POLLIN as u32,
            "Should return POLLIN as an initial event"
        );

        assert!(
            !fs.poll_handler
                .lock()
                .unwrap()
                .registered_poll_handles
                .contains_key(&ph)
        );

        match rx_from_fs.try_recv() {
            Ok(Notification::Poll((poll, _))) => {
                assert_eq!(poll.ph, ph);
                assert_eq!(poll.events, libc::POLLIN as u32);
            }
            _ => panic!("Expected an initial event on the channel"),
        }
    }

    #[test]
    fn test_producer_marks_inode_ready_triggers_event() {
        log::info!("test_producer_marks_inode_ready_triggers_event: starting");
        // For this test, we need an Arc<Mutex<FSelFS>> because producer runs in a separate thread.
        let (mut fs_instance, tx_to_fs, rx_from_fs) = setup_test_fs_with_channel();
        assert!(fs_instance.init_notification_sender(tx_to_fs));

        let idx_to_test: u8 = 2;
        let ino_to_test = FSelData::idx_to_ino(idx_to_test);
        let ph_to_test: u64 = 67890;
        let events_to_test = libc::POLLIN as u32;

        // Simulate a poll request being registered by directly accessing PollData via the Arc
        fs_instance
            .poll_handler
            .lock()
            .unwrap()
            .register_poll_handle(ph_to_test, ino_to_test, events_to_test);
        while rx_from_fs.try_recv().is_ok() {} // Clear channel

        // Manually simulate one iteration of the producer logic for a specific file
        fs_instance.data.lock().unwrap().bytecnt[idx_to_test as usize] = 1;
        fs_instance
            .poll_handler
            .lock()
            .unwrap()
            .mark_inode_ready(ino_to_test, libc::POLLIN as u32);
        log::debug!("test_producer_marks_inode_ready_triggers_event: marked inode ready");

        match rx_from_fs.try_recv() {
            Ok(Notification::Poll((poll, _))) => {
                assert_eq!(poll.ph, ph_to_test);
                assert_eq!(poll.events, libc::POLLIN as u32);
            }
            _ => {
                panic!("Producer marking inode ready should have triggered an event on the channel")
            }
        }
        assert!(
            !fs_instance
                .poll_handler
                .lock()
                .unwrap()
                .registered_poll_handles
                .contains_key(&ph_to_test)
        );
    }
}
