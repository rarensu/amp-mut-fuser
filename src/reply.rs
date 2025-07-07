//! Filesystem operation reply
//!
//! A reply handler object is created to guarantee that each fuse request receives a reponse exactly once.
//! Either the request logic will call the one of the reply handler's self-destructive methods, 
//! or, if the reply handler goes out of scope before that happens, the drop trait will send an error response. 

use crate::ll::{
    self,
    reply::{DirEntList, DirEntOffset},
    reply::DirEntry as ll_DirEntry,
    INodeNo,
};
#[cfg(feature = "abi-7-21")]
use crate::ll::reply::{DirEntPlusList, DirEntryPlus};
#[cfg(feature = "abi-7-21")]
use crate::ll::Generation;
#[cfg(feature = "abi-7-40")]
use crate::{consts::FOPEN_PASSTHROUGH, passthrough::BackingId};
use log::{error, warn};
// use std::ffi::OsString; // Keep for tests only if needed
use std::fmt;
use std::io::IoSlice;
#[cfg(feature = "abi-7-40")]
use std::os::fd::BorrowedFd;
use std::time::Duration;
use zerocopy::IntoBytes;
#[cfg(target_os = "macos")]
use std::time::SystemTime;

use crate::{
    ByteBox, FileAttr, FileType, KernelConfig, OsBox,
    DirEntriesList, DirEntryContainer, // For dir
    DirEntryPlusList, DirEntryPlusContainer, DirEntryPlusData, // For dirplus
};

/// Generic reply callback to send data
pub(crate) trait ReplySender: Send + Sync + Unpin + 'static {
    /// Send data.
    fn send(&self, data: &[IoSlice<'_>]) -> std::io::Result<()>;
    /// Open a backing file
    #[cfg(feature = "abi-7-40")]
    fn open_backing(&self, fd: BorrowedFd<'_>) -> std::io::Result<BackingId>;
}

impl fmt::Debug for Box<dyn ReplySender> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<ReplySender>")
    }
}

/// ReplyHander is a struct which holds the unique identifiers needed to reply
/// to a specific request. Traits are implemented on the struct so that ownership
/// of the struct determines whether the identifiers have ever been used. 
/// This guarantees that a reply is send at most once per request.
#[derive(Debug)]
pub(crate) struct ReplyHandler {
    /// Unique id of the request to reply to
    unique: ll::RequestId,
    /// Closure to call for sending the reply
    sender: Option<Box<dyn ReplySender>>,
}

impl ReplyHandler {
    /// Create a reply handler for a specific request identifier
    pub(crate) fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyHandler {
        let sender = Box::new(sender);
        ReplyHandler {
            unique: ll::RequestId(unique),
            sender: Some(sender),
        }
    }

    /// Reply to a request with a formatted reponse. Can be called
    /// more than once (the `&mut self`` argument does not consume `self`)
    /// Avoid using this variant unless you know what you are doing!
    fn send_ll_mut(&mut self, response: &ll::Response<'_>) {
        assert!(self.sender.is_some());
        let sender = self.sender.take().unwrap();
        let res = response.with_iovec(self.unique, |iov| sender.send(iov));
        if let Err(err) = res {
            error!("Failed to send FUSE reply: {}", err);
        }
    }
    /// Reply to a request with a formatted reponse. May be called
    /// only once (the `mut self`` argument consumes `self`).
    /// Use this variant for general replies. 
    fn send_ll(mut self, response: &ll::Response<'_>) {
        self.send_ll_mut(response)
    }

}

/// Drop is implemented on ReplyHandler so that if the program logic fails 
/// (for example, due to an interrupt or a panic),
/// a reply will be sent when the Reply Handler falls out of scope.
impl Drop for ReplyHandler {
    fn drop(&mut self) {
        if self.sender.is_some() {
            warn!(
                "Reply not sent for operation {}, replying with I/O error",
                self.unique.0
            );
            self.send_ll_mut(&ll::Response::new_error(ll::Errno::EIO));
        }
    }
}

//
// Structs for managing response data
// 

#[derive(Debug)]
/// File attribute response data
pub struct Attr {
    /// Describes a file
    pub attr: FileAttr,
    /// Time to live for the attribute
    pub ttl: Duration
}

