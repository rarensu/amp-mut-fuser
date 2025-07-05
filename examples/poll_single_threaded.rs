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

#[cfg(test)]
#[cfg(feature = "abi-7-11")]
mod test_single_threaded_session {
    use super::*;
    // Removed ReplyPoll as it's no longer used by MockFS::poll
    // Removed ReplyAttr, ReplyData, ReplyEmpty, ReplyEntry, ReplyOpen as they are unused by MockFS
    use crate::{Errno, Filesystem, KernelConfig, RequestMeta}; // Removed Open, PollData
    // use std::ffi::OsStr; // Removed as unused
    use std::sync::atomic::{AtomicBool, Ordering};
    // use std::time::Duration; // Duration is no longer used by current tests.
    // use crossbeam_channel::unbounded; // Removed as unused by current tests
    use crate::ll::fuse_abi; // Direct import for fuse_abi types
    use std::os::unix::io::AsRawFd;
    use std::io::{Read, Write}; // Added for the module
    use std::os::fd::OwnedFd;
    use std::fs::File;

    // Shared state for MockFS, allowing it to be cloned for Session and test assertion/manipulation.
    #[derive(Default, Debug)]
    struct MockFsState {
        init_called: AtomicBool,
        lookup_called: AtomicBool,
        poll_event_sender: Mutex<Option<Sender<(u64, u32)>>>,
        last_lookup_name: Mutex<Option<String>>,
    }

    // A mock Filesystem for testing
    #[derive(Debug, Clone)] // MockFS is now Clone
    struct MockFS {
        state: Arc<MockFsState>,
    }

    impl Default for MockFS {
        fn default() -> Self {
            MockFS {
                state: Arc::new(MockFsState::default()),
            }
        }
    }

    impl Filesystem for MockFS {
        fn init(&mut self, _req: RequestMeta, _config: KernelConfig) -> Result<KernelConfig, Errno> {
            self.state.init_called.store(true, Ordering::SeqCst);
            Ok(KernelConfig::new(0, 0))
        }

        fn lookup(&mut self, _req: RequestMeta, _parent: u64, name: std::ffi::OsString) -> Result<crate::Entry, Errno> {
            self.state.lookup_called.store(true, Ordering::SeqCst);
            *self.state.last_lookup_name.lock().unwrap() = Some(name.to_string_lossy().to_string());
            Err(Errno::ENOENT)
        }

        #[cfg(feature = "abi-7-11")]
        fn init_poll_sender(&mut self, sender: Sender<(u64, u32)>) -> Result<(), Errno> {
            *self.state.poll_event_sender.lock().unwrap() = Some(sender);
            Ok(())
        }

        #[cfg(feature = "abi-7-11")]
        fn poll(&mut self, _req: RequestMeta, _ino: u64, _fh: u64, _ph: u64, _events: u32, _flags: u32) -> Result<u32, Errno> {
            Ok(0)
        }
    }

    // Helper to create a Session with MockFS
    fn create_test_session() -> (Session<MockFS>, File, MockFS) { // Returns MockFS (clone)
        let (kernel_fd_owned, session_fd_owned) = nix::sys::socket::socketpair(
            nix::sys::socket::AddressFamily::Unix,
            nix::sys::socket::SockType::SeqPacket,
            None,
            nix::sys::socket::SockFlag::empty(),
        ).expect("socketpair failed for test");

        let mock_fs_instance = MockFS::default(); // Create one instance

        let kernel_fd_file = File::from(kernel_fd_owned);
        let session_fd_file = OwnedFd::from(session_fd_owned);

        // Session gets a clone, test gets a clone. Both share the Arc<MockFsState>.
        let session = Session::from_fd(mock_fs_instance.clone(), session_fd_file, SessionACL::All);
        (session, kernel_fd_file, mock_fs_instance) // Return the original mock_fs_instance (or another clone)
    }


