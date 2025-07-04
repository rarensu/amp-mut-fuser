//! Filesystem session
//!
//! A session runs a filesystem implementation while it is being mounted to a specific mount
//! point. A session begins by mounting the filesystem and ends by unmounting it. While the
//! filesystem is mounted, the session loop receives, dispatches and replies to kernel requests
//! for filesystem operations under its mount point.
///
/// A session can be run synchronously in the current thread using `run()`, spawned into a
/// background thread using `spawn()`, or run in a single-threaded mode that handles
/// both FUSE requests and poll notifications using `run_single_threaded()`.

use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use nix::unistd::geteuid;
use std::fmt;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd, AsRawFd}; // Added AsRawFd
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::{io, ops::DerefMut};

use crate::ll::fuse_abi as abi;
use crate::request::Request;
use crate::Filesystem;
use crate::MountOption;
use crate::{channel::Channel, mnt::Mount};
#[cfg(feature = "abi-7-11")]
use crate::{channel::ChannelSender, notify::Notifier};
#[cfg(feature = "abi-7-11")]
use crossbeam_channel::{Sender, Receiver};

/// The max size of write requests from the kernel. The absolute minimum is 4k,
/// FUSE recommends at least 128k, max 16M. The FUSE default is 16M on macOS
/// and 128k on other systems.
pub const MAX_WRITE_SIZE: usize = 16 * 1024 * 1024;

/// Size of the buffer for reading a request from the kernel. Since the kernel may send
/// up to MAX_WRITE_SIZE bytes in a write request, we use that value plus some extra space.
const BUFFER_SIZE: usize = MAX_WRITE_SIZE + 4096;

#[derive(Default, Debug, Eq, PartialEq)]
/// How requests should be filtered based on the calling UID.
pub enum SessionACL {
    /// Allow requests from any user. Corresponds to the `allow_other` mount option.
    All,
    /// Allow requests from root. Corresponds to the `allow_root` mount option.
    RootAndOwner,
    /// Allow requests from the owning UID. This is FUSE's default mode of operation.
    #[default]
    Owner,
}

/// The session data structure
#[derive(Debug)]
pub struct Session<FS: Filesystem> {
    /// Filesystem operation implementations
    pub(crate) filesystem: FS,
    /// Communication channel to the kernel driver
    pub(crate) ch: Channel,
    /// Handle to the mount.  Dropping this unmounts.
    mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
    /// Whether to restrict access to owner, root + owner, or unrestricted
    /// Used to implement allow_root and auto_unmount
    pub(crate) allowed: SessionACL,
    /// User that launched the fuser process
    pub(crate) session_owner: u32,
    /// FUSE protocol major version
    pub(crate) proto_major: u32,
    /// FUSE protocol minor version
    pub(crate) proto_minor: u32,
    /// True if the filesystem is initialized (init operation done)
    pub(crate) initialized: bool,
    /// True if the filesystem was destroyed (destroy operation done)
    pub(crate) destroyed: bool,
    #[cfg(feature = "abi-7-11")]
    /// Whether this session currently has poll support
    pub(crate) poll_enabled: bool,
    /// Sender for poll events to the filesystem. It will be cloned and passed to Filesystem.
    #[cfg(feature = "abi-7-11")]
    pub(crate) poll_event_sender: Sender<(u64, u32)>,
    /// Receiver for poll events from the filesystem.
    #[cfg(feature = "abi-7-11")]
    pub(crate) poll_event_receiver: Receiver<(u64, u32)>,
}

impl<FS: Filesystem> AsFd for Session<FS> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ch.as_fd()
    }
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

