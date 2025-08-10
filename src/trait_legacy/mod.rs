//! The legacy, callback-based FUSE API.
//!
//! This module provides the `Filesystem` trait, which is the original,
//! callback-based API from the `fuse` crate. It is recommended to use the
//! `trait_sync` or `trait_async` APIs for new filesystems.
//!
//! ## Example
//!
//! ```rust,no_run
//! use fuser::{
//!     FileAttr, FileType, MountOption, ReplyAttr, ReplyData, ReplyEntry,
//!     ReplyDirectory, Request, trait_legacy::Filesystem,
//! };
//! use std::ffi::OsStr;
//! use std::time::{Duration, UNIX_EPOCH};
//!
//! const TTL: Duration = Duration::from_secs(1); // 1 second
//!
//! const HELLO_DIR_ATTR: FileAttr = FileAttr {
//!     ino: 1,
//!     size: 0,
//!     blocks: 0,
//!     atime: UNIX_EPOCH, // 1970-01-01 00:00:00
//!     mtime: UNIX_EPOCH,
//!     ctime: UNIX_EPOCH,
//!     crtime: UNIX_EPOCH,
//!     kind: FileType::Directory,
//!     perm: 0o755,
//!     nlink: 2,
//!     uid: 501,
//!     gid: 20,
//!     rdev: 0,
//!     flags: 0,
//!     blksize: 512,
//! };
//!
//! struct HelloFS;
//!
//! impl Filesystem for HelloFS {
//!     fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
//!         // ...
//!     }
//!
//!     fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
//!         // ...
//!     }
//!
//!     fn read(
//!         &mut self,
//!         _req: &Request,
//!         ino: u64,
//!         fh: u64,
//!         offset: i64,
//!         size: u32,
//!         reply: ReplyData,
//!     ) {
//!         // ...
//!     }
//!
//!     fn readdir(
//!         &mut self,
//!         _req: &Request,
//!         ino: u64,
//!         fh: u64,
//!         offset: i64,
//!         reply: ReplyDirectory,
//!     ) {
//!         // ...
//!     }
//! }
//!
//! fn main() {
//!     let mountpoint = std::env::args_os().nth(1).unwrap();
//!     let fs = HelloFS;
//!     fuser::mount2(fs.into(), mountpoint, &[MountOption::AutoUnmount]).unwrap();
//! }
//! ```

mod filesystem;
pub use filesystem::Filesystem;
#[cfg(feature = "abi-7-16")]
pub use crate::ll::fuse_abi::fuse_forget_one;

mod dispatch;
pub use dispatch::Request;

mod run;

mod callback;

// Structs
pub use callback::{
    ReplyEmpty,
    ReplyData,
    ReplyEntry,
    ReplyAttr,
    ReplyOpen,
    ReplyWrite,
    ReplyStatfs,
    ReplyCreate,
    ReplyLock,
    ReplyBmap,
    ReplyDirectory,
    ReplyXattr,
};
#[cfg(feature = "abi-7-11")]
pub use callback::ReplyIoctl;
#[cfg(feature = "abi-7-11")]
pub use callback::ReplyPoll;
#[cfg(feature = "abi-7-21")]
pub use callback::ReplyDirectoryPlus;
#[cfg(feature = "abi-7-24")]
pub use callback::ReplyLseek;
#[cfg(target_os = "macos")]
pub use callback::ReplyXTimes;

// Traits
pub use callback::{
    CallbackErr,
    CallbackOk,
    CallbackData,
    CallbackEntry,
    CallbackAttr,
    CallbackOpen,
    CallbackWrite,
    CallbackStatfs,
    CallbackCreate,
    CallbackLock,
    CallbackBmap,
    CallbackDirectory,
    CallbackXattr,
};
#[cfg(feature = "abi-7-11")]
pub use callback::CallbackIoctl;
#[cfg(feature = "abi-7-11")]
pub use callback::CallbackPoll;
#[cfg(feature = "abi-7-21")]
pub use callback::CallbackDirectoryPlus;
#[cfg(feature = "abi-7-24")]
pub use callback::CallbackLseek;
#[cfg(target_os = "macos")]
pub use callback::CallbackXTimes;

#[cfg(feature = "abi-7-11")]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
/// Contains a u64 value.
pub struct PollHandle(pub u64);

#[cfg(feature = "abi-7-40")]
#[derive(Debug)]
/// Contains a new backing id and the original file descriptor
pub struct BackingId {
    /// The owned file, which must be held until the application receives its filehandle.
    pub fd: std::fs::File,
    /// A backing id the kernel uses to address this file.
    pub backing_id: u32
}