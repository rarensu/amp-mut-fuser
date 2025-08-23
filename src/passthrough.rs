use std::fmt;
use std::io;
use std::os::fd::AsRawFd;

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
/// FUSE_DEV_IOC_BACKING_CLOSE call.  It holds a reference on the fuse channel to allow it to
/// make that call.
pub struct BackingId {
    /// The original file descriptor of the Backing File. Currently unused.
    _fd: u32,
    closer: crate::channel::Channel,
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
            let _ = ioctl_close_backing(self.closer.raw_fd, self.id);
        }
    }
}

pub trait BackingSender: Send + Sync + Unpin + 'static {
    fn open_backing(&self, fd: u32) -> io::Result<u32>;
    fn close_backing(&self, id: u32) -> io::Result<u32>;
}

impl fmt::Debug for Box<dyn BackingSender> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingSender>")
    }
}

impl BackingSender for crate::channel::Channel {
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
    /// Closure to call for requesting a backing id
    pub sender: crate::channel::Channel,
}

impl BackingHandler {
    /// Create a handler for backing id operations
    pub fn new(sender: crate::channel::Channel) -> BackingHandler {
        BackingHandler { sender }
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
            closer: self.sender.clone(),
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
