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
    data::{Entry, FileAttr, FileType, Lock, Open, Statfs},
    ll::{
        reply::{mode_from_kind_and_perm, EntListBuf, Response}, Errno
    },
    reply::{ReplyHandler, ReplySender},
};
#[cfg(target_os = "macos")]
use std::time::SystemTime;

#[cfg(feature = "abi-7-40")]
use super::{BackingHandler, BackingId};
#[cfg(feature = "abi-7-40")]
use crate::consts::FOPEN_PASSTHROUGH;
#[cfg(feature = "abi-7-40")]
use std::io;
#[cfg(feature = "abi-7-40")]
use std::os::fd::AsRawFd;

/* ------ Err ------ */

/// Custom callback handler for any Reply
pub trait CallbackErr: Debug {
    /// Reply to a request with the given error code
    fn error(&mut self, err: c_int);
}

/// Legacy callback handler for any Reply
impl<S: ReplySender + Debug> CallbackErr for Option<ReplyHandler<S>> {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}

macro_rules! default_error {
    () => {
        /// Reply to a request with the given error code
        pub fn error(mut self, err: c_int) {
            self.reply.error(err)
        }
    };
}

/* ------ Ok ------ */

/// Custom callback handler for ReplyEmpty
pub trait CallbackOk: CallbackErr {
    /// Reply to a request with nothing
    fn ok(&mut self);
}

/// Legacy callback handler for ReplyEmpty
impl<S: ReplySender + Debug> CallbackOk for Option<ReplyHandler<S>> {
    fn ok(&mut self) {
        if let Some(handler) = self.take() {
            handler.ok();
        }
    }
}

/// Callback container for operations that respond Ok
#[derive(Debug)]
pub struct ReplyEmpty {
    reply: Box<dyn CallbackOk>,
}

impl ReplyEmpty {
    /// Create a ReplyEmpty from the given boxed CallbackOk.
    pub fn new(reply: impl CallbackOk + 'static) -> Self {
        ReplyEmpty { reply: Box::new(reply) }
    }
    /// Reply to a request with nothing
    pub fn ok(mut self) {
        self.reply.ok();
    }
    default_error!();
}

/* ------ Data ------ */

/// Custom callback handler for ReplyData
pub trait CallbackData: CallbackErr {
    /// Reply to a request with the given data
    fn data(&mut self, data: &[u8]);
}

/// Legacy callback handler for ReplyData
impl<S: ReplySender + Debug> CallbackData for Option<ReplyHandler<S>> {
    fn data(&mut self, data: &[u8]) {
        if let Some(handler) = self.take() {
            handler.data(data);
        }
    }
}

/// Callback container for operations that respond with Bytes
#[derive(Debug)]
pub struct ReplyData {
    reply: Box<dyn CallbackData>,
}

impl ReplyData {
    /// Create a ReplyData from the given boxed CallbackData.
    pub fn new(reply: impl CallbackData + 'static) -> Self {
        ReplyData { reply: Box::new(reply) }
    }
    /// Reply to a request with the given data
    pub fn data(mut self, data: &[u8]) {
        self.reply.data(data);
    }
    default_error!();
}

/* ------ Entry ------ */
/// Custom callback handler for ReplyEntry
pub trait CallbackEntry: CallbackErr {
    /// Reply to a request with the given entry
    fn entry(&mut self, ttl: &Duration, attr: &FileAttr, generation: u64);
}

/// Legacy callback handler for ReplyEntry
impl<S: ReplySender + Debug> CallbackEntry for Option<ReplyHandler<S>> {
    fn entry(&mut self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        if let Some(handler) = self.take() {
            handler.entry(&Entry {
                ino: attr.ino,
                generation: Some(generation),
                file_ttl: *ttl,
                attr: *attr,
                attr_ttl: *ttl,
            });
        }
    }
}

/// Callback container for operations that respond with file information
#[derive(Debug)]
pub struct ReplyEntry {
    reply: Box<dyn CallbackEntry>,
}

