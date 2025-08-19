use std::io;
use std::fmt;
#[cfg(feature = "abi-7-12")]
use bytes::Bytes;
use crossbeam_channel::{Sender, Receiver, unbounded};
#[allow(unused)]
use std::{convert::TryInto, ffi::OsStr, ffi::OsString};

use crate::{
    ll::{fuse_abi::fuse_notify_code as notify_code, notify::Notification,
        fuse_ioctl::ioctl_close_backing}
};
use crate::channel::Channel;

/// The list of supported notification types
#[derive(Debug)]
pub enum NotificationKind {
    /// A poll event notification (field: ph)
    #[cfg(feature = "abi-7-11")]
    Poll(u64),
    /// An invalid entry notification (fields: parent, name)
    #[cfg(feature = "abi-7-12")]
    InvalEntry(u64, Bytes),
    /// An invalid inode notification (fields: ino, offset, len)
    #[cfg(feature = "abi-7-12")]
    InvalInode(u64, i64, i64),
    /// An inode metadata update notification (fields: ino, offset, data)
    #[cfg(feature = "abi-7-15")]
    Store(u64, u64, Bytes),
    /// An inode deletion notification (fields: parent, ino, name)
    #[cfg(feature = "abi-7-18")]
    Delete(u64, u64, Bytes),
    /// A request to close a backing ID (field: id)
    #[cfg(feature = "abi-7-40")]
    CloseBacking(u32),
    /// (Internal) Pause processing of notifications
    Disable
}

impl NotificationKind {
    /// A string the describes a Notification, mainly for logging purposes.
    pub fn label(&self) -> &'static str {
        match self {
            #[cfg(feature = "abi-7-11")]
            NotificationKind::Poll(_) => "Poll",
            #[cfg(feature = "abi-7-12")]
            NotificationKind::InvalEntry(..) => "InvalEntry",
            #[cfg(feature = "abi-7-12")]
            NotificationKind::InvalInode(..) => "InvalInode",
            #[cfg(feature = "abi-7-15")]
            NotificationKind::Store(..) => "Store",
            #[cfg(feature = "abi-7-18")]
            NotificationKind::Delete(..) => "Delete",
            #[cfg(feature = "abi-7-40")]
            NotificationKind::CloseBacking(_) => "CloseBacking",
            NotificationKind::Disable => "Disable",
        }
    }
}

/* ------ Kernel Communication ------ */

/// Callback for sending notifications to the fuse device
pub(crate) trait NotificationSender: Send + Sync + Unpin + 'static {
    fn notify(&self, code: notify_code, notification: &Notification<'_>) -> io::Result<()>;
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
    fn close_backing(&self, id: u32) -> io::Result<u32> {
        ioctl_close_backing(self.raw_fd, id)
    }
}

/// Create an error for indicating when a notification message
/// would exceed the capacity that its length descriptor field is
/// capable of encoding.
fn too_big_err(tfie: std::num::TryFromIntError) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("Data too large: {}", tfie))
}

pub(crate) struct NotificationHandler {
    notification: NotificationKind,
    channel: Channel,
}

impl NotificationHandler {
    pub(crate) fn new(
        notification: NotificationKind,
        channel: Channel
    ) -> Self {
        NotificationHandler {notification, channel}
    }
    pub(crate) fn dispatch(self) -> io::Result<()> {
        match &self.notification {
            #[cfg(feature = "abi-7-11")]
            NotificationKind::Poll(ph) => {
                let notif = Notification::new_poll(*ph);
                self.channel.notify(notify_code::FUSE_POLL, &notif)
            }
            #[cfg(feature = "abi-7-12")]
            NotificationKind::InvalEntry(parent, name) => {
                let notif = Notification::new_inval_entry(*parent, name)
                    .map_err(too_big_err)?;
                self.send_inval(notify_code::FUSE_NOTIFY_INVAL_ENTRY, &notif)
            }
            #[cfg(feature = "abi-7-12")]
            NotificationKind::InvalInode(ino, offset, len) => {
                let notif = Notification::new_inval_inode(*ino, *offset, *len);
                self.send_inval(notify_code::FUSE_NOTIFY_INVAL_INODE, &notif)
            }
            #[cfg(feature = "abi-7-15")]
            NotificationKind::Store(ino, offset, data) => {
                let notif = Notification::new_store(*ino, *offset, data)
                    .map_err(too_big_err)?;
                // Not strictly an invalidate, but the inode we're operating
                // on may have been evicted anyway, so treat is as such
                self.send_inval(notify_code::FUSE_NOTIFY_STORE, &notif)
            }
            #[cfg(feature = "abi-7-18")]
            NotificationKind::Delete(parent, ino, name) => {
                let notif = Notification::new_delete(*parent, *ino, &name)
                    .map_err(too_big_err)?;
                self.send_inval(notify_code::FUSE_NOTIFY_DELETE, &notif)
            }
            #[cfg(feature = "abi-7-40")]
            NotificationKind::CloseBacking(id) => {
                self.channel.close_backing(*id).map(|_|{})
            }
            NotificationKind::Disable => {unreachable!();}
        }
    }
    fn send_inval(&self, code: notify_code, notification: &Notification<'_>) -> io::Result<()> {
        match self.channel.notify(code, notification) {
            // ENOENT is harmless for an invalidation (the
            // kernel may have already dropped the cached
            // entry on its own anyway), so ignore it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            x => x,
        }
    }
}

