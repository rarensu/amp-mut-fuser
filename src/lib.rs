//! FUSE userspace library implementation
//!
//! This is an improved rewrite of the FUSE userspace library (lowlevel interface) to fully take
//! advantage of Rust's architecture. The only thing we rely on in the real libfuse are mount
//! and unmount calls which are needed to establish a fd to talk to the kernel driver.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

/* ------ Modules ------ */

mod channel;
mod data;
mod ll;
mod mnt;
mod notify;
#[cfg(feature = "abi-7-40")]
mod passthrough;
mod reply;
mod request;
mod session;
/// Legacy Filesystem trait with callbacks
pub mod trait_legacy;

/* ------ Public Exports ------ */

pub use data::{FileAttr, FileType, KernelConfig};
pub use ll::fuse_abi::FUSE_ROOT_ID;
pub use ll::{Errno, TimeOrNow, fuse_abi::consts};
pub use mnt::mount_options::MountOption;
pub use notify::PollHandle;
#[cfg(feature = "abi-7-40")]
pub use passthrough::BackingId;
pub use request::RequestMeta;
pub use session::{BackgroundSession, Session, SessionACL, SessionUnmounter};

// Default trait is the Legacy `Filesystem` trait with `Reply` callbacks
pub use trait_legacy::{Filesystem, Request, fuse_forget_one};
#[allow(deprecated)]
pub use trait_legacy::{mount, mount2, spawn_mount, spawn_mount2};
#[cfg(feature = "abi-7-21")]
pub use trait_legacy::ReplyDirectoryPlus;
#[cfg(feature = "abi-7-24")]
pub use trait_legacy::ReplyLseek;
#[cfg(target_os = "macos")]
pub use trait_legacy::ReplyXTimes;
pub use trait_legacy::{
    ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyIoctl, ReplyLock, ReplyOpen, ReplyPoll, ReplyStatfs, ReplyWrite, ReplyXattr,
};
pub use trait_legacy::Notifier;