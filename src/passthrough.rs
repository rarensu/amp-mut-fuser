use std::io;
use std::fmt;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::{Arc, Weak};
use crossbeam_channel::{Sender, Receiver, bounded};

use crate::ll::fuse_ioctl::{ioctl_open_backing, ioctl_close_backing};
use crate::notify::NotificationKind;
use crate::channel::Channel;

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
    /// The original file descriptor of the Backing File
    fd: u32,
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
    pub fn close(&mut self) {
        if self.id > 0 {
            let _ = self.queue.send(NotificationKind::CloseBacking(self.id)).unwrap();
            self.id = 0;
        }
    }
    pub fn is_open(&self) -> bool {
        self.id > 0
    }
}

pub trait BackingOpener: Send + Sync + Unpin + 'static  {
    fn open_backing(&self, fd: u32) -> io::Result<u32>;
}

impl fmt::Debug for Box<dyn BackingOpener> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingSender>")
    }
}

impl BackingOpener for crate::channel::Channel {
    fn open_backing(&self, fd: u32) -> std::io::Result<u32> {
        ioctl_open_backing(self.raw_fd, fd)
    }
}

#[derive(Debug)]
/// `BackingHandler` needs documentation
pub struct BackingHandler {
    /// Mechanism to request a backing id
    pub opener: Box<dyn BackingOpener>,
    /// Mechanism to queue a request to close a backing id
    pub queue: Sender<NotificationKind>,
}

impl BackingHandler {
    /// Create a reply handler for a specific request identifier
    pub fn new<S: BackingOpener>(opener: S, queue: Sender<NotificationKind>) -> BackingHandler {
        let opener = Box::new(opener);
        BackingHandler {
            opener,
            queue
        }
    }
}

impl BackingHandler {
    pub fn open_backing<F: AsRawFd>(&self, file: F) -> std::io::Result<BackingId> {
        let fd = file.as_raw_fd() as u32;
        let id = self.opener.open_backing(fd)?;
        Ok( BackingId {
            fd,
            queue: self.queue.clone(),
            id,
        })
    }
    pub fn close_backing(&self, mut backing: BackingId) {
        let id = backing.id;
        backing.id = 0;
        self.queue.send(NotificationKind::CloseBacking(id)).unwrap()   
    }
}