/* ------ Internal Communication ------ */

/*
pub trait NotificationQueue: Clone + Sized + Send + Sync + Unpin + 'static  {
    fn queue(&self, notification: NotificationKind);
}

impl fmt::Debug for Box<dyn NotificationQueue> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<Notifier>")
    }
}

impl NotificationQueue for crossbeam_channel::Sender<NotificationKind> {
    fn queue(&self, notification: NotificationKind){
        self.send(&self, notification);
    }
}
*/
#[derive(Debug)]
pub(crate) struct Queues {
    pub sender: Sender<NotificationKind>,
    pub receiver: Receiver<NotificationKind>,
}
impl Queues {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = unbounded();
        Queues {
            sender,
            receiver,
        }
    }
}

#[derive(Debug)]
pub struct Notifier {
    /// Mechanism to queue a notification
    queue: Sender<NotificationKind>,
}

impl Notifier {
    /// Create a reply handler for a specific request identifier
    pub(crate) fn new(queue: Sender<NotificationKind>) -> Notifier {
        Notifier {
            queue,
        }
    }
}

impl Notifier {
    /// Notify poll clients of I/O readiness
    #[cfg(feature = "abi-7-11")]
    pub fn poll(&self, ph: u64) {
        self.queue.send(NotificationKind::Poll(ph)).unwrap();
    }

    /// Invalidate the kernel cache for a given directory entry
    #[cfg(feature = "abi-7-12")]
    pub fn inval_entry(&self, parent: u64, name: Bytes) {
        self.queue.send(NotificationKind::InvalEntry(parent, name)).unwrap();
    }

    /// Invalidate the kernel cache for a given inode (metadata and
    /// data in the given range)
    #[cfg(feature = "abi-7-12")]
    pub fn inval_inode(&self, ino: u64, offset: i64, len: i64) {
        self.queue.send(NotificationKind::InvalInode(ino, offset, len)).unwrap();
    }

    /// Update the kernel's cached copy of a given inode's data
    #[cfg(feature = "abi-7-15")]
    pub fn store(&self, ino: u64, offset: u64, data: Bytes) {
        self.queue.send(NotificationKind::Store(ino, offset, data)).unwrap();
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn delete(&self, parent: u64, child: u64, name: Bytes) {
        self.queue.send(NotificationKind::Delete(parent, child, name)).unwrap();
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn close_backing(&self, id: u32) {
        self.queue.send(NotificationKind::CloseBacking(id)).unwrap();
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn disable(&self,) {
        self.queue.send(NotificationKind::Disable).unwrap();
    }
}

/* ------ Poll Callback ------ */

/// A handle to a pending poll() request. Can be saved and used to notify the
/// kernel when a poll is ready.
pub struct PollHandler {
    pub handle: u64,
    queue: Sender<NotificationKind>,
}

impl PollHandler {
    pub(crate) fn new(queue: Sender<NotificationKind>, ph: u64) -> Self {
        Self {
            handle: ph,
            queue: queue,
        }
    }

    /// Notify the kernel that the associated file handle has a new event.
    pub fn notify(&self) {
        self.queue.send(NotificationKind::Poll(self.handle)).unwrap();
    }
}

impl From<PollHandler> for u64 {
    fn from(value: PollHandler) -> Self {
        value.handle
    }
}

impl std::fmt::Debug for PollHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PollHandler").field(&self.handle).finish()
    }
}

