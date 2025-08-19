use std::io;
use crate::notify::NotificationSender;
use crate::{
    ll::{fuse_abi::fuse_notify_code as notify_code, notify::Notification}
};
use crate::channel::Channel;
#[cfg(feature = "abi-7-12")]
use crate::notify::too_big_err;

#[derive(Debug)]
/// Legacy method for sending notifications. 
/// Avoid using this in Filesystem operations; may cause deadlocks.
/// Recommended to call this from `heartbeat()` or from a separate thread.
pub struct LegacyNotifier {
    /// The fuse communication channel
    channel: Channel,
}

impl LegacyNotifier {
    pub(crate) fn new(channel: Channel) -> Self {
        LegacyNotifier{channel}
    }
    /// Notify poll clients of I/O readiness
    #[cfg(feature = "abi-7-11")]
    pub fn poll(&self, ph: u64) -> io::Result<()> {
        let notif = Notification::new_poll(ph);
        self.channel.notify(notify_code::FUSE_POLL, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry
    #[cfg(feature = "abi-7-12")]
    pub fn inval_entry(&self, parent: u64, name: &[u8]) -> io::Result<()> {
        let notif = Notification::new_inval_entry(parent, name)
            .map_err(too_big_err)?;
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
        let notif = Notification::new_store(ino, offset, data)
            .map_err(too_big_err)?;
        // Not strictly an invalidate, but the inode we're operating
        // on may have been evicted anyway, so treat is as such
        self.send_inval(notify_code::FUSE_NOTIFY_STORE, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-18")]
    pub fn delete(&self, parent: u64, child: u64, name: &[u8]) -> io::Result<()> {
        let notif = Notification::new_delete(parent, child, name)
            .map_err(too_big_err)?;
        self.send_inval(notify_code::FUSE_NOTIFY_DELETE, &notif)
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    #[cfg(feature = "abi-7-40")]
    pub fn close_backing(&self, id: u32) -> io::Result<()> {
                self.channel.close_backing(id).map(|_|{})
    }
    #[cfg(feature = "abi-7-12")]
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