#[derive(Debug, Clone)]
/// File entry response data
pub struct Entry {
    /// Describes a file
    pub attr: FileAttr,
    /// Time to live for the entry
    pub ttl: Duration,
    /// The generation of the entry
    pub generation: u64
}

#[derive(Debug, Clone)] //TODO #[derive(Copy)]
/// Open file handle response data
pub struct Open {
    /// File handle for the opened file
    pub fh: u64, 
    /// Flags for the opened file
    pub flags: u32
}


#[cfg(target_os = "macos")]
#[derive(Debug)]
/// Xtimes response data
pub struct XTimes {
    /// Backup time
    pub bkuptime: SystemTime,
    /// Creation time
    pub crtime: SystemTime
}

#[derive(Copy, Clone, Debug)]
/// Statfs response data
pub struct Statfs {
    /// Total blocks (in units of frsize)
    pub blocks: u64,
    /// Free blocks
    pub bfree: u64,
    /// Free blocks for unprivileged users
    pub bavail: u64,
    /// Total inodes
    pub files: u64,
    /// Free inodes
    pub ffree: u64,
    /// Filesystem block size
    pub bsize: u32,
    /// Maximum filename length
    pub namelen: u32,
    /// Fundamental file system block size
    pub frsize: u32
}

#[derive(Debug, Clone)]
/// Directory listing response data.
/// This structure holds the data for a single directory entry.
/// The `'name_lt` lifetime parameter is associated with the `name` field if it's a borrowed `OsStr`.
pub struct DirEntryData<'name_lt> {
    /// file inode number
    pub ino: u64,
    /// entry number in directory
    pub offset: i64,
    /// kind of file
    pub kind: FileType,
    /// name of file
    pub name: OsBox<'name_lt>,
}

#[derive(Copy, Clone, Debug)]
/// File lock response data
pub struct Lock {
    /// start of locked byte range
    pub start: u64,
    /// end of locked byte range
    pub end: u64,
    // NOTE: lock field is defined as u32 in fuse_kernel.h in libfuse. However, it is treated as signed
    // TODO enum {F_RDLCK, F_WRLCK, F_UNLCK}
    /// kind of lock (read and/or write) 
    pub typ: i32,
    /// PID of process blocking our lock
    pub pid: u32, 
}

/// `Xattr` represents the response for extended attribute operations (`getxattr`, `listxattr`).
/// It can either indicate the size of the attribute data or provide the data itself
/// using `ByteBox` for flexible ownership.
#[derive(Debug)]
pub enum Xattr<'a> {
    /// Indicates the size of the extended attribute data. Used when the caller
    /// provides a zero-sized buffer to query the required buffer size.
    Size(u32),
    /// Contains the extended attribute data. `ByteBox` allows this data to be
    /// returned via a zero-copy borrow or an owned allocation.
    Data(ByteBox<'a>),
}

#[cfg(feature = "abi-7-11")]
#[derive(Debug)]
/// File io control reponse data
pub struct Ioctl {
    /// Result of the ioctl operation
    pub result: i32,
    /// Data to be returned with the ioctl operation
    pub data: Vec<u8>
}

//
// Methods to reply to a request for each kind of data
//

impl ReplyHandler {

    /// Reply to a general request with Ok
    pub fn ok(self) {
        self.send_ll(&ll::Response::new_empty());
    }

    /// Reply to a general request with an error code
    pub fn error(self, err: ll::Errno) {
        self.send_ll(&ll::Response::new_error(err));
    }

    /// Reply to a general request with data
    pub fn data(self, data: &[u8]) {
        self.send_ll(&ll::Response::new_slice(data));
    }

