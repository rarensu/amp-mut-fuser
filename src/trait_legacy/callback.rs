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
    FileAttr, FileType,
    ll::{
        Errno,
        reply::{EntListBuf, Response, mode_from_kind_and_perm},
    },
    reply::{Entry, Lock, Open, ReplyHandler, Statfs},
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

/// Custom callback handler for any Reply
pub trait CallbackErr: Debug {
    /// Reply to a request with the given error code
    fn error(&mut self, err: c_int);
}

/// Legacy callback handler for any Reply
impl CallbackErr for Option<ReplyHandler> {
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
            self.handler.error(err)
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
impl CallbackOk for Option<ReplyHandler> {
    fn ok(&mut self) {
        if let Some(handler) = self.take() {
            handler.ok();
        }
    }
}

/// Callback container for operations that respond Ok
#[derive(Debug)]
pub struct ReplyEmpty {
    handler: Box<dyn CallbackOk>,
}

impl ReplyEmpty {
    /// Create a ReplyEmpty from the given boxed CallbackOk.
    pub fn new(handler: Box<dyn CallbackOk>) -> Self {
        ReplyEmpty { handler }
    }
    /// Reply to a request with nothing
    pub fn ok(mut self) {
        self.handler.ok();
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
impl CallbackData for Option<ReplyHandler> {
    fn data(&mut self, data: &[u8]) {
        if let Some(handler) = self.take() {
            handler.data(data);
        }
    }
}

/// Callback container for operations that respond with Bytes
#[derive(Debug)]
pub struct ReplyData {
    handler: Box<dyn CallbackData>,
}

impl ReplyData {
    /// Create a ReplyData from the given boxed CallbackData.
    pub fn new(handler: Box<dyn CallbackData>) -> Self {
        ReplyData { handler }
    }
    /// Reply to a request with the given data
    pub fn data(mut self, data: &[u8]) {
        self.handler.data(data);
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
impl CallbackEntry for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackEntry>,
}

impl ReplyEntry {
    /// Create a ReplyEntry from the given boxed CallbackEntry.
    pub fn new(handler: Box<dyn CallbackEntry>) -> Self {
        ReplyEntry { handler }
    }
    /// Reply to a request with the given entry
    pub fn entry(mut self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.handler.entry(ttl, attr, generation);
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
impl CallbackAttr for Option<ReplyHandler> {
    fn attr(&mut self, ttl: &Duration, attr: &FileAttr) {
        if let Some(handler) = self.take() {
            handler.attr(attr, ttl);
        }
    }
}

/// Callback container for operations that respond with file attributes
#[derive(Debug)]
pub struct ReplyAttr {
    handler: Box<dyn CallbackAttr>,
}

impl ReplyAttr {
    /// Create a ReplyAttr from the given boxed CallbackAttr.
    pub fn new(handler: Box<dyn CallbackAttr>) -> Self {
        ReplyAttr { handler }
    }
    /// Reply to a request with the given attribute
    pub fn attr(mut self, ttl: &Duration, attr: &FileAttr) {
        self.handler.attr(ttl, attr);
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
impl CallbackXTimes for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackXTimes>,
}
#[cfg(target_os = "macos")]
impl ReplyXTimes {
    /// Create a ReplyXTimes from the given boxed CallbackXTimes.
    pub fn new(handler: Box<dyn CallbackXTimes>) -> Self {
        ReplyXTimes { handler }
    }
    /// Reply to a request with the given xtimes
    pub fn xtimes(mut self, bkuptime: SystemTime, crtime: SystemTime) {
        self.handler.xtimes(bkuptime, crtime);
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

    /// Registers a fd for passthrough, returning a `Backing`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `ReplyOpen::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    fn open_backing(&self, fd: i32) -> io::Result<BackingId>;
    // Note: although an implementor of this trait may support `<F: AsRawFd>`, trait
    // must specify a concrete type `i32` for dyn compatibility.
}

/// Legacy callback handler for ReplyOpen
#[derive(Debug)]
pub(crate) struct OpenHandler {
    handler: Option<ReplyHandler>,
    #[cfg(feature = "abi-7-40")]
    backer: BackingHandler,
}

impl OpenHandler {
    #[cfg(feature = "abi-7-40")]
    pub fn new(handler: ReplyHandler, backer: BackingHandler) -> Self {
        OpenHandler {
            handler: Some(handler),
            backer,
        }
    }
    #[cfg(not(feature = "abi-7-40"))]
    pub fn new(handler: ReplyHandler) -> Self {
        OpenHandler {
            handler: Some(handler),
        }
    }
}
impl CallbackErr for OpenHandler {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.handler.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}
impl CallbackOpen for OpenHandler {
    fn opened(&mut self, fh: u64, flags: u32) {
        if let Some(handler) = self.handler.take() {
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
        } else if let Some(handler) = self.handler.take() {
            handler.opened(&Open {
                fh,
                flags: flags | FOPEN_PASSTHROUGH,
                backing_id: Some(backing_id.id),
            });
        }
    }
    #[cfg(feature = "abi-7-40")]
    fn open_backing(&self, fd: i32) -> io::Result<BackingId> {
        // Note: although BackingHandler supports generic `<F: AsRawFd>`,
        // CallbackOpen requires a fixed type for this call.
        // `i32` is a valid concrete type for `AsRawFd`
        self.backer.open_backing(fd)
    }
}

/// Callback container for operations that respond with file handles
#[derive(Debug)]
pub struct ReplyOpen {
    handler: Box<dyn CallbackOpen>,
}

impl ReplyOpen {
    /// Create a ReplyOpen from the given boxed CallbackOpen.
    pub fn new(handler: Box<dyn CallbackOpen>) -> Self {
        ReplyOpen { handler }
    }
    /// Reply to a request with the given file handle
    pub fn opened(mut self, fh: u64, flags: u32) {
        self.handler.opened(fh, flags);
    }

    /// Reply to a request with an opened backing id.  Call `ReplyOpen::open_backing()`` to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    pub fn opened_passthrough(mut self, fh: u64, flags: u32, backing_id: &BackingId) {
        self.handler.opened_passthrough(fh, flags, backing_id);
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
        self.handler.open_backing(fd)
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
impl CallbackWrite for Option<ReplyHandler> {
    fn written(&mut self, size: u32) {
        if let Some(handler) = self.take() {
            handler.written(size);
        }
    }
}

/// Callback container for operations that respond with size of data written
#[derive(Debug)]
pub struct ReplyWrite {
    handler: Box<dyn CallbackWrite>,
}

impl ReplyWrite {
    /// Create a ReplyWrite from the given boxed CallbackWrite.
    pub fn new(handler: Box<dyn CallbackWrite>) -> Self {
        ReplyWrite { handler }
    }
    /// Reply to a request with the given open result
    pub fn written(mut self, size: u32) {
        self.handler.written(size);
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
impl CallbackStatfs for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackStatfs>,
}

impl ReplyStatfs {
    /// Create a ReplyStatfs from the given boxed CallbackStatfs.
    pub fn new(handler: Box<dyn CallbackStatfs>) -> Self {
        ReplyStatfs { handler }
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
        self.handler
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
impl CallbackCreate for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackCreate>,
}

impl ReplyCreate {
    /// Create a ReplyCreate from the given boxed CallbackCreate.
    pub fn new(handler: Box<dyn CallbackCreate>) -> Self {
        ReplyCreate { handler }
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
        self.handler.created(ttl, attr, generation, fh, flags);
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
impl CallbackLock for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackLock>,
}

impl ReplyLock {
    /// Create a ReplyLock from the given boxed CallbackLock.
    pub fn new(handler: Box<dyn CallbackLock>) -> Self {
        ReplyLock { handler }
    }
    /// Reply to a request with the given lock result
    pub fn locked(mut self, start: u64, end: u64, typ: i32, pid: u32) {
        self.handler.locked(start, end, typ, pid);
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
impl CallbackBmap for Option<ReplyHandler> {
    fn bmap(&mut self, block: u64) {
        if let Some(handler) = self.take() {
            handler.bmap(block);
        }
    }
}

/// Callback container for operations that respond with bmap blocks
#[derive(Debug)]
pub struct ReplyBmap {
    handler: Box<dyn CallbackBmap>,
}

impl ReplyBmap {
    /// Create a ReplyBmap from the given boxed CallbackBmap.
    pub fn new(handler: Box<dyn CallbackBmap>) -> Self {
        ReplyBmap { handler }
    }
    /// Reply to a request with the given block result
    pub fn bmap(mut self, block: u64) {
        self.handler.bmap(block);
    }
    default_error!();
}

/* ------ Ioctl ------ */

#[cfg(feature = "abi-7-11")]
/// Custom callback handler for ReplyIoctl
pub trait CallbackIoctl: CallbackErr {
    /// Reply to a request with the given data result
    fn ioctl(&mut self, result: i32, data: &[u8]);
}

#[cfg(feature = "abi-7-11")]
/// Legacy callback handler for ReplyIoctl
impl CallbackIoctl for Option<ReplyHandler> {
    fn ioctl(&mut self, result: i32, data: &[u8]) {
        if let Some(handler) = self.take() {
            handler.ioctl(result, data);
        }
    }
}

#[cfg(feature = "abi-7-11")]
/// Callback container for operations that respond with ioctl data
#[derive(Debug)]
pub struct ReplyIoctl {
    handler: Box<dyn CallbackIoctl>,
}

#[cfg(feature = "abi-7-11")]
impl ReplyIoctl {
    /// Create a ReplyIoctl from the given boxed CallbackIoctl.
    pub fn new(handler: Box<dyn CallbackIoctl>) -> Self {
        ReplyIoctl { handler }
    }
    /// Reply to a request with the given data result
    pub fn ioctl(mut self, result: i32, data: &[u8]) {
        self.handler.ioctl(result, data);
    }
    default_error!();
}

/* ------ Poll ------ */

#[cfg(feature = "abi-7-11")]
/// Custom callback handler for ReplyPoll
pub trait CallbackPoll: CallbackErr {
    /// Reply to a request with the given poll result
    fn poll(&mut self, revents: u32);
}

#[cfg(feature = "abi-7-11")]
/// Legacy callback handler for ReplyPoll
impl CallbackPoll for Option<ReplyHandler> {
    fn poll(&mut self, revents: u32) {
        if let Some(handler) = self.take() {
            handler.poll(revents);
        }
    }
}

#[cfg(feature = "abi-7-11")]
/// Callback container for operations that respond with poll events
#[derive(Debug)]
pub struct ReplyPoll {
    handler: Box<dyn CallbackPoll>,
}

#[cfg(feature = "abi-7-11")]
impl ReplyPoll {
    /// Create a ReplyPoll from the given boxed CallbackPoll.
    pub fn new(handler: Box<dyn CallbackPoll>) -> Self {
        ReplyPoll { handler }
    }
    /// Reply to a request with the given poll result
    pub fn poll(mut self, revents: u32) {
        self.handler.poll(revents);
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
pub(crate) struct DirectoryHandler {
    buf: Option<EntListBuf>,
    handler: Option<ReplyHandler>,
}
impl DirectoryHandler {
    pub fn new(max_size: usize, handler: ReplyHandler) -> Self {
        DirectoryHandler {
            buf: Some(EntListBuf::new(max_size)),
            handler: Some(handler),
        }
    }
}
impl CallbackErr for DirectoryHandler {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.handler.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}
impl CallbackDirectory for DirectoryHandler {
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
        if let Some(handler) = self.handler.take() {
            if let Some(buf) = self.buf.take() {
                handler.send_ll(&Response::new_directory(buf));
            }
        }
    }
}

/// Callback container for operations that respond with directory entries
#[derive(Debug)]
pub struct ReplyDirectory {
    handler: Box<dyn CallbackDirectory>,
}

impl ReplyDirectory {
    /// Create a ReplyDirectory from the given boxed CallbackDirectory.
    pub fn new(handler: Box<dyn CallbackDirectory>) -> Self {
        ReplyDirectory { handler }
    }
    /// Add an entry to the directory reply buffer. Returns true if the buffer is full.
    /// A transparent offset value can be provided for each entry. The kernel uses these
    /// value to request the next entries in further readdir calls
    #[must_use]
    pub fn add<T: AsRef<OsStr>>(&mut self, ino: u64, offset: i64, kind: FileType, name: T) -> bool {
        self.handler.add(ino, offset, kind, name.as_ref())
    }
    /// Reply to a request with the filled directory buffer
    pub fn ok(mut self) {
        self.handler.ok();
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
pub(crate) struct DirectoryPlusHandler {
    buf: Option<EntListBuf>,
    handler: Option<ReplyHandler>,
    attr_ttl_override: bool,
}
#[cfg(feature = "abi-7-21")]
impl DirectoryPlusHandler {
    pub fn new(max_size: usize, handler: ReplyHandler) -> Self {
        DirectoryPlusHandler {
            buf: Some(EntListBuf::new(max_size)),
            handler: Some(handler),
            attr_ttl_override: false,
        }
    }
    /// Disable attribute cacheing.
    pub fn attr_ttl_override(&mut self) {
        self.attr_ttl_override = true;
    }
}
#[cfg(feature = "abi-7-21")]
impl CallbackErr for DirectoryPlusHandler {
    fn error(&mut self, err: c_int) {
        if let Some(handler) = self.handler.take() {
            handler.error(Errno::from_i32(err));
        }
    }
}
#[cfg(feature = "abi-7-21")]
impl CallbackDirectoryPlus for DirectoryPlusHandler {
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
        if let Some(handler) = self.handler.take() {
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
    handler: Box<dyn CallbackDirectoryPlus>,
}
#[cfg(feature = "abi-7-21")]
impl ReplyDirectoryPlus {
    /// Create a ReplyDirectoryPlus from the given boxed CallbackDirectoryPlus.
    pub fn new(handler: Box<dyn CallbackDirectoryPlus>) -> Self {
        ReplyDirectoryPlus { handler }
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
        self.handler
            .add(ino, offset, name.as_ref(), ttl, attr, generation)
    }
    /// Reply to a request with the filled directory buffer
    pub fn ok(mut self) {
        self.handler.ok();
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
impl CallbackXattr for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackXattr>,
}

impl ReplyXattr {
    /// Create a ReplyXattr from the given boxed CallbackXattr.
    pub fn new(handler: Box<dyn CallbackXattr>) -> Self {
        ReplyXattr { handler }
    }
    /// Reply to a request with the size of the xattr.
    pub fn size(mut self, size: u32) {
        self.handler.size(size);
    }
    /// Reply to a request with the data in the xattr.
    pub fn data(mut self, data: &[u8]) {
        self.handler.data(data);
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
impl CallbackLseek for Option<ReplyHandler> {
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
    handler: Box<dyn CallbackLseek>,
}

#[cfg(feature = "abi-7-24")]
impl ReplyLseek {
    /// Create a ReplyLseek from the given boxed CallbackLseek.
    pub fn new(handler: Box<dyn CallbackLseek>) -> Self {
        ReplyLseek { handler }
    }
    /// Reply to a request with the given offset
    pub fn offset(mut self, offset: i64) {
        self.handler.offset(offset);
    }
    default_error!();
}
