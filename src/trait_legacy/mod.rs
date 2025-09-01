mod filesystem;
pub use crate::ll::fuse_abi::fuse_forget_one;
pub use filesystem::Filesystem;

mod mount;
#[allow(deprecated)]
pub use mount::{mount, mount2, spawn_mount, spawn_mount2};

/* ------ Additional imports for convenience ------ */
use crate::{KernelConfig, TimeOrNow};
use crate::notify::PollHandle;