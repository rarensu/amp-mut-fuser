//! Filesystem operation reply
//!
//! A reply is passed to filesystem operation implementations and must be used to send back the
//! result of an operation. The reply can optionally be sent to another thread to asynchronously
//! work on an operation and provide the result later. Also it allows replying with a block of
//! data without cloning the data. A reply *must always* be used (by calling either ok() or
//! error() exactly once).

use crate::ll::{
    self,
    reply::{DirEntPlusList, DirEntryPlus},
    Generation,
};
use crate::ll::{
    reply::{DirEntList, DirEntOffset, DirEntry},
    INodeNo,
};
#[cfg(feature = "abi-7-40")]
use crate::{consts::FOPEN_PASSTHROUGH, passthrough::BackingId};
use libc::c_int;
use log::{error, warn};
use std::convert::AsRef;
use std::ffi::OsStr;
use std::fmt;
use std::io::IoSlice;
#[cfg(feature = "abi-7-40")]
use std::os::fd::BorrowedFd;
use std::time::Duration;

#[cfg(target_os = "macos")]
use std::time::SystemTime;

use crate::{FileAttr, FileType};

/// Generic reply callback to send data
pub trait ReplySender: Send + Sync + Unpin + 'static {
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

/// Generic reply trait
pub trait Reply2 {
    /// Create a new reply for the given request
    fn new<S: ReplySender>(unique: u64, sender: S) -> Self;
}

///
/// Raw reply
///
#[derive(Debug)]
pub(crate) struct ReplyRaw2 {
    /// Unique id of the request to reply to
    unique: ll::RequestId,
    /// Closure to call for sending the reply
    sender: Option<Box<dyn ReplySender>>,
}

impl Reply2 for ReplyRaw2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyRaw2 {
        let sender = Box::new(sender);
        ReplyRaw2 {
            unique: ll::RequestId(unique),
            sender: Some(sender),
        }
    }
}

impl ReplyRaw2 {
    /// Reply to a request with the given error code and data. Must be called
    /// only once (the `ok` and `error` methods ensure this by consuming `self`)
    fn send_ll_mut(&mut self, response: &ll::Response<'_>) {
        assert!(self.sender.is_some());
        let sender = self.sender.take().unwrap();
        let res = response.with_iovec(self.unique, |iov| sender.send(iov));
        if let Err(err) = res {
            error!("Failed to send FUSE reply: {}", err);
        }
    }
    fn send_ll(mut self, response: &ll::Response<'_>) {
        self.send_ll_mut(response)
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        assert_ne!(err, 0);
        self.send_ll(&ll::Response::new_error(ll::Errno::from_i32(err)));
    }
}

