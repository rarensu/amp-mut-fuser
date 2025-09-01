use libc::c_int;
use std::convert::AsRef;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::time::Duration;
use zerocopy::IntoBytes;

use crate::{FileAttr, FileType};

macro_rules! default_error {
    () => {
        /// Reply to a request with the given error code
        pub fn error(mut self, err: c_int) {
            self.reply.error(err)
        }
    };
}

///
/// Empty reply
///
#[derive(Debug)]
pub struct ReplyEmpty {
    reply: ReplyRaw,
}

impl ReplyEmpty {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyEmpty {
        ReplyEmpty {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with nothing
    pub fn ok(self) {
        self.reply.send_ll(&ll::Response::new_empty());
    }
    default_error!();
}

///
/// Data reply
///
#[derive(Debug)]
pub struct ReplyData {
    reply: ReplyRaw,
}

impl ReplyData {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyData {
        ReplyData {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given data
    pub fn data(self, data: &[u8]) {
        self.reply.send_ll(&ll::Response::new_slice(data));
    }
    default_error!();
}

///
/// Entry reply
///
#[derive(Debug)]
pub struct ReplyEntry {
    reply: ReplyRaw,
}

impl ReplyEntry {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyEntry {
        ReplyEntry {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given entry
    pub fn entry(self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.reply.send_ll(&ll::Response::new_entry(
            ll::INodeNo(attr.ino),
            ll::Generation(generation),
            &attr.into(),
            *ttl,
            *ttl,
        ));
    }
    default_error!();
}

///
/// Attribute Reply
///
#[derive(Debug)]
pub struct ReplyAttr {
    reply: ReplyRaw,
}

impl ReplyAttr {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyAttr {
        ReplyAttr {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given attribute
    pub fn attr(self, ttl: &Duration, attr: &FileAttr) {
        self.reply
            .send_ll(&ll::Response::new_attr(ttl, &attr.into()));
    }
    default_error!();
}

///
/// XTimes Reply
///
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ReplyXTimes {
    reply: ReplyRaw,
}

#[cfg(target_os = "macos")]
impl ReplyXTimes {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyXTimes {
        ReplyXTimes {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given xtimes
    pub fn xtimes(self, bkuptime: SystemTime, crtime: SystemTime) {
        self.reply
            .send_ll(&ll::Response::new_xtimes(bkuptime, crtime))
    }
    default_error!();
}

///
/// Open Reply
///
#[derive(Debug)]
pub struct ReplyOpen {
    reply: ReplyRaw,
}

impl ReplyOpen {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyOpen {
        ReplyOpen {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    pub fn opened(self, fh: u64, flags: u32) {
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
    pub fn open_backing(&self, fd: impl std::os::fd::AsFd) -> std::io::Result<BackingId> {
        self.reply.sender.as_ref().unwrap().open_backing(fd.as_fd())
    }

    /// Reply to a request with an opened backing id.  Call ReplyOpen::open_backing() to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    pub fn opened_passthrough(self, fh: u64, flags: u32, backing_id: &BackingId) {
        self.reply.send_ll(&ll::Response::new_open(
            ll::FileHandle(fh),
            flags | FOPEN_PASSTHROUGH,
            backing_id.backing_id,
        ))
    }
    default_error!();
}

///
/// Write Reply
///
#[derive(Debug)]
pub struct ReplyWrite {
    reply: ReplyRaw,
}

impl ReplyWrite {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyWrite {
        ReplyWrite {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    pub fn written(self, size: u32) {
        self.reply.send_ll(&ll::Response::new_write(size))
    }
    default_error!();
}

///
/// Statfs Reply
///
#[derive(Debug)]
pub struct ReplyStatfs {
    reply: ReplyRaw,
}

impl ReplyStatfs {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyStatfs {
        ReplyStatfs {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    #[allow(clippy::too_many_arguments)]
    pub fn statfs(
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
    default_error!();
}

///
/// Create reply
///
#[derive(Debug)]
pub struct ReplyCreate {
    reply: ReplyRaw,
}

impl ReplyCreate {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyCreate {
        ReplyCreate {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given entry
    pub fn created(self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32) {
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
    default_error!();
}

///
/// Lock Reply
///
#[derive(Debug)]
pub struct ReplyLock {
    reply: ReplyRaw,
}

impl ReplyLock {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyLock {
        ReplyLock {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    pub fn locked(self, start: u64, end: u64, typ: i32, pid: u32) {
        self.reply.send_ll(&ll::Response::new_lock(&ll::Lock {
            range: (start, end),
            typ,
            pid,
        }))
    }
    default_error!();
}

///
/// Bmap Reply
///
#[derive(Debug)]
pub struct ReplyBmap {
    reply: ReplyRaw,
}

impl ReplyBmap {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyBmap {
        ReplyBmap {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    pub fn bmap(self, block: u64) {
        self.reply.send_ll(&ll::Response::new_bmap(block))
    }
    default_error!();
}

///
/// Ioctl Reply
///
#[derive(Debug)]
pub struct ReplyIoctl {
    reply: ReplyRaw,
}

impl ReplyIoctl {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyIoctl {
        ReplyIoctl {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given open result
    pub fn ioctl(self, result: i32, data: &[u8]) {
        self.reply
            .send_ll(&ll::Response::new_ioctl(result, &[IoSlice::new(data)]));
    }
    default_error!();
}

///
/// Poll Reply
///
#[derive(Debug)]
pub struct ReplyPoll {
    reply: ReplyRaw,
}

impl ReplyPoll {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyPoll {
        ReplyPoll {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the given poll result
    pub fn poll(self, revents: u32) {
        self.reply.send_ll(&ll::Response::new_poll(revents))
    }
    default_error!();
}

///
/// Directory reply
///
#[derive(Debug)]
pub struct ReplyDirectory {
    reply: ReplyRaw,
    data: DirEntList,
}

impl ReplyDirectory {
    /// Creates a new ReplyDirectory with a specified buffer size.
    pub fn new<S: ReplySender>(unique: u64, sender: S, size: usize) -> ReplyDirectory {
        ReplyDirectory {
            reply: Reply::new(unique, sender),
            data: DirEntList::new(size),
        }
    }

    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    pub fn add<T: AsRef<OsStr>>(&mut self, ino: u64, offset: i64, kind: FileType, name: T) -> bool {
        let name = name.as_ref();
        self.data.push(&DirEntry::new(
            INodeNo(ino),
            DirEntOffset(offset),
            kind,
            name,
        ))
    }

    /// Reply to a request with the filled directory buffer
    pub fn ok(self) {
        self.reply.send_ll(&self.data.into());
    }
    default_error!();
}

///
/// DirectoryPlus reply
///
#[derive(Debug)]
pub struct ReplyDirectoryPlus {
    reply: ReplyRaw,
    buf: DirEntPlusList,
}

impl ReplyDirectoryPlus {
    /// Creates a new ReplyDirectory with a specified buffer size.
    pub fn new<S: ReplySender>(unique: u64, sender: S, size: usize) -> ReplyDirectoryPlus {
        ReplyDirectoryPlus {
            reply: Reply::new(unique, sender),
            buf: DirEntPlusList::new(size),
        }
    }

    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    pub fn add<T: AsRef<OsStr>>(
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
    pub fn ok(self) {
        self.reply.send_ll(&self.buf.into());
    }
    default_error!();
}

///
/// Xattr reply
///
#[derive(Debug)]
pub struct ReplyXattr {
    reply: ReplyRaw,
}

impl ReplyXattr {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyXattr {
        ReplyXattr {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with the size of the xattr.
    pub fn size(self, size: u32) {
        self.reply.send_ll(&ll::Response::new_xattr_size(size))
    }

    /// Reply to a request with the data in the xattr.
    pub fn data(self, data: &[u8]) {
        self.reply.send_ll(&ll::Response::new_slice(data))
    }
    default_error!();
}

///
/// Lseek Reply
///
#[derive(Debug)]
pub struct ReplyLseek {
    reply: ReplyRaw,
}

impl ReplyLseek {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyLseek {
        ReplyLseek {
            reply: Reply::new(unique, sender),
        }
    }

    /// Reply to a request with seeked offset
    pub fn offset(self, offset: i64) {
        self.reply.send_ll(&ll::Response::new_lseek(offset))
    }
    default_error!();
}