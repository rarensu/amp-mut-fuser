//!
//! A library for presenting a FUSE filesystem. This is a rewrite of the FUSE userspace library
//! (libfuse) in Rust.
//!
//! The main entry point for this library is the [`mount2`] function, which takes a filesystem
//! and mount options and handles the communication with the kernel.
//!
//! This library also provides a `Request` object that includes the details of a filesystem
//! operation from the kernel (e.g. `read`, `write`, `lookup`).
//!
//! Filesystems are implemented by defining a struct and implementing one of the `Filesystem`
//! traits. The traits have a method for each possible FUSE operation. When the kernel sends a
//! FUSE request to the userspace daemon, the library will call the appropriate method on the
//! `Filesystem` object.
//!
//! To create a FUSE filesystem, you need to:
//!
//! 1. Create a struct that will hold the state of your filesystem.
//! 2. Implement the `fuser::trait_sync::Filesystem` or `fuser::trait_async::Filesystem` trait for your struct.
//! 3. Call the `fuser::mount2` function with an instance of your struct.
//!
//! ## Synchronous Filesystem Example
//!
//! ```rust,no_run
//! use fuser::{
//!     FileAttr, FileType, MountOption, ReplyAttr, ReplyEntry, ReplyDirectory, Request,
//!     trait_sync::Filesystem, Errno,
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
//!     fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) -> Result<(), Errno> {
//!         // ...
//!         Ok(())
//!     }
//!
//!     fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) -> Result<(), Errno> {
//!         // ...
//!         Ok(())
//!     }
//!
//!     fn readdir(
//!         &mut self,
//!         _req: &Request,
//!         ino: u64,
//!         fh: u64,
//!         offset: i64,
//!         reply: ReplyDirectory,
//!     ) -> Result<(), Errno> {
//!         // ...
//!         Ok(())
//!     }
//! }
//!
//! fn main() {
//!     let mountpoint = std::env::args_os().nth(1).unwrap();
//!     let fs = HelloFS;
//!     fuser::mount2(fs.into(), mountpoint, &[MountOption::AutoUnmount]).unwrap();
//! }
//! ```
//!
//! ## Asynchronous Filesystem Example
//!
//! ```rust,no_run
//! use fuser::{
//!     FileAttr, FileType, MountOption, ReplyAttr, ReplyEntry, ReplyDirectory, Request,
//!     trait_async::Filesystem, Errno,
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
//! #[async_trait::async_trait]
//! impl Filesystem for HelloFS {
//!     type Runtime = fuser::trait_async::run::Tokio;
//!
//!     async fn lookup(&self, _req: &Request<'_>, parent: u64, name: &OsStr) -> Result<FileAttr, Errno> {
//!         // ...
//!         Ok(HELLO_DIR_ATTR)
//!     }
//!
//!     async fn getattr(&self, _req: &Request<'_>, ino: u64) -> Result<(FileAttr, Duration), Errno> {
//!         // ...
//!         Ok((HELLO_DIR_ATTR, TTL))
//!     }
//!
//!     async fn readdir(
//!         &self,
//!         _req: &Request<'_>,
//!         ino: u64,
//!         fh: u64,
//!         offset: i64,
//!     ) -> Result<Vec<(u64, FileType, String)>, Errno> {
//!         // ...
//!         Ok(vec![])
//!     }
//! }
//!
//! fn main() {
//!     let mountpoint = std::env::args_os().nth(1).unwrap();
//!     let fs = HelloFS;
//!     fuser::mount2(fs.into(), mountpoint, &[MountOption::AutoUnmount]).unwrap();
//! }
//! ```
//!
//! ## Crate features
//!
//! The following crate features can be turned on:
//!
//! * **`async_std`**: Enables the use of the `async-std` runtime for asynchronous filesystems.
//! * **`tokio`**: Enables the use of the `tokio` runtime for asynchronous filesystems.
//!
#![warn(
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]

pub use any::{AnyFS, MountOption};
pub use channel::Channel;
pub use reply::{
    Reply, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyIoctl, ReplyLock, ReplyLseek, ReplyOpen, ReplyPoll, ReplyStatfs, ReplyWrite, ReplyXattr,
};
pub use request::Request;
pub use session::{Session, SessionConfig};

pub mod any;
mod channel;
pub mod container;
#[macro_use]
pub mod ll;
pub mod mnt;
mod notify;
mod reply;
mod request;
mod session;
pub mod trait_async;
pub mod trait_legacy;
pub mod trait_sync;

// TODO: Use correct mount function for each OS
// TODO: Check if all functions are using correct integer types
// TODO: Most functions of the Filesystem trait are not yet implemented

/// Mounts the given filesystem to the given mountpoint.
///
/// This function will block until the filesystem is unmounted.
///
/// The `mount` function will not return on success. Instead, the process will exit with a status
/// code of 0. On failure, the function will return an `Err` with an error message.
///
/// The `mount` function will automatically handle the `fork` and `daemonize` options.
///
/// # Panics
///
/// This function will panic if the `mountpoint` argument is not a valid path.
pub fn mount<FS: 'static + Send + Sync>(
    filesystem: FS,
    mountpoint: impl AsRef<std::path::Path>,
    options: &[MountOption],
) -> std::io::Result<()>
where
    AnyFS: From<FS>,
{
    let mut session = Session::new(filesystem.into(), mountpoint.as_ref(), options)?;
    session.run()
}

/// Mounts the given filesystem to the given mountpoint. This is a more flexible version of the
/// [`mount`] function.
///
/// This function will block until the filesystem is unmounted.
///
/// The `mount2` function will return a `Result` that indicates whether the filesystem was mounted
/// successfully. On success, the function will return a [`channel::Channel`]. The channel can be
// used to communicate with the filesystem.
///
/// The `mount2` function will not automatically handle the `fork` and `daemonize` options. These
/// options must be handled by the caller.
///
/// # Panics
///
/// This function will panic if the `mountpoint` argument is not a valid path.
pub fn mount2<FS: 'static + Send + Sync>(
    filesystem: FS,
    mountpoint: impl AsRef<std::path::Path>,
    options: &[MountOption],
) -> std::io::Result<Channel>
where
    AnyFS: From<FS>,
{
    let mut session = Session::new(filesystem.into(), mountpoint.as_ref(), options)?;
    session.run_once()
}