    // Reply to an init request with available features
    pub fn config(self, capabilities: u64, config: KernelConfig) {
        let flags = capabilities & config.requested; // use requested features and reported as capable

        let init = ll::fuse_abi::fuse_init_out {
            major: ll::fuse_abi::FUSE_KERNEL_VERSION,
            minor: ll::fuse_abi::FUSE_KERNEL_MINOR_VERSION,
            max_readahead: config.max_readahead,
            #[cfg(not(feature = "abi-7-36"))]
            flags: flags as u32,
            #[cfg(feature = "abi-7-36")]
            flags: (flags | ll::fuse_abi::consts::FUSE_INIT_EXT) as u32,
            #[cfg(not(feature = "abi-7-13"))]
            unused: 0,
            #[cfg(feature = "abi-7-13")]
            max_background: config.max_background,
            #[cfg(feature = "abi-7-13")]
            congestion_threshold: config.congestion_threshold(),
            max_write: config.max_write,
            #[cfg(feature = "abi-7-23")]
            time_gran: config.time_gran.as_nanos() as u32,
            #[cfg(all(feature = "abi-7-23", not(feature = "abi-7-28")))]
            reserved: [0; 9],
            #[cfg(feature = "abi-7-28")]
            max_pages: config.max_pages(),
            #[cfg(feature = "abi-7-28")]
            unused2: 0,
            #[cfg(all(feature = "abi-7-28", not(feature = "abi-7-36")))]
            reserved: [0; 8],
            #[cfg(feature = "abi-7-36")]
            flags2: (flags >> 32) as u32,
            #[cfg(all(feature = "abi-7-36", not(feature = "abi-7-40")))]
            reserved: [0; 7],
            #[cfg(feature = "abi-7-40")]
            max_stack_depth: config.max_stack_depth,
            #[cfg(feature = "abi-7-40")]
            reserved: [0; 6],
        };
        self.send_ll(&ll::Response::new_data(init.as_bytes()));
    }

    /// Reply to a request with a file entry
    pub fn entry(self, entry: Entry) {
        self.send_ll(&ll::Response::new_entry(
            ll::INodeNo(entry.attr.ino),
            ll::Generation(entry.generation),
            &entry.attr.into(),
            entry.ttl,
            entry.ttl,
        ));
    }

    /// Reply to a request with a file attributes
    pub fn attr(self, attr: Attr) {
        self.send_ll(&ll::Response::new_attr(&attr.ttl, &attr.attr.into()));
    }

    #[cfg(target_os = "macos")]
    /// Reply to a request with xtimes attributes
    pub fn xtimes(self, xtimes: XTimes) {
        self.send_ll(&ll::Response::new_xtimes(xtimes.bkuptime, xtimes.crtime))
    }

    /// Reply to a request with a newly opened file handle
    pub fn opened(self, open: Open) {
        #[cfg(feature = "abi-7-40")]
        assert_eq!(open.flags & FOPEN_PASSTHROUGH, 0);
        self.send_ll(&ll::Response::new_open(ll::FileHandle(open.fh), open.flags, 0))
    }

    /// Reply to a request with the number of bytes written
    pub fn written(self, size: u32) {
        self.send_ll(&ll::Response::new_write(size))
    }

    /// Reply to a statfs request 
    pub fn statfs(
        self,
        statfs: Statfs
    ) {
        self.send_ll(&ll::Response::new_statfs(
            statfs.blocks, statfs.bfree, statfs.bavail, statfs.files, statfs.ffree, statfs.bsize, statfs.namelen, statfs.frsize,
        ))
    }

    /// Reply to a request with a newle created file entry and its newly open file handle
    pub fn created(self, entry: Entry, open: Open) {
        #[cfg(feature = "abi-7-40")]
        assert_eq!(open.flags & FOPEN_PASSTHROUGH, 0);
        self.send_ll(&ll::Response::new_create(
            &entry.ttl,
            &entry.attr.into(),
            ll::Generation(entry.generation),
            ll::FileHandle(open.fh),
            open.flags,
            0,
        ))
    }

    /// Reply to a request with a file lock
    pub fn locked(self, lock: Lock) {
        self.send_ll(&ll::Response::new_lock(&ll::Lock{
            range: (lock.start, lock.end),
            typ: lock.typ,
            pid: lock.pid,
        }))
    }

    /// Reply to a request with a bmap
    pub fn bmap(self, block: u64) {
        self.send_ll(&ll::Response::new_bmap(block))
    }

    #[cfg(feature = "abi-7-11")]
    /// Reply to a request with an ioctl
    pub fn ioctl(self, ioctl: Ioctl) {
        self.send_ll(&ll::Response::new_ioctl(ioctl.result, &[IoSlice::new(ioctl.data.as_ref())]));
    }

    #[cfg(feature = "abi-7-11")]
    /// Reply to a request with a poll result
    pub fn poll(self, revents: u32) {
        self.send_ll(&ll::Response::new_poll(revents))
    }

