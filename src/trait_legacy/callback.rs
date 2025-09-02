use libc::c_int;
use std::convert::AsRef;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::time::Duration;
use zerocopy::IntoBytes;

#[cfg(target_os = "macos")]
use crate::XTimes;
#[cfg(feature = "abi-7-21")]
use crate::ll::reply::fuse_attr_from_attr;
use crate::{
    channel::Channel,
    data::{Entry, Lock, Open, FileAttr, FileType, Statfs},
    ll::{
        Errno,
        reply::{EntListBuf, Response, mode_from_kind_and_perm},
    },
    reply::ReplyHandler,
};
#[cfg(target_os = "macos")]
use std::time::SystemTime;

/*
#[cfg(feature = "abi-7-11")]
use super::PollHandler;
*/

#[cfg(feature = "abi-7-40")]
use super::{BackingHandler, BackingId};
#[cfg(feature = "abi-7-40")]
use crate::consts::FOPEN_PASSTHROUGH;
#[cfg(feature = "abi-7-40")]
use std::io;
#[cfg(feature = "abi-7-40")]
use std::os::fd::AsRawFd;

/* ------ Err ------ */


macro_rules! default_error {
    () => {
        /// Reply to a request with the given error code
        pub fn error(mut self, err: c_int) {
            self.reply.error(Errno::from_i32(err))
        }
    };
}

///
/// Empty reply
///
#[derive(Debug)]
pub struct ReplyEmpty {
    reply: ReplyHandler<Channel>,
}

impl ReplyEmpty {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyEmpty {
        ReplyEmpty {reply}
    }

    /// Reply to a request with nothing
    pub fn ok(self) {
        self.reply.ok();
    }
    default_error!();
}

///
/// Data reply
///
#[derive(Debug)]
pub struct ReplyData {
    reply: ReplyHandler<Channel>,
}

impl ReplyData {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyData {
        ReplyData {reply}
    }

    /// Reply to a request with the given data
    pub fn data(self, data: &[u8]) {
        self.reply.data(data);
    }
    default_error!();
}

///
/// Entry reply
///
#[derive(Debug)]
pub struct ReplyEntry {
    reply: ReplyHandler<Channel>,
}

impl ReplyEntry {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyEntry {
        ReplyEntry {reply}
    }

    /// Reply to a request with the given entry
    pub fn entry(self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.reply.entry(&Entry {
                ino: attr.ino,
                generation: Some(generation),
                file_ttl: *ttl,
                attr: *attr,
                attr_ttl: *ttl,
            });
    }
    default_error!();
}

///
/// Attribute Reply
///
#[derive(Debug)]
pub struct ReplyAttr {
    reply: ReplyHandler<Channel>,
}

impl ReplyAttr {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyAttr {
        ReplyAttr {reply}
    }

    /// Reply to a request with the given attribute
    pub fn attr(self, ttl: &Duration, attr: &FileAttr) {
        self.reply.attr(ttl, attr);
    }
    default_error!();
}

///
/// XTimes Reply
///
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ReplyXTimes {
    reply: ReplyHandler<Channel>,
}

#[cfg(target_os = "macos")]
impl ReplyXTimes {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyXTimes {
        ReplyXTimes {reply}
    }

    /// Reply to a request with the given xtimes
    pub fn xtimes(self, bkuptime: SystemTime, crtime: SystemTime) {
        self.reply.xtimes(bkuptime, crtime);
    }
    default_error!();
}

///
/// Open Reply
///
#[derive(Debug)]
pub struct ReplyOpen {
    reply: ReplyHandler<Channel>,
    #[cfg(feature = "abi-7-40")]
    backer: BackingHandler<Channel>,
}

impl ReplyOpen {
    pub fn new(
        reply: ReplyHandler<Channel>,
        #[cfg(feature = "abi-7-40")]
        backer: BackingHandler<Channel>
    ) -> ReplyOpen {
        ReplyOpen {
            reply, 
            #[cfg(feature = "abi-7-40")]
            backer
        }
    }

    /// Reply to a request with the given open result
    pub fn opened(self, fh: u64, flags: u32) {
        self.reply.opened(&Open {
                fh,
                flags,
                backing_id: None,
            })
    }

    /// Registers a fd for passthrough, returning a `BackingId`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `OpenReply::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    pub fn open_backing(&self, fd: impl std::os::fd::AsFd) -> std::io::Result<BackingId> {
        self.backer.open_backing(fd.as_fd())
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
    reply: ReplyHandler<Channel>,
}

impl ReplyWrite {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyWrite {
        ReplyWrite {reply}
    }

    /// Reply to a request with the given open result
    pub fn written(self, size: u32) {
        self.reply.written(size);
    }
    default_error!();
}

///
/// Statfs Reply
///
#[derive(Debug)]
pub struct ReplyStatfs {
    reply: ReplyHandler<Channel>,
}

impl ReplyStatfs {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyStatfs {
        ReplyStatfs {reply}
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
        self.reply.statfs(&Statfs {
                blocks,
                bfree,
                bavail,
                files,
                ffree,
                bsize,
                namelen,
                frsize,
            });
    }
    default_error!();
}

///
/// Create reply
///
#[derive(Debug)]
pub struct ReplyCreate {
    reply: ReplyHandler<Channel>,
}

impl ReplyCreate {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyCreate {
        ReplyCreate {reply}
    }

    /// Reply to a request with the given entry
    pub fn created(self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32) {
        self.reply.created(
                &Entry {
                    ino: attr.ino,
                    generation: Some(generation),
                    file_ttl: *ttl,
                    attr: *attr,
                    attr_ttl: *ttl,
                },
                &Open {
                    fh,
                    flags,
                    backing_id: None,
                },
            );
    }
    default_error!();
}

