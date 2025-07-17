use std::io;

#[allow(unused)]
use std::{convert::TryInto, ffi::OsStr, ffi::OsString};

use crossbeam_channel::Sender;

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
    pub data: Vec<u8>
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
    Poll((Poll, Option<Sender<io::Result<()>>>)),
    #[cfg(feature = "abi-7-12")]
    /// An invalid entry notification
    InvalEntry((InvalEntry, Option<Sender<io::Result<()>>>)),
    #[cfg(feature = "abi-7-12")]
    /// An invalid inode notification
    InvalInode((InvalInode, Option<Sender<io::Result<()>>>)),
    #[cfg(feature = "abi-7-15")]
    /// An inode metadata update notification
    Store((Store, Option<Sender<io::Result<()>>>)),
    #[cfg(feature = "abi-7-18")]
    /// An inode deletion notification
    Delete((Delete, Option<Sender<io::Result<()>>>)),
    /// A request to register a file descriptor and receive a backing ID
    OpenBacking((u32, Option<Sender<io::Result<u32>>>)),
    #[cfg(feature = "abi-7-18")]
    /// A request to close a backing ID
    CloseBacking((u32, Option<Sender<io::Result<u32>>>)),
    /// (Internal) Disable notifications for this session
    Stop
}

#[cfg(feature = "abi-7-11")]
impl From<Poll>       for Notification {fn from(notification: Poll)       -> Self{Notification::Poll((notification, None))}}
#[cfg(feature = "abi-7-12")]
impl From<InvalEntry> for Notification {fn from(notification: InvalEntry) -> Self{Notification::InvalEntry((notification, None))}}
#[cfg(feature = "abi-7-12")]
impl From<InvalInode> for Notification {fn from(notification: InvalInode) -> Self{Notification::InvalInode((notification, None))}}
#[cfg(feature = "abi-7-15")]
impl From<Store>      for Notification {fn from(notification: Store)      -> Self{Notification::Store((notification, None))}}
#[cfg(feature = "abi-7-18")]
impl From<Delete>     for Notification {fn from(notification: Delete)     -> Self{Notification::Delete((notification, None))}}

/// A handle by which the application can send notifications to the server
#[derive(Debug, Clone)]
pub(crate) struct Notifier(ChannelSender);

impl Notifier {
    pub(crate) fn new(cs: ChannelSender) -> Self {
        Self(cs)
    }

    pub(crate) fn notify(&self, notification: Notification) -> Result<(),()> {
        match notification {
            #[cfg(feature = "abi-7-11")]
            Notification::Poll((data, sender)) => {
                let res = self.poll(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            #[cfg(feature = "abi-7-12")]
            Notification::InvalEntry((data, sender)) => {
                let res = self.inval_entry(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            #[cfg(feature = "abi-7-12")]
            Notification::InvalInode((data, sender)) => {
                let res = self.inval_inode(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            #[cfg(feature = "abi-7-15")]
            Notification::Store((data, sender)) => {
                let res = self.store(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            #[cfg(feature = "abi-7-18")]
            Notification::Delete((data, sender)) => {
                let res = self.delete(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            Notification::OpenBacking((data, sender)) => {
                let res = self.open_backing(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
            Notification::CloseBacking((data, sender)) => {
                let res = self.close_backing(data);
                if let Some(sender) = sender {
                    if let Err(_) = sender.send(res) {
                        Err(())
                    } else { Ok(()) } 
                } else { Ok(()) }
            },
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

    /// Registers a file descriptor for passthrough and returns a backing ID.
    pub fn open_backing(&self, fd: u32) -> io::Result<u32> {
        self.0.open_backing(fd)
    }

    /// Deregisters a backing ID.
    pub fn close_backing(&self, backing_id: u32) -> io::Result<u32> {
        self.0.close_backing(backing_id)
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::channel::ChannelSender;
    use std::fs::File;
    use std::sync::Arc;

    struct MockChannelSender {
        open_backing_fn: Box<dyn Fn(u32) -> io::Result<u32> + Send + Sync>,
        close_backing_fn: Box<dyn Fn(u32) -> io::Result<u32> + Send + Sync>,
    }

    impl crate::reply::ReplySender for MockChannelSender {
        fn send(&self, _bufs: &[io::IoSlice<'_>]) -> io::Result<()> {
            unimplemented!()
        }
    }

    impl ChannelSender {
        fn mock(
            open_backing_fn: Box<dyn Fn(u32) -> io::Result<u32> + Send + Sync>,
            close_backing_fn: Box<dyn Fn(u32) -> io::Result<u32> + Send + Sync>,
        ) -> Self {
            // This is a bit of a hack to allow mocking the ChannelSender.
            // We create a dummy file and then use `mem::transmute` to create a `ChannelSender`
            // with our mock functions. This is safe because `ChannelSender` is just a wrapper
            // around an `Arc<File>` and we are not actually using the file.
            let file = Arc::new(File::open("/dev/null").unwrap());
            let sender = ChannelSender(file);
            let mock_sender = Arc::new(MockChannelSender {
                open_backing_fn,
                close_backing_fn,
            });
            let ptr = Arc::into_raw(mock_sender);
            unsafe {
                let mut sender = sender;
                let p: *mut Self = &mut sender;
                *p = std::mem::transmute_copy(&ptr);
                sender
            }
        }
    }

    impl Drop for MockChannelSender {
        fn drop(&mut self) {
            // Do nothing
        }
    }

    impl Notifier {
        fn mock_open_backing(&self, fd: u32) -> io::Result<u32> {
            let mock = self.0.0.clone();
            let mock: Arc<MockChannelSender> = unsafe { std::mem::transmute_copy(&mock) };
            (mock.open_backing_fn)(fd)
        }

        fn mock_close_backing(&self, backing_id: u32) -> io::Result<u32> {
            let mock = self.0.0.clone();
            let mock: Arc<MockChannelSender> = unsafe { std::mem::transmute_copy(&mock) };
            (mock.close_backing_fn)(backing_id)
        }
    }

    #[test]
    fn test_open_backing() {
        let sender = ChannelSender::mock(
            Box::new(|fd| {
                assert_eq!(fd, 123);
                Ok(456)
            }),
            Box::new(|_| panic!("should not be called")),
        );
        let notifier = Notifier::new(sender);
        assert_eq!(notifier.mock_open_backing(123).unwrap(), 456);
    }

    #[test]
    fn test_close_backing() {
        let sender = ChannelSender::mock(
            Box::new(|_| panic!("should not be called")),
            Box::new(|backing_id| {
                assert_eq!(backing_id, 789);
                Ok(0)
            }),
        );
        let notifier = Notifier::new(sender);
        assert_eq!(notifier.mock_close_backing(789).unwrap(), 0);
    }
}
