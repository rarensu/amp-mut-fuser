use std::io;

/// A handle to a pending poll() request. Can be saved and used to notify the
/// kernel when a poll is ready.
#[derive(Debug)]
pub struct Notifier {
    inner: crate::notify::NotificationHandler<crate::channel::Channel>,
}

impl Notifier {
    /// Create a notification handler from a fuse channel
    pub(crate) fn new(handler: crate::notify::NotificationHandler<crate::channel::Channel>) -> Self {
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

impl<FS: super::Filesystem> crate::Session<FS> {
    /// Returns an object that can be used to send notifications to the kernel
    pub fn notifier(&self) -> crate::trait_legacy::Notifier {
        #[cfg(not(feature = "side-channel"))]
        let this_ch = self.ch_main.clone();
        #[cfg(feature = "side-channel")]
        let this_ch = self.ch_side.clone();
        crate::trait_legacy::Notifier::new(crate::notify::NotificationHandler::new(this_ch))
    }
}
impl crate::BackgroundSession {
    /// Returns an object that can be used to send notifications to the kernel
    pub fn notifier(&self) -> crate::trait_legacy::Notifier {
        crate::trait_legacy::Notifier::new(crate::notify::NotificationHandler::new(self.sender.clone()))
    }
}
