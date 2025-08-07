mod filesystem;
pub use filesystem::Filesystem;
pub use filesystem::fuse_forget_one;

mod request;
pub use request::Request;

mod reply;
pub use reply::{
    // Structs
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
    // Traits
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
pub use reply::{ReplyXTimes, CallbackXTimes};


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