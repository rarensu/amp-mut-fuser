use libc::c_int;
use std::convert::AsRef;
use std::ffi::OsStr;
use std::fmt::Debug;
#[cfg(feature = "abi-7-40")]
use std::fs::File;
use std::time::Duration;

#[cfg(target_os = "macos")]
use std::time::SystemTime;

use crate::{FileAttr, FileType};

use super::BackingId;

/* ------ Err ------ */

/// Custom callback handler for any Reply
pub trait CallbackErr: Debug {
    /// Reply to a request with the given error code
    fn error(&mut self, err: c_int);
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

/// Callback for operations that respond Ok
#[derive(Debug)]
pub struct ReplyEmpty {
    handler: Box<dyn CallbackOk>
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

/// Callback for operations that respond with Bytes
#[derive(Debug)]
pub struct ReplyData {
    handler: Box<dyn CallbackData>
}

impl ReplyData {
    /// Create a ReplyData from the given boxed CallbackData.
    pub fn new(handler: Box<dyn CallbackData>) -> Self {
        ReplyData { handler }
    }
    /// Reply to a request with the given data
    pub fn data(mut self, data: &[u8]){
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

/// Callback for operations that respond with file information
#[derive(Debug)]
pub struct ReplyEntry {
    handler: Box<dyn CallbackEntry>
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

/// Callback for operations that respond with file attributes
#[derive(Debug)]
pub struct ReplyAttr {
    handler: Box<dyn CallbackAttr>
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

/// Callback for operations that respond with xtimes
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct ReplyXTimes {
    handler: Box<dyn CallbackXTimes>
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

    /// Registers a fd for passthrough, returning a `BackingId`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `ReplyOpen::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    fn open_backing(&mut self, fd: File) -> std::io::Result<BackingId>;
}

/// Callback for operations that respond with file handles
#[derive(Debug)]
pub struct ReplyOpen {
    handler: Box<dyn CallbackOpen>
}

impl ReplyOpen {
    /// Create a ReplyOpen from the given boxed CallbackOpen.
    pub fn new(handler: Box<dyn CallbackOpen>) -> Self {
        ReplyOpen { handler }
    }
    /// Reply to a request with the given file handle
    pub fn opened(mut self, fh: u64, flags: u32){
        self.handler.opened(fh, flags);
    }

    /// Reply to a request with an opened backing id.  Call `ReplyOpen::open_backing()`` to get one of
    /// these.
    #[cfg(feature = "abi-7-40")]
    pub fn opened_passthrough(mut self, fh: u64, flags: u32, backing_id: &BackingId){
        self.handler.opened_passthrough(fh, flags, backing_id);
    }

    /// Registers a fd for passthrough, returning a `BackingId`.  Once you have the backing ID,
    /// you can pass it as the 3rd parameter of `ReplyOpen::opened_passthrough()`.  This is done in
    /// two separate steps because it may make sense to reuse backing IDs (to avoid having to
    /// repeatedly reopen the underlying file or potentially keep thousands of fds open).
    #[cfg(feature = "abi-7-40")]
    pub fn open_backing(&mut self, fd: File) -> std::io::Result<BackingId>{
        self.handler.open_backing(fd)
    }
}

/* ------ Write ------ */

/// Custom callback handler for ReplyWrite
pub trait CallbackWrite: CallbackErr {
    /// Reply to a request with the given open result
    fn written(&mut self, size: u32);
}

/// Callback for operations that respond with size of data written
#[derive(Debug)]
pub struct ReplyWrite {
    handler: Box<dyn CallbackWrite>
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

/// Callback for operations that respond with filesystem information
#[derive(Debug)]
pub struct ReplyStatfs {
    handler: Box<dyn CallbackStatfs>
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
        self.handler.statfs(blocks, bfree, bavail, files, ffree, bsize, namelen, frsize);
    }
    default_error!();
}

/* ------ Create ------ */

/// Custom callback handler for ReplyCreate
pub trait CallbackCreate: CallbackErr {
    /// Reply to a request with the given entry and file handle
    fn created(&mut self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32);
}

/// Callback for operations that respond with file handles and file information
#[derive(Debug)]
pub struct ReplyCreate {
    handler: Box<dyn CallbackCreate>
}

impl ReplyCreate {
    /// Create a ReplyCreate from the given boxed CallbackCreate.
    pub fn new(handler: Box<dyn CallbackCreate>) -> Self {
        ReplyCreate { handler }
    }
    /// Reply to a request with the given entry and file handle
    pub fn created(mut self, ttl: &Duration, attr: &FileAttr, generation: u64, fh: u64, flags: u32) {
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

/// Callback for operations that respond with file locks
#[derive(Debug)]
pub struct ReplyLock {
    handler: Box<dyn CallbackLock>
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

/// Callback for operations that respond with bmap blocks
#[derive(Debug)]
pub struct ReplyBmap {
    handler: Box<dyn CallbackBmap>
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

/// Custom callback handler for ReplyIoctl
pub trait CallbackIoctl: CallbackErr {
    /// Reply to a request with the given data result
    fn ioctl(&mut self, result: i32, data: &[u8]);
}

/// Callback for operations that respond with ioctl data
#[derive(Debug)]
pub struct ReplyIoctl {
    handler: Box<dyn CallbackIoctl>
}

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

/// Custom callback handler for ReplyPoll
pub trait CallbackPoll: CallbackErr {
    /// Reply to a request with the given poll result
    fn poll(&mut self, revents: u32);
}

/// Callback for operations that respond with poll events
#[derive(Debug)]
pub struct ReplyPoll {
    handler: Box<dyn CallbackPoll>
}

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

/// Callback for operations that respond with directory entries
#[derive(Debug)]
pub struct ReplyDirectory {
    handler: Box<dyn CallbackDirectory>
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

/// Callback for operations that respond with director entries and file information
#[derive(Debug)]
pub struct ReplyDirectoryPlus {
    handler: Box<dyn CallbackDirectoryPlus>
}

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
        self.handler.add(ino, offset, name.as_ref(), ttl, attr, generation)
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

/// Callback for operations that respond with extended file attributes
#[derive(Debug)]
pub struct ReplyXattr {
    handler: Box<dyn CallbackXattr>
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
pub trait CallbackLseek: CallbackErr {
    /// Reply to a request with the given offset
    fn offset(&mut self, offset: i64);
}

/// Callback for operations that respond with file offsets
#[derive(Debug)]
pub struct ReplyLseek {
    handler: Box<dyn CallbackLseek>
}

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