    #[test]
    fn test_single_threaded_processes_init_request() {
        let (mut session, kernel_fd_file, mock_fs_handle) = create_test_session();

        // Manually call init_poll_sender on the session's filesystem instance.
        let poll_sender = session.get_poll_sender();
        session.filesystem.init_poll_sender(poll_sender).unwrap();

        let session_thread = std::thread::spawn(move || {
            session.run_single_threaded().expect("run_single_threaded failed");
            session // Return session
        });

        // Construct FUSE_INIT request
        // Construct fuse_init_in. For abi-7-11 (and not higher ones like 7-12 or 7-23),
        // it only has major, minor, max_readahead, flags.
        let init_in = fuse_abi::fuse_init_in {
            major: fuse_abi::FUSE_KERNEL_VERSION,
            minor: fuse_abi::FUSE_KERNEL_MINOR_VERSION,
            max_readahead: 0,
            flags: 0,
            // If testing with abi-7-12, .conn would be needed.
            // If testing with abi-7-23, .flags2 would be needed.
            // Since this test module is cfg(feature = "abi-7-11"), we assume this base version.
        };

        let header = fuse_abi::fuse_in_header {
            len: (std::mem::size_of::<fuse_abi::fuse_in_header>() + std::mem::size_of_val(&init_in)) as u32,
            opcode: fuse_abi::fuse_opcode::FUSE_INIT as u32,
            unique: 1,
            nodeid: crate::FUSE_ROOT_ID,
            uid: 0,
            gid: 0,
            pid: 0,
            padding: 0,
        };

        let mut req_buf = Vec::new();
        req_buf.extend_from_slice(unsafe {
            std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of_val(&header))
        });
        req_buf.extend_from_slice(unsafe {
            std::slice::from_raw_parts(&init_in as *const _ as *const u8, std::mem::size_of_val(&init_in))
        });

        let mut kfile_write = kernel_fd_file.try_clone().expect("Failed to clone kernel_fd for writing");
        let mut kfile_read = kernel_fd_file;

        kfile_write.write_all(&req_buf).expect("Failed to write FUSE_INIT to kernel_fd");

        // Session thread should process INIT and reply. Read the reply.
        let mut reply_buf = vec![0; 1024];
        let fd_read = kfile_read.as_raw_fd();
        let mut poll_fds_kernel_read = [libc::pollfd { fd: fd_read, events: libc::POLLIN, revents: 0 }];

        // Poll with a timeout to wait for the reply
        let poll_res_read = unsafe { libc::poll(poll_fds_kernel_read.as_mut_ptr(), 1, 1000) }; // 1 second timeout
        assert!(poll_res_read > 0, "Kernel FD did not become readable for INIT reply (poll timeout or error)");

        let reply_len = kfile_read.read(&mut reply_buf).expect("Failed to read FUSE_INIT reply");
        assert!(reply_len >= std::mem::size_of::<fuse_abi::fuse_out_header>(), "INIT reply too short");

        // Basic check on reply header (e.g., error code should be 0)
        let reply_header_ptr = reply_buf.as_ptr() as *const fuse_abi::fuse_out_header;
        let reply_header = unsafe { &*reply_header_ptr };
        assert_eq!(reply_header.unique, header.unique, "INIT reply unique ID mismatch");
        assert_eq!(reply_header.error, 0, "INIT reply error non-zero");

        // Check if Filesystem::init was called
        assert!(mock_fs_handle.state.init_called.load(Ordering::SeqCst), "Filesystem init should have been called");

