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