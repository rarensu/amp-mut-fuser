use std::fmt;
use std::io;
use std::os::fd::AsRawFd;

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
/// FUSE_DEV_IOC_BACKING_CLOSE call.  It holds a weak reference on the fuse channel to allow it to
/// make that call (if the channel hasn't already been closed).
#[derive(Debug)]
pub struct BackingId {
    closer: Box<dyn BackingCloser>,
    /// The backing_id field passed to and from the kernel
    pub id: u32,
}

impl Drop for BackingId {
    fn drop(&mut self) {
        if self.id > 0 {
            self.closer.close_backing(self.id);
        }
    }
}

/// Generic tool for managing backings
pub trait BackingCloser: Send + Sync + Unpin + 'static {
    /// Close a backing id, without waiting for the result.
    /// Used in `BackingId`'s drop implementation.
    fn close_backing(&self, id: u32);
}
/// Generic tool for managing backings
pub trait BackingCloserFactory {
    /// Create a boxed callback for closing a backing id.
    /// Used to construct `BackingId`
    fn new_closer(&self) -> Box<dyn BackingCloser>;
}
/// Generic tool for managing backings
pub trait BackingSender: Send + Sync + Unpin + 'static {
    /// Directly open a backing id.
    fn open_backing(&self, fd: u32) -> io::Result<u32>;
    /// Directly close a backing id.
    fn close_backing(&self, id: u32) -> io::Result<u32>;
}

// Primary implementation for BackingCloser
impl BackingCloser for Channel {
    fn close_backing(&self, id: u32) {
        let _ = ioctl_close_backing(self.raw_fd, id);
    }
}
// Primary implementation for BackingCloserFactory
impl BackingCloserFactory for Channel {
    fn new_closer(&self) -> Box<dyn BackingCloser> {
        Box::new(self.clone())
    }
}
// Primary implementation for BackingSender
impl BackingSender for Channel {
    fn open_backing(&self, fd: u32) -> io::Result<u32> {
        ioctl_open_backing(self.raw_fd, fd)
    }
    fn close_backing(&self, id: u32) -> io::Result<u32> {
        ioctl_close_backing(self.raw_fd, id)
    }
}

impl fmt::Debug for Box<dyn BackingCloser> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingCloser>")
    }
}
impl fmt::Debug for Box<dyn BackingSender> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingSender>")
    }
}
impl fmt::Debug for Box<dyn BackingCloserFactory> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingCloserFactory>")
    }
}

#[derive(Debug)]
/// `BackingHandler` allows the filesystem to create (and destroy) `BackingId`.
pub struct BackingHandler<S: BackingSender, C: BackingCloserFactory> {
    /// Tool for requesting a backing id
    sender: S,
    /// Makes closures to call for closing a backing id
    factory: C,
}

impl<S: BackingSender, C: BackingCloserFactory> BackingHandler<S, C> {
    /// Create a handler for backing id operations
    pub fn new(sender: S, factory: C) -> BackingHandler<S, C> {
        BackingHandler { sender, factory }
    }

    /// This method creates a `BackingId` for the provided File.
    /// You may use this during `open` or `opendir`.
    /// # Errors
    /// Reports errors with the underlying fuse connection.
    pub fn open_backing<F: AsRawFd>(&self, file: F) -> io::Result<BackingId> {
        let fd = file.as_raw_fd() as u32;
        let id = self.sender.open_backing(fd)?;
        Ok(BackingId {
            closer: self.factory.new_closer(),
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
