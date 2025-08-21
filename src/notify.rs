use std::io;
use std::fmt;
#[allow(unused)]
use std::{convert::TryInto, ffi::OsStr};

use crate::{
    ll::{fuse_abi::fuse_notify_code as notify_code, notify::Notification}
};
#[cfg(feature = "abi-7-40")]
use crate::ll::fuse_ioctl::ioctl_close_backing;
use crate::channel::Channel;

/* ------ General Notification Handling ------ */

/* ------ Kernel Communication ------ */

/// Callback for sending notifications to the fuse device
pub(crate) trait NotificationSender: Send + Sync + Unpin + 'static {
    fn notify(&self, code: notify_code, notification: &Notification<'_>) -> io::Result<()>;
    #[cfg(feature = "abi-7-40")]
    fn close_backing(&self, id: u32) -> io::Result<u32>;
}

impl fmt::Debug for Box<dyn NotificationSender> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<NotificationSender>")
    }
}

// Legacy callback for sending notifications to the fuse device
impl NotificationSender for crate::channel::Channel {
    fn notify(&self, code: notify_code, notification: &Notification<'_>) -> io::Result<()> {
        notification
            .with_iovec(code, |iov| self.send(iov))
            .map_err(too_big_err)?
    }
    #[cfg(feature = "abi-7-40")]
    fn close_backing(&self, id: u32) -> io::Result<u32> {
        ioctl_close_backing(self.raw_fd, id)
    }
}

/// Create an error for indicating when a notification message
/// would exceed the capacity that its length descriptor field is
/// capable of encoding.
pub(crate) fn too_big_err(tfie: std::num::TryFromIntError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("Data too large: {}", tfie))
}

#[derive(Debug)]
pub struct NotificationHandler {
    channel: Channel,
}
impl NotificationHandler {
    /// Create a reply handler for a specific request identifier
    pub(crate) fn new(channel: Channel) -> NotificationHandler {
        NotificationHandler {
            channel,
        }
    }
}

impl NotificationHandler {
    /// Notify poll clients of I/O readiness
    #[cfg(feature = "abi-7-11")]
    pub fn poll(&self, ph: u64) -> io::Result<()> {
        let notif = Notification::new_poll(ph);
        self.channel.notify(notify_code::FUSE_POLL, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry
    #[cfg(feature = "abi-7-12")]
    pub fn inval_entry(&self, parent: u64, name: &[u8]) -> io::Result<()> {
        let notif = Notification::new_inval_entry(parent, name).map_err(too_big_err)?;
        self.send_inval(notify_code::FUSE_NOTIFY_INVAL_ENTRY, &notif)
    }

    /// Invalidate the kernel cache for a given inode (metadata and
    /// data in the given range)
    #[cfg(feature = "abi-7-12")]
    pub fn inval_inode(&self, ino: u64, offset: i64, len: i64) -> io::Result<()> {
        let notif = Notification::new_inval_inode(ino, offset, len);
        self.send_inval(notify_code::FUSE_NOTIFY_INVAL_INODE, &notif)
    }

    /// Update the kernel's cached copy of a given inode's data
    #[cfg(feature = "abi-7-15")]
    pub fn store(&self, ino: u64, offset: u64, data: &[u8]) -> io::Result<()> {
        let notif = Notification::new_store(ino, offset, data).map_err(too_big_err)?;
        // Not strictly an invalidate, but the inode we're operating
        // on may have been evicted anyway, so treat is as such
        self.send_inval(notify_code::FUSE_NOTIFY_STORE, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn delete(&self, parent: u64, child: u64, name: &[u8]) -> io::Result<()> {
        let notif = Notification::new_delete(parent, child, name).map_err(too_big_err)?;
        self.send_inval(notify_code::FUSE_NOTIFY_DELETE, &notif)
    }

    #[allow(unused)]
    fn send_inval(&self, code: notify_code, notification: &Notification<'_>) -> io::Result<()> {
        match self.channel
        .notify(code, notification) {
            // ENOENT is harmless for an invalidation (the
            // kernel may have already dropped the cached
            // entry on its own anyway), so ignore it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            x => x,
        }
    }
}

/* ------ Poll Callback ------ */

/// A handle to a pending poll() request. Can be saved and used to notify the
/// kernel when a poll is ready.
#[derive(Debug)]
pub struct PollHandler {
    handle: u64,
    sender: NotificationHandler,
}

impl PollHandler {
    pub(crate) fn new(handler: NotificationHandler, ph: u64) -> Self {
        Self {
            handle: ph,
            sender: handler,
        }
    }

    /// Notify the kernel that the associated file handle is ready to be polled.
    pub fn notify(self) -> io::Result<()> {
        self.sender.poll(self.handle)
    }
}

impl From<PollHandler> for u64 {
    fn from(value: PollHandler) -> Self {
        value.handle
    }
}
/*
impl std::fmt::Debug for PollHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PollHandler").field(&self.handle).finish()
    }
}
*/