impl ReplyEntry {
    /// Create a ReplyEntry from the given boxed CallbackEntry.
    pub fn new(reply: impl CallbackEntry + 'static) -> Self {
        ReplyEntry { reply: Box::new(reply) }
    }
    /// Reply to a request with the given entry
    pub fn entry(mut self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.reply.entry(ttl, attr, generation);
    }
    default_error!();
}

/* ------ Attr ------ */

/// Custom callback handler for ReplyAttr
pub trait CallbackAttr: CallbackErr {
    /// Reply to a request with the given attribute
    fn attr(&mut self, ttl: &Duration, attr: &FileAttr);
}

/// Legacy callback handler for ReplyAttr
impl<S: ReplySender + Debug> CallbackAttr for Option<ReplyHandler<S>> {
    fn attr(&mut self, ttl: &Duration, attr: &FileAttr) {
        if let Some(handler) = self.take() {
            handler.attr(ttl, attr);
        }
    }
}

/// Callback container for operations that respond with file attributes
#[derive(Debug)]
pub struct ReplyAttr {
    reply: Box<dyn CallbackAttr>,
}

impl ReplyAttr {
    /// Create a ReplyAttr from the given boxed CallbackAttr.
    pub fn new(reply: impl CallbackAttr + 'static) -> Self {
        ReplyAttr { reply: Box::new(reply) }
    }
    /// Reply to a request with the given attribute
    pub fn attr(mut self, ttl: &Duration, attr: &FileAttr) {
        self.reply.attr(ttl, attr);
    }
    default_error!();
}

/* ------ XTimes ------ */

/// Custom callback handler for ReplyXTimes

#[cfg(target_os = "macos")]
pub trait CallbackXTimes: CallbackErr {
    /// Reply to a request with the given xtimes
    fn xtimes(&mut self, bkuptime: SystemTime, crtime: SystemTime);
}
#[cfg(target_os = "macos")]
/// Legacy callback handler for ReplyZ
impl<S: ReplySender + Debug> CallbackXTimes for Option<ReplyHandler<S>> {
    fn xtimes(&mut self, bkuptime: SystemTime, crtime: SystemTime) {
        if let Some(handler) = self.take() {
            handler.xtimes(XTimes { bkuptime, crtime });
        }
    }
}

/// Callback container for operations that respond with xtimes
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ReplyXTimes {
    reply: Box<dyn CallbackXTimes>,
}
#[cfg(target_os = "macos")]
impl ReplyXTimes {
    /// Create a ReplyXTimes from the given boxed CallbackXTimes.
    pub fn new(reply: impl CallbackXTimes + 'static) -> Self {
        ReplyXTimes { reply: Box::new(reply) }
    }
    /// Reply to a request with the given xtimes
    pub fn xtimes(mut self, bkuptime: SystemTime, crtime: SystemTime) {
        self.reply.xtimes(bkuptime, crtime);
    }
    default_error!();
}

/* ------ Open ------ */

/// Custom callback handler for ReplyOpen
pub trait CallbackOpen: CallbackErr {
    /// Reply to a request with the given open result
    fn opened(&mut self, fh: u64, flags: u32);

    /// Reply to a request with an opened backing id.  Call `ReplyOpen::open_backing()`` to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    fn opened_passthrough(&mut self, fh: u64, flags: u32, backing_id: &BackingId);
}

/// Legacy callback handler for ReplyOpen
impl<S: ReplySender + Debug> CallbackOpen for Option<ReplyHandler<S>> {
    fn opened(&mut self, fh: u64, flags: u32) {
        if let Some(handler) = self.take() {
            handler.opened(&Open {
                fh,
                flags,
                backing_id: None,
            });
        }
    }
    #[cfg(feature = "abi-7-40")]
    fn opened_passthrough(&mut self, fh: u64, flags: u32, backing_id: &BackingId) {
        if backing_id.id == 0 {
            log::error!("Attemped to re-use already-closed backing id");
        } else if let Some(handler) = self.take() {
            handler.opened(&Open {
                fh,
                flags: flags | FOPEN_PASSTHROUGH,
                backing_id: Some(backing_id.id),
            });
        }
    }
}

