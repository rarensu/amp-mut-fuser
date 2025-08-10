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
    ReplyIoctl,
    ReplyPoll,
    ReplyDirectory,
    ReplyDirectoryPlus,
    ReplyXattr,
    ReplyLseek,
};
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
    CallbackIoctl,
    CallbackPoll,
    CallbackDirectory,
    CallbackDirectoryPlus,
    CallbackXattr,
    CallbackLseek,
};
#[cfg(target_os = "macos")]
pub use callback::CallbackXTimes;


#[derive(Copy, Clone, Eq, PartialEq, Debug)]
/// Contains a u64 value.
pub struct PollHandle(pub u64);

use std::fs::File;

#[derive(Debug)]
/// Contains a new backing id and the original file descriptor
pub struct BackingId {
    /// The owned file, which must be held until the application receives its filehandle.
    pub fd: File,
    /// A backing id the kernel uses to address this file.
    pub backing_id: u32
}