use std::io;

/// A handle to a pending poll() request. Can be saved and used to notify the
/// kernel when a poll is ready.
pub struct Notifier {
    inner: crate::notify::NotificationHandler<crate::channel::Channel>,
}

impl Notifier {
    /// Create a notification handler from a fuse channel
    pub fn new(handler: crate::notify::NotificationHandler<crate::channel::Channel>) -> Self {
        Self { inner: handler }
    }

    /// Notify poll clients of I/O readiness
    pub fn poll(&self, ph: u64) -> io::Result<()> {
        self.inner.poll(ph)
    }

    /// Invalidate the kernel cache for a given directory entry
    pub fn inval_entry(&self, parent: u64, name: &[u8]) -> io::Result<()> {
        self.inner.inval_entry(parent, name)
    }

    /// Invalidate the kernel cache for a given inode (metadata and
    /// data in the given range)
    pub fn inval_inode(&self, ino: u64, offset: i64, len: i64) -> io::Result<()> {
        self.inner.inval_inode(ino, offset, len)
    }

    /// Update the kernel's cached copy of a given inode's data
    pub fn store(&self, ino: u64, offset: u64, data: &[u8]) -> io::Result<()> {
        self.inner.store(ino, offset, data)
    }

    /// Invalidate the kernel cache for a given directory entry and inform
    /// inotify watchers of a file deletion.
    pub fn delete(&self, parent: u64, child: u64, name: &[u8]) -> io::Result<()> {
        self.inner.delete(parent, child, name)
    }
}