impl<FS: Filesystem> Session<FS> {
    /// Create a new session by mounting the given filesystem to the given mountpoint
    pub fn new<P: AsRef<Path>>(
        filesystem: FS,
        mountpoint: P,
        options: &[MountOption],
    ) -> io::Result<Session<FS>> {
        let mountpoint = mountpoint.as_ref();
        info!("Mounting {}", mountpoint.display());
        // If AutoUnmount is requested, but not AllowRoot or AllowOther we enforce the ACL
        // ourself and implicitly set AllowOther because fusermount needs allow_root or allow_other
        // to handle the auto_unmount option
        let (file, mount) = if options.contains(&MountOption::AutoUnmount)
            && !(options.contains(&MountOption::AllowRoot)
                || options.contains(&MountOption::AllowOther))
        {
            warn!("Given auto_unmount without allow_root or allow_other; adding allow_other, with userspace permission handling");
            let mut modified_options = options.to_vec();
            modified_options.push(MountOption::AllowOther);
            Mount::new(mountpoint, &modified_options)?
        } else {
            Mount::new(mountpoint, options)?
        };
        // Create the channel for fuse messages
        let ch = Channel::new(file);
        let allowed = if options.contains(&MountOption::AllowRoot) {
            SessionACL::RootAndOwner
        } else if options.contains(&MountOption::AllowOther) {
            SessionACL::All
        } else {
            SessionACL::Owner
        };
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (pxs, pxr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        let mut filesystem = filesystem;
        // Pass the sender to the filesystem.
        #[cfg(feature = "abi-7-11")]
        let poll_enabled = match filesystem.init_poll_sender(pxs.clone()) {
            Err(e) => {
                // Log an error if the filesystem explicitely states it does not support polling.
                // ENOSYS is the default from the trait if not implemented.
                if e != crate::Errno::ENOSYS {
                    warn!("Filesystem failed to initialize poll sender: {:?}. Channel-based polling might not work as expected.", e);
                } else {
                    info!("Filesystem does not implement init_poll_sender (ENOSYS). Assuming no channel-based poll support or uses legacy poll.");
                }
                // Proceeding even if init_poll_sender fails, as FS might use legacy poll or no poll.
                // The poll_event_loop will still be spawned if abi-7-11 is enabled,
                // but it might not receive anything if FS doesn't use the sender.
                false
            },
            Ok(()) => true
        };
        let new_session = Session {
            filesystem,
            ch,
            mount: Arc::new(Mutex::new(Some((mountpoint.to_owned(), mount)))),
            allowed,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            poll_enabled,
            #[cfg(feature = "abi-7-11")]
            poll_event_sender: pxs,
            #[cfg(feature = "abi-7-11")]
            poll_event_receiver: pxr,
        };
        Ok(new_session)
    }

    /// Wrap an existing /dev/fuse file descriptor. This doesn't mount the
    /// filesystem anywhere; that must be done separately.
    pub fn from_fd(filesystem: FS, fd: OwnedFd, acl: SessionACL) -> Self {
        // Create the channel for fuse messages
        let ch = Channel::new(Arc::new(fd.into()));
        #[cfg(feature = "abi-7-11")]
        // Create the channel for poll events.
        let (pxs, pxr) = crossbeam_channel::unbounded();
        #[cfg(feature = "abi-7-11")]
        let mut filesystem = filesystem;
        #[cfg(feature = "abi-7-11")]
        let poll_enabled = match filesystem.init_poll_sender(pxs.clone()) {
            Err(e) => {
                // Log an error if the filesystem explicitely states it does not support polling.
                // ENOSYS is the default from the trait if not implemented.
                if e != crate::Errno::ENOSYS {
                    warn!("Filesystem failed to initialize poll sender: {:?}. Channel-based polling might not work as expected.", e);
                } else {
                    info!("Filesystem does not implement init_poll_sender (ENOSYS). Assuming no channel-based poll support or uses legacy poll.");
                }
                // Proceeding even if init_poll_sender fails, as FS might use legacy poll or no poll.
                // The poll_event_loop will still be spawned if abi-7-11 is enabled,
                // but it might not receive anything if FS doesn't use the sender.
                false
            },
            Ok(()) => true
        };
        Session {
            filesystem,
            ch,
            mount: Arc::new(Mutex::new(None)),
            allowed: acl,
            session_owner: geteuid().as_raw(),
            proto_major: 0,
            proto_minor: 0,
            initialized: false,
            destroyed: false,
            #[cfg(feature = "abi-7-11")]
            poll_enabled,
            #[cfg(feature = "abi-7-11")]
            poll_event_sender: pxs,
            #[cfg(feature = "abi-7-11")]
            poll_event_receiver: pxr,
        }
    }

    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub fn run(&mut self) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = aligned_sub_buf(
            buffer.deref_mut(),
            std::mem::align_of::<abi::fuse_in_header>(),
        );
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            match self.ch.receive(buf) {
                Ok(size) => match Request::new(self.ch.sender(), &buf[..size]) {
                    // Dispatch request
                    Some(req) => req.dispatch(self),
                    // Quit loop on illegal request
                    None => break,
                },
                Err(err) => match err.raw_os_error() {
                    // Operation interrupted. Accordingly to FUSE, this is safe to retry
                    Some(ENOENT) => continue,
                    // Interrupted system call, retry
                    Some(EINTR) => continue,
                    // Explicitly try again
                    Some(EAGAIN) => continue,
                    // Filesystem was unmounted, quit the loop
                    Some(ENODEV) => break,
                    // Unhandled error
                    _ => return Err(err),
                },
            }
        }
        Ok(())
    }

    /// Unmount the filesystem
    pub fn unmount(&mut self) {
        drop(std::mem::take(&mut *self.mount.lock().unwrap()));
    }

    /// Returns a thread-safe object that can be used to unmount the Filesystem
    pub fn unmount_callable(&mut self) -> SessionUnmounter {
        SessionUnmounter {
            mount: self.mount.clone(),
        }
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> Notifier {
        Notifier::new(self.ch.sender())
    }

    /// Returns an object that can be used to send poll event notifications
    #[cfg(feature = "abi-7-11")]
    pub fn get_poll_sender(&self) -> Sender<(u64, u32)> {
        self.poll_event_sender.clone()
    }

    /// Run the session loop in a single thread, processing both FUSE requests and poll events.
    ///
    /// This method provides an alternative to `run()` (blocking multi-threaded via `spawn()`)
    /// for applications that prefer or require a single-threaded operational model.
    /// The loop operates as follows:
    /// 1. It first checks for pending poll events received from the `Filesystem` implementation
    ///    (via the channel initialized by `Filesystem::init_poll_sender`). These are processed
    ///    non-blockingly using `try_recv()`. If an event is found, a `FUSE_NOTIFY_POLL`
    ///    is sent to the kernel.
    /// 2. It then checks for incoming FUSE requests from the kernel on the FUSE device
    ///    descriptor. This check is performed non-blockingly using `libc::poll()` with a
    ///    timeout of zero. If a request is ready, it's read and dispatched to the
    ///    appropriate `Filesystem` method.
    /// 3. If neither poll events nor FUSE requests are immediately available, the loop
    ///    pauses for a very short duration (1 millisecond) using `std::thread::sleep()`
    ///    to prevent busy-waiting and yield CPU time.
    ///
    /// This loop continues until the FUSE session is unmounted (e.g., `ENODEV` is received)
    /// or a fatal error occurs.
    ///
    /// ## Usage
    ///
    /// This method is suitable for `Filesystem` implementations that manage their own
    /// asynchronous tasks (if any) and can signal I/O readiness for polling via the
    /// `poll_event_sender` channel provided by `Session::get_poll_sender()` and
    /// typically passed to the `Filesystem` during its `init_poll_sender` call.
    ///
    /// For example, a `Filesystem` might have a `PollData` struct that, upon being
    /// notified of data readiness for a polled file handle (`kh`), sends `(kh, event_mask)`
    /// through this channel. `run_single_threaded` will then pick this up and notify the kernel.
    ///
    /// ```rust,no_run
    /// # use fuser::{Filesystem, Session, MountOption, SessionACL, KernelConfig, ReplyEntry, RequestMeta, Errno};
    /// # use std::ffi::OsStr;
    /// # use crossbeam_channel::Sender;
    /// # struct MyFS;
    /// # impl Filesystem for MyFS {
    /// #     fn init(&mut self, _req: RequestMeta, _config: KernelConfig) -> Result<KernelConfig, Errno> { Ok(KernelConfig::default()) }
    /// #     fn lookup(&mut self, _req: RequestMeta, _parent: u64, _name: &OsStr, reply: ReplyEntry) { reply.error(Errno::ENOENT); }
    /// #     #[cfg(feature = "abi-7-11")]
    /// #     fn init_poll_sender(&mut self, _sender: Sender<(u64, u32)>) -> Result<(), Errno> { Ok(()) }
    /// # }
    /// # fn main() -> std::io::Result<()> {
    /// # let mountpoint = "/tmp/fuse_mount";
    /// # std::fs::create_dir_all(mountpoint).ok();
    /// let fs = MyFS;
    /// let options = vec![MountOption::FSName("myfs".to_string())];
    /// let mut session = Session::new(fs, mountpoint, &options)?;
    /// // Initialize the poll sender in your Filesystem implementation if it uses polling.
    /// // session.filesystem.init_poll_sender(session.get_poll_sender()).unwrap();
    ///
    /// // Run the single-threaded session loop.
    /// // This will block until the filesystem is unmounted or an error occurs.
    /// session.run_single_threaded()?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "abi-7-11")]
    pub fn run_single_threaded(&mut self) -> io::Result<()> {
        // Buffer for receiving requests from the kernel
        let mut buffer = vec![0; BUFFER_SIZE];
        let buf = aligned_sub_buf(
            buffer.deref_mut(),
            std::mem::align_of::<abi::fuse_in_header>(),
        );

        let fuse_fd = self.ch.as_fd().as_raw_fd();
        let mut pollfds = [libc::pollfd {
            fd: fuse_fd,
            events: libc::POLLIN,
            revents: 0,
        }];

        info!("Running FUSE session in single-threaded mode");

        loop {
            // 1. Check for and process pending poll events (non-blocking)
            if self.poll_enabled {
                match self.poll_event_receiver.try_recv() {
                    Ok((kh, events)) => {
                        // Note: Original plan mentioned calling self.notifier().poll(kh).
                        // The existing poll loop in BackgroundSession directly calls notifier.poll(ph).
                        // We'll replicate that behavior.
                        // The `events` variable from `poll_event_receiver` is not directly used by `Notifier::poll`,
                        // as `Notifier::poll` only takes `kh`. This matches existing behavior.
                        debug!("Processing poll event for kh: {}, events: {:x}", kh, events);
                        if let Err(e) = self.notifier().poll(kh) {
                            error!("Failed to send poll notification for kh {}: {}", kh, e);
                            // Decide if error is fatal. ENODEV might mean unmounted.
                            if e.raw_os_error() == Some(libc::ENODEV) {
                                warn!("FUSE device not available for poll notification, likely unmounted. Exiting.");
                                break;
                            }
                        }
                        // Continue immediately to prioritize processing all available internal events
                        continue;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => {
                        // No poll events pending, proceed to check FUSE FD
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        // Filesystem's poll event sender side dropped.
                        // This is not necessarily a fatal error for the session itself,
                        // as FUSE requests can still be processed.
                        warn!("Poll event channel disconnected by sender. No more poll events will be processed.");
                        // We could choose to break or continue; for now, let FUSE requests continue.
                    }
                }
            }

            // 2. Check for incoming FUSE requests (non-blocking)
            let poll_timeout_ms = 0; // Non-blocking poll
            let ret = unsafe { libc::poll(pollfds.as_mut_ptr(), 1, poll_timeout_ms) };

            match ret {
                -1 => {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() == Some(EINTR) {
                        debug!("libc::poll interrupted (EINTR), retrying.");
                        continue;
                    }
                    error!("Error polling FUSE FD: {}", err);
                    return Err(err); // Fatal error
                }
                0 => {
                    // Timeout with no events on FUSE FD.
                    // And no poll notifications were pending (checked above).
                    // Sleep briefly to yield CPU.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                _ => { // ret > 0, FUSE FD has events
                    if (pollfds[0].revents & libc::POLLIN) != 0 {
                        // FUSE FD is ready to read.
                        match self.ch.receive(buf) {
                            Ok(size) => {
                                if size == 0 {
                                    // Read of 0 bytes on FUSE FD typically means it was closed (unmounted)
                                    info!("FUSE channel read 0 bytes, session ending.");
                                    break;
                                }
                                match Request::new(self.ch.sender(), &buf[..size]) {
                                    Some(req) => req.dispatch(self),
                                    None => {
                                        warn!("Failed to parse FUSE request, session ending.");
                                        break; // Illegal request, quit loop
                                    }
                                }
                            }
                            Err(err) => match err.raw_os_error() {
                                Some(ENOENT) => {
                                    debug!("FUSE channel receive ENOENT, retrying.");
                                    continue;
                                }
                                Some(EINTR) => {
                                    debug!("FUSE channel receive EINTR, retrying.");
                                    continue;
                                }
                                Some(EAGAIN) => {
                                    debug!("FUSE channel receive EAGAIN, retrying.");
                                    continue;
                                }
                                Some(ENODEV) => {
                                    info!("FUSE device not available (ENODEV), session ending.");
                                    break; // Filesystem was unmounted
                                }
                                _ => {
                                    error!("Error receiving FUSE request: {}", err);
                                    return Err(err); // Unhandled error
                                }
                            },
                        }
                    } else if (pollfds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                        info!("FUSE FD error or hangup detected (revents: {:#x}). Session ending.", pollfds[0].revents);
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
/// A thread-safe object that can be used to unmount a Filesystem
pub struct SessionUnmounter {
    mount: Arc<Mutex<Option<(PathBuf, Mount)>>>,
}

impl SessionUnmounter {
    /// Unmount the filesystem
    pub fn unmount(&mut self) -> io::Result<()> {
        drop(std::mem::take(&mut *self.mount.lock().unwrap()));
        Ok(())
    }
}

fn aligned_sub_buf(buf: &mut [u8], alignment: usize) -> &mut [u8] {
    let off = alignment - (buf.as_ptr() as usize) % alignment;
    if off == alignment {
        buf
    } else {
        &mut buf[off..]
    }
}

impl<FS: 'static + Filesystem + Send> Session<FS> {
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}

impl<FS: Filesystem> Drop for Session<FS> {
    fn drop(&mut self) {
        if !self.destroyed {
            self.filesystem.destroy();
            self.destroyed = true;
        }

        if let Some((mountpoint, _mount)) = std::mem::take(&mut *self.mount.lock().unwrap()) {
            info!("unmounting session at {}", mountpoint.display());
        }
    }
}

/// The background session data structure
pub struct BackgroundSession {
    /// Thread guard of the main session loop
    pub main_loop_guard: JoinHandle<io::Result<()>>,
    /// Thread guard for the poll event notification loop
    #[cfg(feature = "abi-7-11")]
    pub poll_event_loop_guard: Option<JoinHandle<()>>,
    /// Object for creating Notifiers for client use
    #[cfg(feature = "abi-7-11")]
    sender: ChannelSender,
    /// Ensures the filesystem is unmounted when the session ends
    _mount: Option<Mount>,
}

impl BackgroundSession {
    /// Create a new background session for the given session by running its
    /// session loop in a background thread. If the returned handle is dropped,
    /// the filesystem is unmounted and the given session ends.
    pub fn new<FS: Filesystem + Send + 'static>(mut se: Session<FS>) -> io::Result<BackgroundSession> {
        #[cfg(feature = "abi-7-11")]
        let poll_event_receiver_for_loop = se.poll_event_receiver.clone(); // Receiver is copied into the poll_event_loop_guard's thread
        #[cfg(feature = "abi-7-11")]
        let notifier_for_poll_loop = Notifier::new(se.ch.sender().clone()); // Notifier needs its own sender clone
        #[cfg(feature = "abi-7-11")]
        let extra_sender_clone = se.ch.sender().clone();

        let mount = std::mem::take(&mut *se.mount.lock().unwrap()).map(|(_, mount)| mount);

        #[cfg(not(feature = "abi-7-11"))]
        // The main session (se) is moved into this thread.
        let main_loop_guard = thread::spawn(move || {
            se.run()
        });
        #[cfg(feature = "abi-7-11")]
        let main_loop_guard = thread::spawn(move || {
            se.run_single_threaded()
        });

        #[cfg(feature = "abi-7-11")]
        let poll_event_loop_guard = {
            // Note: se.ch.sender() is used for the notifier, se.poll_event_receiver for this loop.
            info!("Spawning poll event notification thread.");
            Some(thread::spawn(move || {
                loop {
                    match poll_event_receiver_for_loop.recv() { // uses clone of receiver
                        Ok((ph, _events)) => {
                            if let Err(e) = notifier_for_poll_loop.poll(ph) {
                                log::error!("Failed to send poll notification for ph {}: {}", ph, e);
                                if e.kind() == io::ErrorKind::BrokenPipe || e.raw_os_error() == Some(libc::ENODEV) {
                                    warn!("Poll notification channel broken, exiting poll event loop.");
                                    break;
                                }
                            } else {
                                debug!("Sent poll notification for ph {}", ph);
                            }
                        }
                        Err(e) => {
                            info!("Poll event channel disconnected: {}. Exiting poll event loop.", e);
                            break;
                        }
                    }
                }
            }))
        };
        // No explicit poll_event_loop_guard = None for the else case, as the field itself is conditional in BackgroundSession

        Ok(BackgroundSession {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            poll_event_loop_guard,
            #[cfg(feature = "abi-7-11")]
            sender: extra_sender_clone, // This sender is for the Notifier method on BackgroundSession
            _mount: mount,
        })
    }
    /// Unmount the filesystem and join the background thread.
    pub fn join(self) {
        let Self {
            main_loop_guard,
            #[cfg(feature = "abi-7-11")]
            poll_event_loop_guard,
            #[cfg(feature = "abi-7-11")] // sender is conditionally present
            sender: _,
            _mount,
        } = self;
        drop(_mount); // Unmounts the filesystem
        main_loop_guard.join().unwrap().unwrap();
        #[cfg(feature = "abi-7-11")]
        if let Some(guard) = poll_event_loop_guard {
            guard.join().unwrap();
        }
    }

    /// Returns an object that can be used to send notifications to the kernel
    #[cfg(feature = "abi-7-11")]
    pub fn notifier(&self) -> Notifier {
        Notifier::new(self.sender.clone())
    }
}

// replace with #[derive(Debug)] if Debug ever gets implemented for
// thread_scoped::JoinGuard
impl fmt::Debug for BackgroundSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let mut builder = f.debug_struct("BackgroundSession");
        builder.field("main_loop_guard", &self.main_loop_guard);
        #[cfg(feature = "abi-7-11")]
        {
            builder.field("poll_event_loop_guard", &self.poll_event_loop_guard);
            builder.field("sender", &self.sender);
        }
        builder.field("_mount", &self._mount);
        builder.finish()
    }
}