    /// Reply to a request with a filled directory buffer
    pub fn dir<'list_lt, 'entry_lt, 'name_lt>(
        self,
        entries_list: &DirEntriesList<'list_lt, 'entry_lt, 'name_lt>,
        size: usize
    ) {
        let mut buf = DirEntList::new(size);
        for container in entries_list.as_ref().iter() {
            let item_data = container.as_ref();
            let full= buf.push(&ll_DirEntry::new(
                INodeNo(item_data.ino),
                DirEntOffset(item_data.offset),
                item_data.kind,
                item_data.name.as_ref()
            ));
            if full {
                break;
            }
        }
        self.send_ll(&buf.into());
    }

    #[cfg(feature = "abi-7-21")]
    // Reply to a request with a filled directory plus buffer
    pub fn dirplus<'list_lt, 'entry_lt, 'name_lt>(
        self,
        entries_plus_list: &DirEntryPlusList<'list_lt, 'entry_lt, 'name_lt>,
        size: usize
    ) {
        let mut buf = DirEntPlusList::new(size);
        log::debug!("ReplyHandler::dirplus: initial buf max_size: {}, size passed_to_fn: {}", size, size);
        let mut count = 0;
        for container in entries_plus_list.as_ref().iter() {
            count += 1;
            log::debug!("ReplyHandler::dirplus: processing container {}", count);
            let plus_data = container.as_ref();
            let item_data = &plus_data.entry_data;
            let item_attr = &plus_data.attr_entry;

            let full = buf.push(&DirEntryPlus::new(
                INodeNo(item_data.ino),
                Generation(item_attr.generation),
                DirEntOffset(item_data.offset),
                item_data.name.as_ref(),
                item_attr.ttl,
                item_attr.attr.into(),
                item_attr.ttl,
            ));
            if full {
                break;
            }     
        }
        self.send_ll(&buf.into());
    }

    /// Reply to a request with extended attributes.
    pub fn xattr(self, reply: Xattr<'_>) {
        match reply {
            Xattr::Size(s) => self.xattr_size(s),
            Xattr::Data(d) => self.xattr_data(d),
        }
    }

    /// Reply to a request with the size of an xattr result.
    pub fn xattr_size(self, size: u32) {
        self.send_ll(&ll::Response::new_xattr_size(size))
    }

    /// Reply to a request with the data in an xattr result.
    pub fn xattr_data(self, data: ByteBox<'_>) {
        self.send_ll(&ll::Response::new_slice(data.as_ref()))
    }

    #[cfg(feature = "abi-7-24")]
    /// Reply to a request with a seeked offset
    pub fn offset(self, offset: i64) {
        self.send_ll(&ll::Response::new_lseek(offset))
    }

}

