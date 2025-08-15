mod filesystem;
#[cfg(feature = "abi-7-16")]
pub use crate::ll::fuse_abi::fuse_forget_one;
pub use filesystem::Filesystem;

mod dispatch;
pub use dispatch::Request;

mod run;

mod callback;

/* ------ Structs ------ */
#[cfg(feature = "abi-7-21")]
pub use callback::ReplyDirectoryPlus;
#[cfg(feature = "abi-7-11")]
pub use callback::ReplyIoctl;
#[cfg(feature = "abi-7-24")]
pub use callback::ReplyLseek;
#[cfg(feature = "abi-7-11")]
pub use callback::ReplyPoll;
#[cfg(target_os = "macos")]
pub use callback::ReplyXTimes;
pub use callback::{
    ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyLock, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
};
/* ------ Traits ------ */
#[cfg(feature = "abi-7-21")]
pub use callback::CallbackDirectoryPlus;
#[cfg(feature = "abi-7-11")]
pub use callback::CallbackIoctl;
#[cfg(feature = "abi-7-24")]
pub use callback::CallbackLseek;
#[cfg(feature = "abi-7-11")]
pub use callback::CallbackPoll;
#[cfg(target_os = "macos")]
pub use callback::CallbackXTimes;
pub use callback::{
    CallbackAttr, CallbackBmap, CallbackCreate, CallbackData, CallbackDirectory, CallbackEntry,
    CallbackErr, CallbackLock, CallbackOk, CallbackOpen, CallbackStatfs, CallbackWrite,
    CallbackXattr,
};

/*
#[cfg(feature = "abi-7-11")]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
/// Contains a u64 value.
pub struct PollHandle(pub u64);
*/
#[cfg(feature = "abi-7-11")]
use crate::notify::PollHandle;

/*
#[cfg(feature = "abi-7-40")]
#[derive(Debug)]
/// Contains a new backing id and the original file descriptor
pub struct BackingId {
    /// The owned file, which must be held until the application receives its filehandle.
    pub fd: std::fs::File,
    /// A backing id the kernel uses to address this file.
    pub backing_id: u32,
}
*/
#[cfg(feature = "abi-7-40")]
use crate::passthrough::BackingId;