/// Custom backing handler for ReplyOpen
#[cfg(feature = "abi-7-40")]
pub trait CallbackBacking {
    /// Registers a fd for passthrough, returning a `Backing`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `ReplyOpen::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    fn open_backing(&self, fd: i32) -> io::Result<BackingId>;
    // Note: although an implementor of this trait may support `<F: AsRawFd>`, trait
    // must specify a concrete type `i32` for dyn compatibility.
}

/// Legacy backing handler for ReplyOpen
#[cfg(feature = "abi-7-40")]
impl<S: BackingSender + Debug> CallbackBacking for BackingHandler<S> {
    fn open_backing(&self, fd: i32) -> io::Result<BackingId> {
        // Note: although BackingHandler supports generic `<F: AsRawFd>`,
        // CallbackOpen requires a fixed type for this call.
        // `i32` is a valid concrete type for `AsRawFd`
        self.open_backing(fd)
    }
}

/// Callback container for operations that respond with file handles
#[derive(Debug)]
pub struct ReplyOpen {
    reply: Box<dyn CallbackOpen>,
    #[cfg(feature = "abi-7-40")]
    backer: Box<dyn CallbackBacking>,
}

impl ReplyOpen {
    /// Create a ReplyOpen from the given boxed CallbackOpen.
    pub fn new(
        reply: impl CallbackOpen + 'static, 
        #[cfg(feature = "abi-7-40")]
        backer: impl CallbackBacking + 'static
    ) -> Self {
        ReplyOpen {
            reply: Box::new(reply),
            #[cfg(feature = "abi-7-40")]
            backer: Box::new(backer),
        }
    }
    /// Reply to a request with the given file handle
    pub fn opened(mut self, fh: u64, flags: u32) {
        self.reply.opened(fh, flags);
    }

    /// Reply to a request with an opened backing id.  Call `ReplyOpen::open_backing()`` to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    pub fn opened_passthrough(mut self, fh: u64, flags: u32, backing_id: &BackingId) {
        self.reply.opened_passthrough(fh, flags, backing_id);
    }

    /// Registers a file for passthrough, returning a `BackingId`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `ReplyOpen::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    pub fn open_backing<F: AsRawFd>(&self, file: F) -> io::Result<BackingId> {
        let fd = file.as_raw_fd();
        // If the user passes in an single-owner `File`, this function must retain it long enough
        // to obtain a backing id. It can be dropped (closed) afterwards.
        self.backer.open_backing(fd)
    }
    default_error!();
}

/* ------ Write ------ */

/// Custom callback handler for ReplyWrite
pub trait CallbackWrite: CallbackErr {
    /// Reply to a request with the given open result
    fn written(&mut self, size: u32);
}

/// Legacy callback handler for ReplyZ
impl<S: ReplySender + Debug> CallbackWrite for Option<ReplyHandler<S>> {
    fn written(&mut self, size: u32) {
        if let Some(handler) = self.take() {
            handler.written(size);
        }
    }
}

/// Callback container for operations that respond with size of data written
#[derive(Debug)]
pub struct ReplyWrite {
    reply: Box<dyn CallbackWrite>,
}

impl ReplyWrite {
    /// Create a ReplyWrite from the given boxed CallbackWrite.
    pub fn new(reply: impl CallbackWrite + 'static) -> Self {
        ReplyWrite { reply: Box::new(reply) }
    }
    /// Reply to a request with the given open result
    pub fn written(mut self, size: u32) {
        self.reply.written(size);
    }
    default_error!();
}

/* ------ Statfs ------ */