#[cfg(test)]
mod test {
    use super::*; // For ReplyHandler, Attr, Entry, Open, Statfs, Lock, Xattr, Ioctl, DirEntryData
    use crate::{
        FileAttr, FileType, OsBox, ByteBox, KernelConfig, // KernelConfig for config test if added
        DirEntriesList, DirEntryContainer,
        DirEntryPlusList, DirEntryPlusContainer, DirEntryPlusData,
        ll, // For ll::Errno in reply_error test
    };
    use std::ffi::{OsStr, OsString};
    use std::io::IoSlice;
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, UNIX_EPOCH};
    use zerocopy::{Immutable, IntoBytes};

    // Define TestSender once for the module: captures sent data for inspection.
    #[derive(Clone)]
    struct TestSender {
        sent_data: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl ReplySender for TestSender {
        fn send(&self, data: &[IoSlice<'_>]) -> std::io::Result<()> {
            let mut consolidated_data = vec![];
            for iovec in data {
                consolidated_data.extend_from_slice(iovec);
            }
            *self.sent_data.lock().unwrap() = Some(consolidated_data);
            Ok(())
        }
        #[cfg(feature = "abi-7-40")]
        fn open_backing(&self, _fd: BorrowedFd<'_>) -> std::io::Result<BackingId> { unreachable!() }
    }

    // AssertSender: panics if sent data doesn't match expected.
    struct AssertSender {
        expected: Vec<u8>,
    }

    impl ReplySender for AssertSender {
        fn send(&self, data: &[IoSlice<'_>]) -> std::io::Result<()> {
            let mut v = vec![];
            for x in data {
                v.extend_from_slice(x)
            }
            assert_eq!(self.expected, v);
            Ok(())
        }
        #[cfg(feature = "abi-7-40")]
        fn open_backing(&self, _fd: BorrowedFd<'_>) -> std::io::Result<BackingId> {
            unreachable!()
        }
    }

    #[derive(Debug, IntoBytes, Immutable)]
    #[repr(C)]
    struct Data {
        a: u8,
        b: u8,
        c: u16,
    }

    #[test]
    fn serialize_empty() {
        assert!(().as_bytes().is_empty());
    }

    #[test]
    fn serialize_slice() {
        let data: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
        assert_eq!(data.as_bytes(), [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn serialize_struct() {
        let data = Data {
            a: 0x12,
            b: 0x34,
            c: 0x5678,
        };
        assert_eq!(data.as_bytes(), [0x12, 0x34, 0x78, 0x56]);
    }

    #[test]
    fn reply_raw() {
        let data = Data {
            a: 0x12,
            b: 0x34,
            c: 0x5678,
        };
        let sender = AssertSender {
            expected: vec![
                0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x12, 0x34, 0x78, 0x56,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.send_ll(&ll::Response::new_data(data.as_bytes()));
    }

    #[test]
    fn reply_error() {
        let sender = AssertSender {
            expected: vec![
                0x10, 0x00, 0x00, 0x00, 0xbe, 0xff, 0xff, 0xff, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.error(ll::Errno::from_i32(66));
    }

    #[test]
    fn reply_empty() {
        let sender = AssertSender {
            expected: vec![
                0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.ok();
    }

    #[test]
    fn reply_data() {
        let sender = AssertSender {
            expected: vec![
                0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0xde, 0xad, 0xbe, 0xef,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.data(&[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn reply_entry() {
        let mut expected = if cfg!(target_os = "macos") {
            vec![
                0x98, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x87,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56,
                0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00,
                0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x99, 0x00, 0x00, 0x00,
            ]
        } else {
            vec![
                0x88, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x87,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00,
                0x78, 0x56, 0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00,
                0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00,
            ]
        };

        if cfg!(feature = "abi-7-9") {
            expected.extend(vec![0xbb, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        }
        expected[0] = (expected.len()) as u8;

        let sender = AssertSender { expected };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        let time = UNIX_EPOCH + Duration::new(0x1234, 0x5678);
        let ttl = Duration::new(0x8765, 0x4321);
        let attr = FileAttr {
            ino: 0x11,
            size: 0x22,
            blocks: 0x33,
            atime: time,
            mtime: time,
            ctime: time,
            crtime: time,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 0x55,
            uid: 0x66,
            gid: 0x77,
            rdev: 0x88,
            flags: 0x99,
            blksize: 0xbb,
        };
        replyhandler.entry(
            Entry{
                attr,
                ttl,
                generation: 0xaa
            }
        );
    }

    #[test]
    fn reply_attr() {
        let mut expected = if cfg!(target_os = "macos") {
            vec![
                0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56,
                0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00,
                0x66, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x99, 0x00,
                0x00, 0x00,
            ]
        } else {
            vec![
                0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00,
                0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00,
                0x00, 0x00, 0x66, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00,
            ]
        };

        if cfg!(feature = "abi-7-9") {
            expected.extend_from_slice(&[0xbb, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        }
        expected[0] = expected.len() as u8;

        let sender = AssertSender { expected };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        let time = UNIX_EPOCH + Duration::new(0x1234, 0x5678);
        let ttl = Duration::new(0x8765, 0x4321);
        let attr = FileAttr {
            ino: 0x11,
            size: 0x22,
            blocks: 0x33,
            atime: time,
            mtime: time,
            ctime: time,
            crtime: time,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 0x55,
            uid: 0x66,
            gid: 0x77,
            rdev: 0x88,
            flags: 0x99,
            blksize: 0xbb,
        };
        replyhandler.attr(
            Attr { attr, ttl }
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn reply_xtimes() {
        let sender = AssertSender {
            expected: vec![
                0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        let time = UNIX_EPOCH + Duration::new(0x1234, 0x5678);
        replyhandler.xtimes(
            XTimes{
                bkuptime: time,
                crtime: time,
            }
        );
    }

    #[test]
    fn reply_open() {
        let sender = AssertSender {
            expected: vec![
                0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x22, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.opened(
            Open { fh: 0x1122, flags: 0x33}
        );
    }

    #[test]
    fn reply_write() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x22, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.written(0x1122);
    }

    #[test]
    fn reply_statfs() {
        let sender = AssertSender {
            expected: vec![
                0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x66, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.statfs(
            Statfs{
                blocks: 0x11, 
                bfree: 0x22, 
                bavail: 0x33, 
                files: 0x44, 
                ffree: 0x55, 
                bsize: 0x66, 
                namelen: 0x77, 
                frsize: 0x88
            }
        );
    }

    #[test]
    fn reply_create() {
        let mut expected = if cfg!(target_os = "macos") {
            vec![
                0xa8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x87,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56,
                0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00,
                0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x99, 0x00, 0x00, 0x00, 0xbb, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xcc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        } else {
            vec![
                0x98, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x65, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x65, 0x87,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00, 0x21, 0x43, 0x00, 0x00,
                0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x12,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00, 0x78, 0x56, 0x00, 0x00,
                0x78, 0x56, 0x00, 0x00, 0xa4, 0x81, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00,
                0x00, 0x00, 0x77, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0xbb, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        };

        if cfg!(feature = "abi-7-9") {
            let insert_at = expected.len() - 16;
            expected.splice(
                insert_at..insert_at,
                vec![0xdd, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            );
        }
        expected[0] = (expected.len()) as u8;

        let sender = AssertSender { expected };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        let time = UNIX_EPOCH + Duration::new(0x1234, 0x5678);
        let ttl = Duration::new(0x8765, 0x4321);
        let attr = FileAttr {
            ino: 0x11,
            size: 0x22,
            blocks: 0x33,
            atime: time,
            mtime: time,
            ctime: time,
            crtime: time,
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 0x55,
            uid: 0x66,
            gid: 0x77,
            rdev: 0x88,
            flags: 0x99,
            blksize: 0xdd,
        };
        replyhandler.created(
            Entry {
                attr: attr, 
                ttl: ttl,
                generation: 0xaa
            },
            Open {
                fh: 0xbb,
                flags: 0x0f
            }
        );
    }

    #[test]
    fn reply_lock() {
        let sender = AssertSender {
            expected: vec![
                0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0x44, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.locked(
            Lock {
                start: 0x11,
                end: 0x22,
                typ: 0x33,
                pid: 0x44
            }
        );
    }

    #[test]
    fn reply_bmap() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.bmap(0x1234);
    }

    #[test]
    fn reply_directory() {
        let sent_data_arc = Arc::new(Mutex::new(None));
        let test_sender = TestSender { sent_data: sent_data_arc.clone() };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, test_sender);

        let entry_data1 = DirEntryData {
            ino: 0xaabb,
            offset: 1,
            kind: FileType::Directory,
            name: OsBox::Owned(OsString::from("hello").into_boxed_os_str()),
        };
        let entry_data2 = DirEntryData {
            ino: 0xccdd,
            offset: 2,
            kind: FileType::RegularFile,
            name: OsBox::Owned(OsString::from("world.rs").into_boxed_os_str()),
        };

        let containers: Vec<DirEntryContainer<'static, 'static>> = vec![
            DirEntryContainer::Owned(entry_data1),
            DirEntryContainer::Owned(entry_data2),
        ];

        let entries_list = DirEntriesList::Owned(containers.into_boxed_slice());
        replyhandler.dir(&entries_list, std::mem::size_of::<u8>()*128);

        let sent_data_guard = sent_data_arc.lock().unwrap();
        let sent_data_option = &*sent_data_guard;
        assert!(sent_data_option.is_some(), "Data should have been sent by dir");
        if let Some(data) = sent_data_option {
            // A FUSE header is 16 bytes. If entries were added, it should be larger.
            assert!(data.len() > 16, "Sent data should be more than just a header for non-empty entries_list");
            // Further byte validation is complex and done in ll::reply::test::reply_directory
        }
    }

    // TODO: Add a similar test for reply_dirplus if it exists and needs updating,
    // or ensure it's covered by other tests or removed if no longer applicable.
    // The current `dirplus` in ReplyHandler takes `&[(DirEntryData, Entry)]` which is now incorrect.
    // It should take `&DirEntryPlusList<...>`

    #[test]
    #[cfg(feature = "abi-7-21")]
    fn reply_dirplus() {
        use std::ffi::{OsStr, OsString};
        // FileAttr is available from crate scope via `super::*` or direct `crate::FileAttr`
        use crate::byte_box::{DirEntryPlusData, DirEntryPlusContainer, DirEntryPlusList, OsBox as ActualOsBox}; // Alias OsBox to avoid conflict if super::OsBox exists
        use crate::FileAttr; // Explicit import if not covered by super::* or to be clear

        let sent_data_arc = Arc::new(Mutex::new(None));
        let test_sender = TestSender { sent_data: sent_data_arc.clone() }; // Uses module-level TestSender
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, test_sender);

        let entry_data1 = super::DirEntryData { // Use super to be explicit
            ino: 0xaabb,
            offset: 1,
            kind: FileType::Directory,
            name: ActualOsBox::Borrowed(OsStr::new("hello")),
        };
        let attr1_fa = crate::FileAttr { ino: 0xaabb, size: 0, blocks: 0, atime: UNIX_EPOCH, mtime: UNIX_EPOCH, ctime: UNIX_EPOCH, crtime: UNIX_EPOCH, kind: FileType::Directory, perm: 0o755, nlink: 2, uid: 1000, gid: 1000, rdev: 0, blksize: 4096, flags: 0 };
        let attr1 = super::Attr { attr: attr1_fa, ttl: Duration::from_secs(1) };
        let plus_data1 = DirEntryPlusData { entry_data: entry_data1, attr_entry: super::Entry { attr: attr1.attr, ttl: attr1.ttl, generation: 1 } };

        let entry_data2 = super::DirEntryData {
            ino: 0xccdd,
            offset: 2,
            kind: FileType::RegularFile,
            name: ActualOsBox::Owned(OsString::from("world.rs").into_boxed_os_str()),
        };
        let attr2_fa = crate::FileAttr { ino: 0xccdd, size: 10, blocks: 1, atime: UNIX_EPOCH, mtime: UNIX_EPOCH, ctime: UNIX_EPOCH, crtime: UNIX_EPOCH, kind: FileType::RegularFile, perm: 0o644, nlink: 1, uid: 1000, gid: 1000, rdev: 0, blksize: 4096, flags: 0 };
        let attr2 = super::Attr { attr: attr2_fa, ttl: Duration::from_secs(1) };
        let plus_data2 = DirEntryPlusData { entry_data: entry_data2, attr_entry: super::Entry { attr: attr2.attr, ttl: attr2.ttl, generation: 1 } };

        let containers: Vec<DirEntryPlusContainer<'static, 'static>> = vec![
            DirEntryPlusContainer::Owned(plus_data1.clone()),
            DirEntryPlusContainer::Owned(plus_data2.clone()),
        ];

        let entries_list = DirEntryPlusList::Owned(containers.into_boxed_slice());

        replyhandler.dirplus(&entries_list, std::mem::size_of::<u8>()*256);

        let sent_data_guard = sent_data_arc.lock().unwrap();
        let sent_data_option = &*sent_data_guard;

        assert!(sent_data_option.is_some(), "Data should have been sent by dirplus");
        if let Some(data) = sent_data_option {
            // A FUSE header is 16 bytes. If entries were added, it should be larger.
            assert!(data.len() > 16, "Sent data should be more than just a header for non-empty entries_list. Actual length: {}", data.len());
        }
    }

    #[test]
    fn reply_xattr_size() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00,
                0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        replyhandler.xattr(Xattr::Size(0x12345678));
    }

    #[test]
    fn reply_xattr_data() {
        let sender = AssertSender {
            expected: vec![
                0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x22, 0x33, 0x44,
            ],
        };
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, sender);
        let data_vec = vec![0x11, 0x22, 0x33, 0x44];
        replyhandler.xattr(Xattr::Data(ByteBox::from(data_vec)));
    }

    impl super::ReplySender for SyncSender<()> {
        fn send(&self, _: &[IoSlice<'_>]) -> std::io::Result<()> {
            self.send(()).unwrap();
            Ok(())
        }

        #[cfg(feature = "abi-7-40")]
        fn open_backing(&self, _fd: BorrowedFd<'_>) -> std::io::Result<BackingId> {
            unreachable!()
        }
    }

    #[test]
    fn async_reply() {
        let (tx, rx) = sync_channel::<()>(1);
        let replyhandler: ReplyHandler = ReplyHandler::new(0xdeadbeef, tx);
        thread::spawn(move || {
            replyhandler.ok();
        });
        rx.recv().unwrap();
    }
}
