use std::io;

#[allow(unused)]
use std::{convert::TryInto, ffi::OsStr, ffi::OsString};

use crate::{
    channel::ChannelSender,
    ll::{fuse_abi::fuse_notify_code as notify_code, notify::Notification as ll_Notification},

    // What we're sending here aren't really replies, but they
    // move in the same direction (userspace->kernel), so we can
    // reuse ReplySender for it.
    reply::ReplySender,
};

#[cfg(feature = "abi-7-11")]
/// Poll event data to be sent to the kernel
#[derive(Debug, Copy, Clone)]
pub struct Poll {
    /// Poll handle: the unique idenifier from a previous poll request
    pub ph: u64,
    /// Events flag: binary encoded information about resource availability
    pub events: u32
}

#[cfg(feature = "abi-7-12")]
/// Invalid entry notification to be sent to the kernel
#[derive(Debug, Clone)]
pub struct InvalEntry {
    /// Parent: the inode of the parent of the invalid entry
    pub parent: u64,
    /// Name: the file name of the invalid entry
    pub name: OsString
}

#[cfg(feature = "abi-7-12")]
/// Invalid inode notification to be sent to the kernel
#[derive(Debug, Copy, Clone)]
pub struct InvalInode {
    /// Inode with invalid metadata
    pub ino: u64,
    /// Start of invalid metadata
    pub offset: i64,
    /// Length of invalid metadata
    pub len: i64
}

#[cfg(feature = "abi-7-15")]
/// Store inode notification to be sent to the kernel
#[derive(Debug, Clone)]
pub struct Store {
    /// ino: the inode to be updated
    pub ino: u64,
    /// The start location of the metadata to be updated
    pub offset: u64,
    /// The new metadata
    data: Vec<u8>
}

#[cfg(feature = "abi-7-18")]
/// Deleted file notification to be sent to the kernel
#[derive(Debug, Clone)]
pub struct Delete {
    /// Parent: the inode of the parent directory that contained the deleted entry
    pub parent: u64,
    /// ino: the inode of the deleted file
    pub ino: u64,
    /// Name: the file name of the deleted entry
    pub name: OsString
}

#[derive(Debug)]
/// The list of supported notification types
pub enum Notification {
    #[cfg(feature = "abi-7-11")]
    /// A poll event notification
    Poll(Poll),
    #[cfg(feature = "abi-7-12")]
    /// An invalid entry notification
    InvalEntry(InvalEntry),
    #[cfg(feature = "abi-7-12")]
    /// An invalid inode notification
    InvalInode(InvalInode),
    #[cfg(feature = "abi-7-15")]
    /// An inode metadata update notification
    Store(Store),
    #[cfg(feature = "abi-7-18")]
    /// An inode deletion notification
    Delete(Delete),
    /// (Internal) Disable notifications for this session
    Stop
}

#[cfg(feature = "abi-7-11")]
impl From<Poll>       for Notification {fn from(notification: Poll)       -> Self{Notification::Poll(notification)}}
#[cfg(feature = "abi-7-12")]
impl From<InvalEntry> for Notification {fn from(notification: InvalEntry) -> Self{Notification::InvalEntry(notification)}}
#[cfg(feature = "abi-7-12")]
impl From<InvalInode> for Notification {fn from(notification: InvalInode) -> Self{Notification::InvalInode(notification)}}
#[cfg(feature = "abi-7-15")]
impl From<Store>      for Notification {fn from(notification: Store)      -> Self{Notification::Store(notification)}}
#[cfg(feature = "abi-7-18")]
impl From<Delete>     for Notification {fn from(notification: Delete)     -> Self{Notification::Delete(notification)}}

/// A handle by which the application can send notifications to the server
#[derive(Debug, Clone)]
pub(crate) struct Notifier(ChannelSender);

impl Notifier {
    pub(crate) fn new(cs: ChannelSender) -> Self {
        Self(cs)
    }

    pub(crate) fn notify(&self, notification: Notification) -> io::Result<()> {
        match notification {
            #[cfg(feature = "abi-7-11")]
            Notification::Poll(data) =>       self.poll(data),
            #[cfg(feature = "abi-7-12")]
            Notification::InvalEntry(data) => self.inval_entry(data),
            #[cfg(feature = "abi-7-12")]
            Notification::InvalInode(data) => self.inval_inode(data),
            #[cfg(feature = "abi-7-15")]
            Notification::Store(data) =>      self.store(data),
            #[cfg(feature = "abi-7-18")]
            Notification::Delete(data) =>     self.delete(data),
            // For completeness
            Notification::Stop => Ok(())
        }
    }

    /// Notify poll clients of I/O readiness
    #[cfg(feature = "abi-7-11")]
    pub fn poll(&self, notification: Poll) -> io::Result<()> {
        let notif = ll_Notification::new_poll(notification.ph);
        self.send(notify_code::FUSE_POLL, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry
    #[cfg(feature = "abi-7-12")]
    pub fn inval_entry(&self, notification: InvalEntry) -> io::Result<()> {
        let notif = ll_Notification::new_inval_entry(notification.parent, notification.name.as_ref()).map_err(Self::too_big_err)?;
        self.send_inval(notify_code::FUSE_NOTIFY_INVAL_ENTRY, &notif)
    }

    /// Invalidate the kernel cache for a given inode (metadata and
    /// data in the given range)
    #[cfg(feature = "abi-7-12")]
    pub fn inval_inode(&self, notification: InvalInode ) -> io::Result<()> {
        let notif = ll_Notification::new_inval_inode(notification.ino, notification.offset, notification.len);
        self.send_inval(notify_code::FUSE_NOTIFY_INVAL_INODE, &notif)
    }

    /// Update the kernel's cached copy of a given inode's data
    #[cfg(feature = "abi-7-15")]
    pub fn store(&self, notification: Store) -> io::Result<()> {
        let notif = ll_Notification::new_store(notification.ino, notification.offset, &notification.data).map_err(Self::too_big_err)?;
        // Not strictly an invalidate, but the inode we're operating
        // on may have been evicted anyway, so treat is as such
        self.send_inval(notify_code::FUSE_NOTIFY_STORE, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn delete(&self, notification: Delete) -> io::Result<()> {
        let notif = ll_Notification::new_delete(notification.parent, notification.ino, &notification.name).map_err(Self::too_big_err)?;
        self.send_inval(notify_code::FUSE_NOTIFY_DELETE, &notif)
    }

    #[allow(unused)]
    fn send_inval(&self, code: notify_code, notification: &ll_Notification<'_>) -> io::Result<()> {
        match self.send(code, notification) {
            // ENOENT is harmless for an invalidation (the
            // kernel may have already dropped the cached
            // entry on its own anyway), so ignore it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            x => x,
        }
    }

    fn send(&self, code: notify_code, notification: &ll_Notification<'_>) -> io::Result<()> {
        notification
            .with_iovec(code, |iov| self.0.send(iov))
            .map_err(Self::too_big_err)?
    }

    /// Create an error for indicating when a notification message
    /// would exceed the capacity that its length descriptor field is
    /// capable of encoding.
    fn too_big_err(tfie: std::num::TryFromIntError) -> io::Error {
        io::Error::new(io::ErrorKind::Other, format!("Data too large: {}", tfie))
    }
}
