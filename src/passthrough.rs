use std::io;
use std::fmt;
use std::os::fd::{AsRawFd};
use crossbeam_channel::{Sender};

use crate::notify::NotificationKind;
use crate::channel::Channel;
use crate::ll::fuse_ioctl::{ioctl_close_backing, ioctl_open_backing};

/// A reference to a previously opened fd intended to be used for passthrough
///
/// You can create these via `ReplyOpen::open_backing()` and send them via
/// `ReplyOpen::opened_passthrough()`.
///
/// When working with backing IDs you need to ensure that they live "long enough".  A good practice
/// is to create them in the Filesystem::open() impl, store them in the struct of your Filesystem
/// impl, then drop them in the Filesystem::release() impl.  Dropping them immediately after
/// sending them in the Filesystem::open() impl can lead to the kernel returning EIO when userspace
/// attempts to access the file.
///
/// This is implemented as a safe wrapper around the backing_id field of the fuse_backing_map
/// struct used by the ioctls involved in fd passthrough.  It is created by performing a
/// FUSE_DEV_IOC_BACKING_OPEN ioctl on an fd and has a Drop trait impl which makes a matching
/// FUSE_DEV_IOC_BACKING_CLOSE call.  It holds a reference to a backing sender to allow it to
/// make that call.
pub struct BackingId {
    /// The original file descriptor of the Backing File. Unused.
    _fd: u32,
    /// The notification queue used to alert the kernel after the Backing is no longer needed.
    queue: Sender<NotificationKind>,
    /// The backing_id field passed to and from the kernel
    pub id: u32,
}
impl fmt::Debug for BackingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "BackingId({:?})", &self.id)
    }
}
impl Drop for BackingId {
    fn drop(&mut self) {
        if self.id > 0 {
            let _ = self.queue.send(NotificationKind::CloseBacking(self.id)).unwrap();
            self.id = 0;
        }
    }
}
impl BackingId {
    /// Send a message to the kernel that this Backing id is no longer needed.
    pub fn close(&mut self) {
        if self.id > 0 {
            let _ = self.queue.send(NotificationKind::CloseBacking(self.id)).unwrap();
            self.id = 0;
        }
    }
    /// Whether the `close()` message has been sent for this Backing id.
    pub fn is_open(&self) -> bool {
        self.id > 0
    }
}

pub trait BackingSender: Send + Sync + Unpin + 'static {
    fn open_backing(&self, fd: u32) -> io::Result<u32>;
    fn close_backing(&self, id: u32) -> io::Result<u32>;
}

impl BackingSender for Channel {
    fn open_backing(&self, fd: u32) -> io::Result<u32> {
        ioctl_open_backing(self.raw_fd, fd)
    }
    fn close_backing(&self, id: u32) -> io::Result<u32> {
        ioctl_close_backing(self.raw_fd, id)
    }
}

#[derive(Debug)]
/// `BackingHandler` allows the filesystem to open (and close) `BackingId`.
pub struct BackingHandler {
    /// Mechanism to open and close backing id
    sender: Channel,
    /// Mechanism to queue close requests as notifications
    queue: Sender<NotificationKind>,
}

impl BackingHandler {
    /// Create a reply handler for a specific request identifier
    pub(crate) fn new(sender: Channel, queue: Sender<NotificationKind>) -> BackingHandler {
        BackingHandler {
            sender,
            queue
        }
    }
}

impl BackingHandler {
    /// This method builds a `BackingId` for the provided File.
    /// You may use this during `open` or `opendir` or `heartbeat`.
    /// # Errors
    /// Reports errors with the underlying fuse connection.
    pub fn open_backing<F: AsRawFd>(&self, file: F) -> io::Result<BackingId> {
        let fd = file.as_raw_fd() as u32;
        let id = self.sender.open_backing(fd)?;
        Ok(BackingId {
            _fd: fd,
            queue: self.queue.clone(),
            id,
        })
    }
    /// This method destroys a `BackingId`.
    /// You may use this during `open` or `opendir` or `heartbeat`.
    /// # Errors
    /// Reports errors with the underlying fuse connection.
    #[allow(unused)] // for completeness
    pub fn close_backing(&self, mut backing: BackingId) -> io::Result<u32> {
        let id = backing.id;
        backing.id = 0;
        self.sender.close_backing(id)
    }
}