        // Send DESTROY to cleanly shut down the session thread
        let destroy_header = fuse_abi::fuse_in_header {
            len: std::mem::size_of::<fuse_abi::fuse_in_header>() as u32,
            opcode: fuse_abi::fuse_opcode::FUSE_DESTROY as u32,
            unique: 2, // Different unique ID from INIT
            nodeid: crate::FUSE_ROOT_ID,
            uid: 0, gid: 0, pid: 0, padding: 0,
        };
        let destroy_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(&destroy_header as *const _ as *const u8, std::mem::size_of_val(&destroy_header))
        };
        // kfile_write is the handle for writing to the kernel side
        kfile_write.write_all(destroy_bytes).expect("Failed to write FUSE_DESTROY");

        // Close kernel FDs to signal end to the session thread
        drop(kfile_write);
        drop(kfile_read); // kfile_read is the handle for reading from the kernel side (original kernel_fd_file)

        let _session_after_run = session_thread.join().expect("Session thread panicked");
    }

    #[test]
    fn test_single_threaded_processes_poll_event() {
        let (mut session, mut kernel_fd_read, mock_fs_handle) = create_test_session();
        let mut kfile_write = kernel_fd_read.try_clone().expect("Failed to clone kernel_fd for writing");

        let poll_sender_from_session = session.get_poll_sender();
        session.filesystem.init_poll_sender(poll_sender_from_session.clone()).unwrap();

        // Session must be initialized for poll notifications to work
        // std::io::{Read, Write} is already imported at the top of the test module for test_single_threaded_processes_init_request
        // so no new import needed here.
        let init_in = fuse_abi::fuse_init_in {
            major: fuse_abi::FUSE_KERNEL_VERSION,
            minor: fuse_abi::FUSE_KERNEL_MINOR_VERSION,
            max_readahead: 0,
            flags: 0,
            // Assuming base abi-7-11 for this test module
        };
        let header = fuse_abi::fuse_in_header {
            len: (std::mem::size_of::<fuse_abi::fuse_in_header>() + std::mem::size_of_val(&init_in)) as u32,
            opcode: fuse_abi::fuse_opcode::FUSE_INIT as u32,
            unique: 1, nodeid: crate::FUSE_ROOT_ID, uid: 0, gid: 0, pid: 0, padding: 0,
        };
        let mut req_buf = Vec::new();
        req_buf.extend_from_slice(unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of_val(&header)) });
        req_buf.extend_from_slice(unsafe { std::slice::from_raw_parts(&init_in as *const _ as *const u8, std::mem::size_of_val(&init_in)) });

        kfile_write.write_all(&req_buf).expect("Failed to write FUSE_INIT for poll test");

        let mut reply_buf_init = vec![0;1024];
        let fd_read_init = kernel_fd_read.as_raw_fd();
        let mut poll_fds_init_reply = [libc::pollfd { fd: fd_read_init, events: libc::POLLIN, revents: 0 }];
        let poll_res_init = unsafe { libc::poll(poll_fds_init_reply.as_mut_ptr(), 1, 1000) };
        assert!(poll_res_init > 0, "Kernel FD did not become readable for INIT reply in poll test");
        kernel_fd_read.read(&mut reply_buf_init).expect("Failed to read FUSE_INIT reply in poll test");
        // INIT processed, session should be initialized.

        let session_thread = std::thread::spawn(move || {
            session.run_single_threaded().expect("run_single_threaded failed");
        });

        // Send a poll event from the "filesystem"
        let test_kh = 12345u64;
        let test_events = libc::POLLIN as u32;
        mock_fs_handle.state.poll_event_sender.lock().unwrap().as_ref().unwrap().send((test_kh, test_events))
            .expect("Failed to send poll event to session");

        // How to verify Notifier::poll was called?
        // The `run_single_threaded` loop calls `self.notifier().poll(kh)`.
        // This sends a message through `self.ch.sender()`.
        // So, we should expect a FUSE_POLL message on `kernel_fd_file`.

        // Expected FUSE_POLL message structure:
        // fuse_out_header: len, error=0, unique=0 (notifications have unique=0)
        // fuse_notify_poll_wakeup_out: kh (u64)
        // Total length: sizeof(fuse_out_header) + sizeof(fuse_notify_poll_wakeup_out)
        // sizeof(fuse_out_header) = 16 (len,error,unique)
        // sizeof(fuse_notify_poll_wakeup_out) = 8 (kh)
        // Total = 24

        let mut read_buf = vec![0u8; 100];
        use std::io::Read;
        let mut kfile = kernel_fd_file;

        // Set a timeout for the read, as the test might hang if no poll notification is sent.
        let fd = kfile.as_raw_fd();
        let mut poll_fds_kernel = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
        let poll_res = unsafe { libc::poll(poll_fds_kernel.as_mut_ptr(), 1, 1000) }; // 1s timeout
        assert!(poll_res > 0, "Kernel FD did not become readable for poll notification");

        let bytes_read = kfile.read(&mut read_buf).expect("Failed to read from kernel_fd");
        assert_eq!(bytes_read, 16 + 8, "Unexpected size of FUSE_POLL notification");

        let header_ptr = read_buf.as_ptr() as *const fuse_abi::fuse_out_header;
        let header = unsafe { &*header_ptr };
        assert_eq!(header.len as usize, 16 + 8);
        assert_eq!(header.error, 0); // Error should be 0 for successful notification
        assert_eq!(header.unique, 0); // Notifications have unique ID 0

        // Check opcode (implicitly part of how Notifier sends, not in fuse_out_header directly for notifies)
        // The actual "opcode" for notification is part of the initial send structure,
        // not present in fuse_out_header in this way. What IS sent is:
        // struct fuse_out_header: len, error=FUSE_NOTIFY_POLL (this is how it's distinguished), unique
        // struct fuse_notify_poll_wakeup_out: kh
        // So header.error should be FUSE_POLL (-4) as per fuse_abi.rs for poll notifications.
        // Let's re-check ll::notify::Notification::new_poll - it sets error to the code.
        assert_eq!(header.error, fuse_abi::fuse_notify_code::FUSE_POLL as i32, "Notification code mismatch");


        let poll_wakeup_out_ptr = unsafe { read_buf.as_ptr().add(std::mem::size_of::<fuse_abi::fuse_out_header>()) }
            as *const fuse_abi::fuse_notify_poll_wakeup_out; // Used fuse_abi alias
        let poll_wakeup_out = unsafe { &*poll_wakeup_out_ptr };
        assert_eq!(poll_wakeup_out.kh, test_kh, "Poll handle (kh) in notification mismatch");

        // Cleanly shut down the session thread by closing the kernel_fd
        drop(kfile);
        session_thread.join().expect("Session thread panicked");
    }
}