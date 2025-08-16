use std::io;
use std::fmt;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::{Arc, Weak};
use crossbeam_channel::{Sender, Receiver, bounded};

use crate::ll::fuse_ioctl::{ioctl_open_backing, ioctl_close_backing};

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
    fd: u32,
    queuer: Box<dyn BackingQueuer>,
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
            let _ = self.queuer.close_backing(id);
            self.id = 0;
        }
    }
}
impl BackingId {
    fn close(&mut self) -> io::Result<u32> {
        match self.sender.close_backing(self.id){
            Ok(code) => {
                self.id = 0;
                Ok(code)
            }
            Err(e) => Err(e)
        }
    }
}

pub trait BackingSender: Send + Sync + Unpin + 'static  {
    fn open_backing(&self, fd: u32) -> io::Result<u32>;
    fn close_backing(&self, id: u32) -> io::Result<u32>;
}

impl fmt::Debug for Box<dyn BackingSender> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingSender>")
    }
}

impl BackingSender for crate::channel::Channel {
    fn open_backing(&self, fd: u32) -> std::io::Result<u32> {
        ioctl_open_backing(self.raw_fd, fd)
    }
    fn close_backing(&self, id: u32) -> std::io::Result<u32> {
        ioctl_close_backing(self.raw_fd, id)
    }
}

pub trait BackingQueuer: Send + Sync + Unpin + 'static  {
    fn open_backing(&self, fd: u32) -> Receiver<io::Result<BackingId>>;
    fn close_backing(&self, id: u32) -> Receiver<io::Result<u32>>;
}

impl fmt::Debug for Box<dyn BackingQueuer> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<BackingQueuer>")
    }
}

impl BackingQueuer for crate::queue::InternalChannel {
    fn open_backing(&self, fd: u32) -> Receiver<std::io::Result<BackingId>> {
        let (oss, osr) = bounded(1);
        self.open_backing_out.send((fd, oss));
        osr
    }
    fn close_backing(&self, id: u32) -> Receiver<std::io::Result<u32>> {
        let (oss, osr) = bounded(1);
        self.open_backing_out.send((id, oss));
        osr
    }
}

#[derive(Debug)]
/// `BackingHandler` needs documentation
pub struct BackingHandler {
    /// Closure to call for requesting a backing id
    pub sender: Box<dyn BackingSender>,
    /// Closure to call for queuing a request for a backing id
    pub queuer: Box<dyn BackingQueuer>,

}

impl BackingHandler {
    /// Create a reply handler for a specific request identifier
    pub fn new<S: BackingSender, Q: BackingQueuer>(sender: S, queuer: Q) -> BackingHandler {
        let sender = Box::new(sender);
        let queuer = Box::new(queuer);
        BackingHandler {
            sender,
            queuer
        }
    }
}

impl BackingHandler {
    pub fn open_backing<F: AsRawFd>(&self, file: F) -> std::io::Result<BackingId> {
        let fd = file.as_raw_fd() as u32;
        self.sender.open_backing(fd)?;
        Ok( BackingId {
            fd,
            queuer: Box::new(self.queuer.clone()) as Box<dyn BackingQueuer>,
            id,
        })
    }
    pub fn close_backing(&self, mut backing: BackingId) -> std::io::Result<u32> {
        match self.sender.close_backing(backing.id){
            Ok(code) => {
                backing.id = 0;
                Ok(code)
            }
            Err(e) => Err(e)
        }    
    }
    pub fn queue_open_backing<F: AsRawFd>(&self, file: F) -> Receiver<io::Result<BackingId>> {
        let fd = file.as_raw_fd() as u32;
        self.queuer.open_backing(fd)
    }
    pub fn queue_close_backing(&self, mut backing: BackingId) -> Receiver<std::io::Result<u32>> {
        let id = backing.id;
        backing.id = 0;
        self.queuer.close_backing(id)   
    }
}