impl Drop for ReplyRaw2 {
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

///
/// Empty reply
///
#[derive(Debug)]
pub struct ReplyEmpty2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyEmpty2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyEmpty2 {
        ReplyEmpty2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyEmpty2 {
    /// Reply to a request with nothing
    pub fn ok2(self) {
        self.reply.send_ll(&ll::Response::new_empty());
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Data reply
///
#[derive(Debug)]
pub struct ReplyData2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyData2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyData2 {
        ReplyData2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyData2 {
    /// Reply to a request with the given data
    pub fn data2(self, data: &[u8]) {
        self.reply.send_ll(&ll::Response::new_slice(data));
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Entry reply
///
#[derive(Debug)]
pub struct ReplyEntry2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyEntry2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyEntry2 {
        ReplyEntry2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyEntry2 {
    /// Reply to a request with the given entry
    pub fn entry2(self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.reply.send_ll(&ll::Response::new_entry(
            ll::INodeNo(attr.ino),
            ll::Generation(generation),
            &attr.into(),
            *ttl,
            *ttl,
        ));
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Attribute Reply
///
#[derive(Debug)]
pub struct ReplyAttr2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyAttr2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyAttr2 {
        ReplyAttr2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyAttr2 {
    /// Reply to a request with the given attribute
    pub fn attr2(self, ttl: &Duration, attr: &FileAttr) {
        self.reply
            .send_ll(&ll::Response::new_attr(ttl, &attr.into()));
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// XTimes Reply
///
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ReplyXTimes2 {
    reply: ReplyRaw2,
}

#[cfg(target_os = "macos")]
impl Reply2 for ReplyXTimes2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyXTimes2 {
        ReplyXTimes2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

#[cfg(target_os = "macos")]
impl ReplyXTimes2 {
    /// Reply to a request with the given xtimes
    pub fn xtimes2(self, bkuptime: SystemTime, crtime: SystemTime) {
        self.reply
            .send_ll(&ll::Response::new_xtimes(bkuptime, crtime))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Open Reply
///
#[derive(Debug)]
pub struct ReplyOpen2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyOpen2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyOpen2 {
        ReplyOpen2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyOpen2 {
    /// Reply to a request with the given open result
    pub fn opened2(self, fh: u64, flags: u32) {
        #[cfg(feature = "abi-7-40")]
        assert_eq!(flags & FOPEN_PASSTHROUGH, 0);
        self.reply
            .send_ll(&ll::Response::new_open(ll::FileHandle(fh), flags, 0))
    }

    /// Registers a fd for passthrough, returning a `BackingId`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `OpenReply::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    pub fn open_backing2(&self, fd: impl std::os::fd::AsFd) -> std::io::Result<BackingId> {
        self.reply.sender.as_ref().unwrap().open_backing(fd.as_fd())
    }

    /// Reply to a request with an opened backing id.  Call ReplyOpen2::open_backing() to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    pub fn opened_passthrough2(self, fh: u64, flags: u32, backing_id: &BackingId) {
        self.reply.send_ll(&ll::Response::new_open(
            ll::FileHandle(fh),
            flags | FOPEN_PASSTHROUGH,
            backing_id.backing_id,
        ))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Write Reply
///
#[derive(Debug)]
pub struct ReplyWrite2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyWrite2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyWrite2 {
        ReplyWrite2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyWrite2 {
    /// Reply to a request with the given open result
    pub fn written2(self, size: u32) {
        self.reply.send_ll(&ll::Response::new_write(size))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Statfs Reply
///
#[derive(Debug)]
pub struct ReplyStatfs2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyStatfs2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyStatfs2 {
        ReplyStatfs2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyStatfs2 {
    /// Reply to a request with the given open result
    #[allow(clippy::too_many_arguments)]
    pub fn statfs2(
        self,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        bsize: u32,
        namelen: u32,
        frsize: u32,
    ) {
        self.reply.send_ll(&ll::Response::new_statfs(
            blocks, bfree, bavail, files, ffree, bsize, namelen, frsize,
        ))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Create reply
///
#[derive(Debug)]
pub struct ReplyCreate2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyCreate2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyCreate2 {
        ReplyCreate2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyCreate2 {
    /// Reply to a request with the given entry
    pub fn created2(self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32) {
        #[cfg(feature = "abi-7-40")]
        assert_eq!(flags & FOPEN_PASSTHROUGH, 0);
        self.reply.send_ll(&ll::Response::new_create(
            ttl,
            &attr.into(),
            ll::Generation(generation),
            ll::FileHandle(fh),
            flags,
            0,
        ))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Lock Reply
///
#[derive(Debug)]
pub struct ReplyLock2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyLock2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyLock2 {
        ReplyLock2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyLock2 {
    /// Reply to a request with the given open result
    pub fn locked2(self, start: u64, end: u64, typ: i32, pid: u32) {
        self.reply.send_ll(&ll::Response::new_lock(&ll::Lock {
            range: (start, end),
            typ,
            pid,
        }))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Bmap Reply
///
#[derive(Debug)]
pub struct ReplyBmap2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyBmap2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyBmap2 {
        ReplyBmap2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyBmap2 {
    /// Reply to a request with the given open result
    pub fn bmap2(self, block: u64) {
        self.reply.send_ll(&ll::Response::new_bmap(block))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Ioctl Reply
///
#[derive(Debug)]
pub struct ReplyIoctl2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyIoctl2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyIoctl2 {
        ReplyIoctl2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyIoctl2 {
    /// Reply to a request with the given open result
    pub fn ioctl2(self, result: i32, data: &[u8]) {
        self.reply
            .send_ll(&ll::Response::new_ioctl(result, &[IoSlice::new(data)]));
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Poll Reply
///
#[derive(Debug)]
#[cfg(feature = "abi-7-11")]
pub struct ReplyPoll2 {
    reply: ReplyRaw2,
}

#[cfg(feature = "abi-7-11")]
impl Reply2 for ReplyPoll2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyPoll2 {
        ReplyPoll2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

#[cfg(feature = "abi-7-11")]
impl ReplyPoll2 {
    /// Reply to a request with the given poll result
    pub fn poll2(self, revents: u32) {
        self.reply.send_ll(&ll::Response::new_poll(revents))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Directory reply
///
#[derive(Debug)]
pub struct ReplyDirectory2 {
    reply: ReplyRaw2,
    data: DirEntList,
}

impl ReplyDirectory2 {
    /// Creates a new ReplyDirectory2 with a specified buffer size.
    pub fn new<S: ReplySender>(unique: u64, sender: S, size: usize) -> ReplyDirectory2 {
        ReplyDirectory2 {
            reply: Reply2::new(unique, sender),
            data: DirEntList::new(size),
        }
    }

    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    pub fn add2<T: AsRef<OsStr>>(&mut self, ino: u64, offset: i64, kind: FileType, name: T) -> bool {
        let name = name.as_ref();
        self.data.push(&DirEntry::new(
            INodeNo(ino),
            DirEntOffset(offset),
            kind,
            name,
        ))
    }

    /// Reply to a request with the filled directory buffer
    pub fn ok2(self) {
        self.reply.send_ll(&self.data.into());
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// DirectoryPlus reply
///
#[derive(Debug)]
pub struct ReplyDirectoryPlus2 {
    reply: ReplyRaw2,
    buf: DirEntPlusList,
}

impl ReplyDirectoryPlus2 {
    /// Creates a new ReplyDirectory2 with a specified buffer size.
    pub fn new<S: ReplySender>(unique: u64, sender: S, size: usize) -> ReplyDirectoryPlus2 {
        ReplyDirectoryPlus2 {
            reply: Reply2::new(unique, sender),
            buf: DirEntPlusList::new(size),
        }
    }

    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    pub fn add2<T: AsRef<OsStr>>(
        &mut self,
        ino: u64,
        offset: i64,
        name: T,
        ttl: &Duration,
        attr: &FileAttr,
        generation: u64,
    ) -> bool {
        let name = name.as_ref();
        self.buf.push(&DirEntryPlus::new(
            INodeNo(ino),
            Generation(generation),
            DirEntOffset(offset),
            name,
            *ttl,
            attr.into(),
            *ttl,
        ))
    }

    /// Reply to a request with the filled directory buffer
    pub fn ok2(self) {
        self.reply.send_ll(&self.buf.into());
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Xattr reply
///
#[derive(Debug)]
pub struct ReplyXattr2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyXattr2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyXattr2 {
        ReplyXattr2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyXattr2 {
    /// Reply to a request with the size of the xattr.
    pub fn size2(self, size: u32) {
        self.reply.send_ll(&ll::Response::new_xattr_size(size))
    }

    /// Reply to a request with the data in the xattr.
    pub fn data2(self, data: &[u8]) {
        self.reply.send_ll(&ll::Response::new_slice(data))
    }

    /// Reply to a request with the given error code.
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

///
/// Lseek Reply
///
#[derive(Debug)]
pub struct ReplyLseek2 {
    reply: ReplyRaw2,
}

impl Reply2 for ReplyLseek2 {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyLseek2 {
        ReplyLseek2 {
            reply: Reply2::new(unique, sender),
        }
    }
}

impl ReplyLseek2 {
    /// Reply to a request with seeked offset
    pub fn offset2(self, offset: i64) {
        self.reply.send_ll(&ll::Response::new_lseek(offset))
    }

    /// Reply to a request with the given error code
    pub fn error2(self, err: c_int) {
        self.reply.error2(err);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{FileAttr, FileType};
    use std::io::IoSlice;
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::thread;
    use std::time::{Duration, UNIX_EPOCH};
    use zerocopy::{Immutable, IntoBytes};

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

    struct AssertSender {
        expected: Vec<u8>,
    }

    impl super::ReplySender for AssertSender {
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
        let reply: ReplyRaw2 = Reply2::new(0xdeadbeef, sender);
        reply.send_ll(&ll::Response::new_data(data.as_bytes()));
    }

    #[test]
    fn reply_error() {
        let sender = AssertSender {
            expected: vec![
                0x10, 0x00, 0x00, 0x00, 0xbe, 0xff, 0xff, 0xff, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00,
            ],
        };
        let reply: ReplyRaw2 = Reply2::new(0xdeadbeef, sender);
        reply.error(66);
    }

    #[test]
    fn reply_empty() {
        let sender = AssertSender {
            expected: vec![
                0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00,
            ],
        };
        let reply: ReplyEmpty2 = Reply2::new(0xdeadbeef, sender);
        reply.ok();
    }

    #[test]
    fn reply_data() {
        let sender = AssertSender {
            expected: vec![
                0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0xde, 0xad, 0xbe, 0xef,
            ],
        };
        let reply: ReplyData2 = Reply2::new(0xdeadbeef, sender);
        reply.data(&[0xde, 0xad, 0xbe, 0xef]);
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
        let reply: ReplyEntry2 = Reply2::new(0xdeadbeef, sender);
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
        reply.entry(&ttl, &attr, 0xaa);
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
        let reply: ReplyAttr2 = Reply2::new(0xdeadbeef, sender);
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
        reply.attr(&ttl, &attr);
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
        let reply: ReplyXTimes2 = Reply2::new(0xdeadbeef, sender);
        let time = UNIX_EPOCH + Duration::new(0x1234, 0x5678);
        reply.xtimes(time, time);
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
        let reply: ReplyOpen2 = Reply2::new(0xdeadbeef, sender);
        reply.opened(0x1122, 0x33);
    }

    #[test]
    fn reply_write() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x22, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let reply: ReplyWrite2 = Reply2::new(0xdeadbeef, sender);
        reply.written(0x1122);
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
        let reply: ReplyStatfs2 = Reply2::new(0xdeadbeef, sender);
        reply.statfs(0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88);
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
                0x00, 0x00, 0x00, 0x00, 0xcc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
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
        let reply: ReplyCreate2 = Reply2::new(0xdeadbeef, sender);
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
        reply.created(&ttl, &attr, 0xaa, 0xbb, 0xcc);
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
        let reply: ReplyLock2 = Reply2::new(0xdeadbeef, sender);
        reply.locked(0x11, 0x22, 0x33, 0x44);
    }

    #[test]
    fn reply_bmap() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let reply: ReplyBmap2 = Reply2::new(0xdeadbeef, sender);
        reply.bmap(0x1234);
    }

    #[test]
    fn reply_directory() {
        let sender = AssertSender {
            expected: vec![
                0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xef, 0xbe, 0xad, 0xde, 0x00, 0x00,
                0x00, 0x00, 0xbb, 0xaa, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x68, 0x65,
                0x6c, 0x6c, 0x6f, 0x00, 0x00, 0x00, 0xdd, 0xcc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00,
                0x00, 0x00, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x2e, 0x72, 0x73,
            ],
        };
        let mut reply = ReplyDirectory2::new(0xdeadbeef, sender, 4096);
        assert!(!reply.add(0xaabb, 1, FileType::Directory, "hello"));
        assert!(!reply.add(0xccdd, 2, FileType::RegularFile, "world.rs"));
        reply.ok();
    }

    #[test]
    fn reply_xattr_size() {
        let sender = AssertSender {
            expected: vec![
                0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00,
                0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00,
            ],
        };
        let reply = ReplyXattr2::new(0xdeadbeef, sender);
        reply.size(0x12345678);
    }

    #[test]
    fn reply_xattr_data() {
        let sender = AssertSender {
            expected: vec![
                0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00,
                0x00, 0x00, 0x11, 0x22, 0x33, 0x44,
            ],
        };
        let reply = ReplyXattr2::new(0xdeadbeef, sender);
        reply.data(&[0x11, 0x22, 0x33, 0x44]);
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
        let reply: ReplyEmpty2 = Reply2::new(0xdeadbeef, tx);
        thread::spawn(move || {
            reply.ok();
        });
        rx.recv().unwrap();
    }
}