/// Custom callback handler for ReplyStatfs
pub trait CallbackStatfs: CallbackErr {
    /// Reply to a request with filesystem information
    #[allow(clippy::too_many_arguments)]
    fn statfs(
        &mut self,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        bsize: u32,
        namelen: u32,
        frsize: u32,
    );
}

/// Legacy callback handler for ReplyStatfs
impl<S: ReplySender + Debug> CallbackStatfs for Option<ReplyHandler<S>> {
    fn statfs(
        &mut self,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        bsize: u32,
        namelen: u32,
        frsize: u32,
    ) {
        if let Some(handler) = self.take() {
            handler.statfs(&Statfs {
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
    }
}

/// Callback container for operations that respond with filesystem information
#[derive(Debug)]
pub struct ReplyStatfs {
    reply: Box<dyn CallbackStatfs>,
}

impl ReplyStatfs {
    /// Create a ReplyStatfs from the given boxed CallbackStatfs.
    pub fn new(reply: impl CallbackStatfs + 'static) -> Self {
        ReplyStatfs { reply: Box::new(reply) }
    }
    /// Reply to a request with the given open result
    #[allow(clippy::too_many_arguments)]
    pub fn statfs(
        mut self,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        bsize: u32,
        namelen: u32,
        frsize: u32,
    ) {
        self.reply
            .statfs(blocks, bfree, bavail, files, ffree, bsize, namelen, frsize);
    }
    default_error!();
}

/* ------ Create ------ */

/// Custom callback handler for ReplyCreate
pub trait CallbackCreate: CallbackErr {
    /// Reply to a request with the given entry and file handle
    fn created(&mut self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32);
}

/// Legacy callback handler for ReplyCreate
impl<S: ReplySender + Debug> CallbackCreate for Option<ReplyHandler<S>> {
    fn created(&mut self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32) {
        if let Some(handler) = self.take() {
            handler.created(
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
    }
}

/// Callback container for operations that respond with file handles and file information
#[derive(Debug)]
pub struct ReplyCreate {
    reply: Box<dyn CallbackCreate>,
}

impl ReplyCreate {
    /// Create a ReplyCreate from the given boxed CallbackCreate.
    pub fn new(reply: impl CallbackCreate + 'static) -> Self {
        ReplyCreate { reply: Box::new(reply) }
    }
    /// Reply to a request with the given entry and file handle
    pub fn created(
        mut self,
        ttl: &Duration,
        attr: &FileAttr,
        generation: u64,
        fh: u64,
        flags: u32,
    ) {
        self.reply.created(ttl, attr, generation, fh, flags);
    }
    default_error!();
}

/* ------ Lock ------ */

/// Custom callback handler for ReplyLock
pub trait CallbackLock: CallbackErr {
    /// Reply to a request with the given lock result
    fn locked(&mut self, start: u64, end: u64, typ: i32, pid: u32);
}

/// Legacy callback handler for ReplyLock
impl<S: ReplySender + Debug> CallbackLock for Option<ReplyHandler<S>> {
    fn locked(&mut self, start: u64, end: u64, typ: i32, pid: u32) {
        if let Some(handler) = self.take() {
            handler.locked(&Lock {
                start,
                end,
                typ,
                pid,
            });
        }
    }
}

/// Callback container for operations that respond with file locks
#[derive(Debug)]
pub struct ReplyLock {
    reply: Box<dyn CallbackLock>,
}

impl ReplyLock {
    /// Create a ReplyLock from the given boxed CallbackLock.
    pub fn new(reply: impl CallbackLock + 'static) -> Self {
        ReplyLock { reply: Box::new(reply) }
    }
    /// Reply to a request with the given lock result
    pub fn locked(mut self, start: u64, end: u64, typ: i32, pid: u32) {
        self.reply.locked(start, end, typ, pid);
    }
    default_error!();
}

/* ------ Bmap ------ */

/// Custom callback handler for ReplyBmap
pub trait CallbackBmap: CallbackErr {
    /// Reply to a request with the given block result
    fn bmap(&mut self, block: u64);
}

/// Legacy callback handler for ReplyBmap
impl<S: ReplySender + Debug> CallbackBmap for Option<ReplyHandler<S>> {
    fn bmap(&mut self, block: u64) {
        if let Some(handler) = self.take() {
            handler.bmap(block);
        }
    }
}

/// Callback container for operations that respond with bmap blocks
#[derive(Debug)]
pub struct ReplyBmap {
    reply: Box<dyn CallbackBmap>,
}

impl ReplyBmap {
    /// Create a ReplyBmap from the given boxed CallbackBmap.
    pub fn new(reply: impl CallbackBmap + 'static) -> Self {
        ReplyBmap { reply: Box::new(reply) }
    }
    /// Reply to a request with the given block result
    pub fn bmap(mut self, block: u64) {
        self.reply.bmap(block);
    }
    default_error!();
}

/* ------ Ioctl ------ */

/// Custom callback handler for ReplyIoctl
pub trait CallbackIoctl: CallbackErr {
    /// Reply to a request with the given data result
    fn ioctl(&mut self, result: i32, data: &[u8]);
}

/// Legacy callback handler for ReplyIoctl
impl<S: ReplySender + Debug> CallbackIoctl for Option<ReplyHandler<S>> {
    fn ioctl(&mut self, result: i32, data: &[u8]) {
        if let Some(handler) = self.take() {
            handler.ioctl(result, data);
        }
    }
}

/// Callback container for operations that respond with ioctl data
#[derive(Debug)]
pub struct ReplyIoctl {
    reply: Box<dyn CallbackIoctl>,
}

impl ReplyIoctl {
    /// Create a ReplyIoctl from the given boxed CallbackIoctl.
    pub fn new(reply: impl CallbackIoctl + 'static) -> Self {
        ReplyIoctl { reply: Box::new(reply) }
    }
    /// Reply to a request with the given data result
    pub fn ioctl(mut self, result: i32, data: &[u8]) {
        self.reply.ioctl(result, data);
    }
    default_error!();
}

/* ------ Poll ------ */

/// Custom callback handler for ReplyPoll
pub trait CallbackPoll: CallbackErr {
    /// Reply to a request with the given poll result
    fn poll(&mut self, revents: u32);
}

/// Legacy callback handler for ReplyPoll
impl<S: ReplySender + Debug> CallbackPoll for Option<ReplyHandler<S>> {
    fn poll(&mut self, revents: u32) {
        if let Some(handler) = self.take() {
            handler.poll(revents);
        }
    }
}

/// Callback container for operations that respond with poll events
#[derive(Debug)]
pub struct ReplyPoll {
    reply: Box<dyn CallbackPoll>,
}

impl ReplyPoll {
    /// Create a ReplyPoll from the given boxed CallbackPoll.
    pub fn new(reply: impl CallbackPoll + 'static) -> Self {
        ReplyPoll { reply: Box::new(reply) }
    }
    /// Reply to a request with the given poll result
    pub fn poll(mut self, revents: u32) {
        self.reply.poll(revents);
    }
    default_error!();
}

/* ------ DirentList ------ */

/// Custom callback handler for ReplyDirectory
pub trait CallbackDirectory: CallbackErr {
    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    fn add(&mut self, ino: u64, offset: i64, kind: FileType, name: &OsStr) -> bool;

    /// Reply to a request with the filled directory buffer
    fn ok(&mut self);
}

/// Legacy callback handler for ReplyDirectory
#[derive(Debug)]
pub(crate) struct DirectoryHandler<S: ReplySender + Debug> {
    buf: Option<EntListBuf>,
    reply: Option<ReplyHandler<S>>,
}

impl<S: ReplySender + Debug> DirectoryHandler<S> {
    pub fn new(max_size: usize, reply: ReplyHandler<S>) -> Self {
        DirectoryHandler {
            buf: Some(EntListBuf::new(max_size)),
            reply: Some(reply),
        }
    }
}
impl<S: ReplySender + Debug> CallbackErr for DirectoryHandler<S> {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.reply.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}
impl<S: ReplySender + Debug> CallbackDirectory for DirectoryHandler<S> {
    fn add(&mut self, ino: u64, offset: i64, kind: FileType, name: &OsStr) -> bool {
        if let Some(mut buf) = self.buf.take() {
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
            self.buf = Some(buf);
            res
        } else {
            true
        }
    }
    fn ok(&mut self) {
        if let Some(handler) = self.reply.take() {
            if let Some(buf) = self.buf.take() {
                handler.send_ll(&Response::new_directory(buf));
            }
        }
    }
}

/// Callback container for operations that respond with directory entries
#[derive(Debug)]
pub struct ReplyDirectory {
    reply: Box<dyn CallbackDirectory>,
}

impl ReplyDirectory {
    /// Create a ReplyDirectory from the given boxed CallbackDirectory.
    pub fn new(reply: impl CallbackDirectory + 'static) -> Self {
        ReplyDirectory { reply: Box::new(reply) }
    }
    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    pub fn add<T: AsRef<OsStr>>(&mut self, ino: u64, offset: i64, kind: FileType, name: T) -> bool {
        self.reply.add(ino, offset, kind, name.as_ref())
    }
    /// Reply to a request with the filled directory buffer
    pub fn ok(&mut self) {
        self.reply.ok();
    }
    default_error!();
}

/* ------ DirentPlusList ------ */

/// Custom callback handler for ReplyDirectoryPlus
#[cfg(feature = "abi-7-21")]
pub trait CallbackDirectoryPlus: CallbackErr {
    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    fn add(
        &mut self,
        ino: u64,
        offset: i64,
        name: &OsStr,
        ttl: &Duration,
        attr: &FileAttr,
        generation: u64,
    ) -> bool;

    /// Reply to a request with the filled directory buffer
    fn ok(&mut self);
}

/// Legacy callback handler for ReplyDirectory
#[derive(Debug)]
#[cfg(feature = "abi-7-21")]
pub(crate) struct DirectoryPlusHandler<S: ReplySender> {
    buf: Option<EntListBuf>,
    reply: Option<ReplyHandler<S>>,
    attr_ttl_override: bool,
}
#[cfg(feature = "abi-7-21")]
impl<S: ReplySender> DirectoryPlusHandler<S> {
    pub fn new(max_size: usize, reply: ReplyHandler<S>) -> Self {
        DirectoryPlusHandler {
            buf: Some(EntListBuf::new(max_size)),
            reply: Some(reply),
            attr_ttl_override: false,
        }
    }
    /// Disable attribute cacheing.
    pub fn attr_ttl_override(&mut self) {
        self.attr_ttl_override = true;
    }
}
#[cfg(feature = "abi-7-21")]
impl<S: ReplySender + Debug> CallbackErr for DirectoryPlusHandler<S> {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.reply.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}
#[cfg(feature = "abi-7-21")]
impl<S: ReplySender + Debug> CallbackDirectoryPlus for DirectoryPlusHandler<S> {
    fn add(
        &mut self,
        ino: u64,
        offset: i64,
        name: &OsStr,
        ttl: &Duration,
        attr: &FileAttr,
        generation: u64,
    ) -> bool {
        if let Some(mut buf) = self.buf.take() {
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
                    attr_valid: if self.attr_ttl_override {
                        0
                    } else {
                        ttl.as_secs()
                    },
                    entry_valid_nsec: ttl.subsec_nanos(),
                    attr_valid_nsec: if self.attr_ttl_override {
                        0
                    } else {
                        ttl.subsec_nanos()
                    },
                    attr: fuse_attr_from_attr(attr),
                },
                dirent: crate::ll::fuse_abi::fuse_dirent {
                    ino,
                    off: offset,
                    namelen,
                    typ: mode_from_kind_and_perm(attr.kind, 0) >> 12,
                },
            };
            let res = buf.push([header.as_bytes(), name.as_encoded_bytes()]);
            self.buf = Some(buf);
            res
        } else {
            true
        }
    }
    fn ok(&mut self) {
        if let Some(handler) = self.reply.take() {
            if let Some(buf) = self.buf.take() {
                handler.send_ll(&Response::new_directory(buf));
            }
        }
    }
}

/// Callback container for operations that respond with director entries and file information
#[cfg(feature = "abi-7-21")]
#[derive(Debug)]
pub struct ReplyDirectoryPlus {
    reply: Box<dyn CallbackDirectoryPlus>,
}
#[cfg(feature = "abi-7-21")]
impl ReplyDirectoryPlus {
    /// Create a ReplyDirectoryPlus from the given boxed CallbackDirectoryPlus.
    pub fn new(reply: impl CallbackDirectoryPlus + 'static) -> Self {
        ReplyDirectoryPlus { reply: Box::new(reply) }
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
        self.reply
            .add(ino, offset, name.as_ref(), ttl, attr, generation)
    }
    /// Reply to a request with the filled directory buffer
    pub fn ok(mut self) {
        self.reply.ok();
    }
    default_error!();
}

/* ------ Xattr ------ */

/// Custom callback handler for ReplyXattr
pub trait CallbackXattr: CallbackErr {
    /// Reply to a request with the size of the xattr.
    fn size(&mut self, size: u32);

    /// Reply to a request with the data in the xattr.
    fn data(&mut self, data: &[u8]);
}

/// Legacy callback handler for ReplyXattr
impl<S: ReplySender + Debug> CallbackXattr for Option<ReplyHandler<S>> {
    fn size(&mut self, size: u32) {
        if let Some(handler) = self.take() {
            handler.xattr_size(size);
        }
    }
    fn data(&mut self, data: &[u8]) {
        if let Some(handler) = self.take() {
            handler.xattr_data(data);
        }
    }
}

/// Callback container for operations that respond with extended file attributes
#[derive(Debug)]
pub struct ReplyXattr {
    reply: Box<dyn CallbackXattr>,
}

impl ReplyXattr {
    /// Create a ReplyXattr from the given boxed CallbackXattr.
    pub fn new(reply: impl CallbackXattr + 'static) -> Self {
        ReplyXattr { reply: Box::new(reply) }
    }
    /// Reply to a request with the size of the xattr.
    pub fn size(mut self, size: u32) {
        self.reply.size(size);
    }
    /// Reply to a request with the data in the xattr.
    pub fn data(mut self, data: &[u8]) {
        self.reply.data(data);
    }
    default_error!();
}

/* ------ Lseek ------ */

/// Custom callback handler for ReplyLseek
#[cfg(feature = "abi-7-24")]
pub trait CallbackLseek: CallbackErr {
    /// Reply to a request with the given offset
    fn offset(&mut self, offset: i64);
}

/// Legacy callback handler for ReplyLseek
#[cfg(feature = "abi-7-24")]
impl<S: ReplySender + Debug> CallbackLseek for Option<ReplyHandler<S>> {
    fn offset(&mut self, offset: i64) {
        if let Some(handler) = self.take() {
            handler.offset(offset);
        }
    }
}

/// Callback container for operations that respond with file offsets
#[derive(Debug)]
#[cfg(feature = "abi-7-24")]
pub struct ReplyLseek {
    reply: Box<dyn CallbackLseek>,
}

#[cfg(feature = "abi-7-24")]
impl ReplyLseek {
    /// Create a ReplyLseek from the given boxed CallbackLseek.
    pub fn new(reply: impl CallbackLseek + 'static) -> Self {
        ReplyLseek { reply: Box::new(reply) }
    }
    /// Reply to a request with the given offset
    pub fn offset(mut self, offset: i64) {
        self.reply.offset(offset);
    }
    default_error!();
}