///
/// Lock Reply
///
#[derive(Debug)]
pub struct ReplyLock {
    reply: ReplyHandler<Channel>,
}

impl ReplyLock {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyLock {
        ReplyLock {reply}
    }

    /// Reply to a request with the given open result
    pub fn locked(self, start: u64, end: u64, typ: i32, pid: u32) {
        self.reply.locked(&Lock {
                start,
                end,
                typ,
                pid,
            });
    }
    default_error!();
}

///
/// Bmap Reply
///
#[derive(Debug)]
pub struct ReplyBmap {
    reply: ReplyHandler<Channel>,
}

impl ReplyBmap {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyBmap {
        ReplyBmap {reply}
    }

    /// Reply to a request with the given open result
    pub fn bmap(self, block: u64) {
        self.reply.bmap(block);
    }
    default_error!();
}

///
/// Ioctl Reply
///
#[derive(Debug)]
pub struct ReplyIoctl {
    reply: ReplyHandler<Channel>,
}

impl ReplyIoctl {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyIoctl {
        ReplyIoctl {reply}
    }

    /// Reply to a request with the given open result
    pub fn ioctl(self, result: i32, data: &[u8]) {
        self.reply.ioctl(result, data);
    }
    default_error!();
}

///
/// Poll Reply
///
#[derive(Debug)]
pub struct ReplyPoll {
    reply: ReplyHandler<Channel>,
}

impl ReplyPoll {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyPoll {
        ReplyPoll {reply}
    }

    /// Reply to a request with the given poll result
    pub fn poll(self, revents: u32) {
        self.reply.poll(revents);
    }
    default_error!();
}

///
/// Directory reply
///
#[derive(Debug)]
pub struct ReplyDirectory {
    reply: ReplyHandler<Channel>,
    data: EntListBuf,
}

impl ReplyDirectory {
    /// Creates a new ReplyDirectory with a specified buffer size.
    pub fn new(reply: ReplyHandler<Channel>, size: usize) -> ReplyDirectory {
        ReplyDirectory {
            reply,
            data: EntListBuf::new(size),
        }
    }

    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    fn add(&mut self, ino: u64, offset: i64, kind: FileType, name: &OsStr) -> bool {
        let mut buf = &mut self.data; 
            let namelen = match name.len().try_into() {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Directory entry name too long or {e:?}");
                    log::error!("{:?}", name);
                    return true;
                }
            };
            let header = crate::ll::fuse_abi::fuse_dirent {
                ino,
                off: offset,
                namelen,
                typ: mode_from_kind_and_perm(kind, 0) >> 12,
            };
            let res = buf.push([header.as_bytes(), name.as_encoded_bytes()]);
            res

    }

    /// Reply to a request with the filled directory buffer
    pub fn ok(self) {
        self.reply.send_ll(&Response::new_directory(self.data));
    }
    default_error!();
}

///
/// DirectoryPlus reply
///
#[cfg(feature = "abi-7-21")]

#[derive(Debug)]
pub struct ReplyDirectoryPlus {
    reply: ReplyHandler<Channel>,
    buf: EntListBuf,
}
#[cfg(feature = "abi-7-21")]

impl ReplyDirectoryPlus {
    /// Creates a new ReplyDirectory with a specified buffer size.
    pub fn new(reply: ReplyHandler<Channel>, size: usize) -> ReplyDirectoryPlus {
        ReplyDirectoryPlus {
            reply,
            buf: EntListBuf::new(size),
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
        let namelen = match name.len().try_into() {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Directory entry name too long or {e:?}");
                    log::error!("{:?}", name);
                    return true;
                }
            };
            let header = crate::ll::fuse_abi::fuse_direntplus {
                entry_out: crate::ll::fuse_abi::fuse_entry_out {
                    nodeid: ino,
                    generation,
                    entry_valid: ttl.as_secs(),
                    attr_valid:
                        ttl.as_secs()
                    ,
                    entry_valid_nsec: ttl.subsec_nanos(),
                    attr_valid_nsec: 
                        ttl.subsec_nanos()
                    ,
                    attr: fuse_attr_from_attr(attr),
                },
                dirent: crate::ll::fuse_abi::fuse_dirent {
                    ino,
                    off: offset,
                    namelen,
                    typ: mode_from_kind_and_perm(attr.kind, 0) >> 12,
                },
            };
            let res = self.buf.push([header.as_bytes(), name.as_encoded_bytes()]);
            res
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
    reply: ReplyHandler<Channel>,
}

impl ReplyXattr {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyXattr {
        ReplyXattr {reply}
    }

    /// Reply to a request with the size of the xattr.
    pub fn size(self, size: u32) {
        self.reply.xattr_size(size);
    }

    /// Reply to a request with the data in the xattr.
    pub fn data(self, data: &[u8]) {
        self.reply.xattr_data(data);
    }
    default_error!();
}

///
/// Lseek Reply
///
#[cfg(feature = "abi-7-24")]
#[derive(Debug)]
pub struct ReplyLseek {
    reply: ReplyHandler<Channel>,
}

#[cfg(feature = "abi-7-24")]
impl ReplyLseek {
    pub fn new(reply: ReplyHandler<Channel>) -> ReplyLseek {
        ReplyLseek {reply}
    }

    /// Reply to a request with seeked offset
    pub fn offset(self, offset: i64) {
        self.reply.offset(offset);
    }
    default_error!();
}