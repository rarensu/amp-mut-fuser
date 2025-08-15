//! FUSE userspace library implementation
//!
//! This is an improved rewrite of the FUSE userspace library (lowlevel interface) to fully take
//! advantage of Rust's architecture. The only thing we rely on in the real libfuse are mount
//! and unmount calls which are needed to establish a fd to talk to the kernel driver.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

use log::warn;
use mnt::mount_options::parse_options_from_args;
#[cfg(feature = "serializable")]
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::io;
use std::path::Path;
#[cfg(feature = "abi-7-23")]
use std::time::Duration;
use std::{convert::AsRef, io::ErrorKind};

pub use crate::ll::fuse_abi::FUSE_ROOT_ID;
use crate::ll::fuse_abi::consts::*;
pub use crate::ll::{Errno, TimeOrNow, fuse_abi::consts};
use crate::mnt::mount_options::check_option_conflicts;
use crate::session::MAX_WRITE_SIZE;
#[cfg(feature = "abi-7-16")]
pub use ll::fuse_abi::fuse_forget_one;
pub use mnt::mount_options::MountOption;
#[cfg(feature = "abi-7-11")]
pub use notify::{NotificationHandler, PollHandler};
#[cfg(feature = "abi-7-11")]
pub use trait_legacy::ReplyIoctl;
#[cfg(feature = "abi-7-11")]
pub use trait_legacy::ReplyDirectoryPlus;
#[cfg(feature = "abi-7-24")]
pub use trait_legacy::ReplyLseek;
#[cfg(feature = "abi-7-40")]
pub use passthrough::BackingId;
#[cfg(feature = "abi-7-11")]
pub use trait_legacy::ReplyPoll;
#[cfg(target_os = "macos")]
pub use trait_legacy::ReplyXTimes;
pub use trait_legacy::{
    ReplyAttr, ReplyData, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyXattr,
    ReplyBmap, ReplyCreate, ReplyDirectory, ReplyLock, ReplyStatfs, ReplyWrite,
};
pub use request::RequestMeta;
pub use reply::{FileAttr, FileType};
pub use session::{BackgroundSession, Session, SessionACL, SessionUnmounter};
#[cfg(feature = "abi-7-28")]
use std::cmp::max;
#[cfg(feature = "abi-7-13")]
use std::cmp::min;

mod channel;
mod ll;
mod mnt;
#[cfg(feature = "abi-7-11")]
mod notify;
#[cfg(feature = "abi-7-40")]
mod passthrough;
mod reply;
mod request;
mod session;

/// Legacy Filesystem trait with callbacks
pub mod trait_legacy;
pub use trait_legacy::{Filesystem, Request};

/// We generally support async reads
#[cfg(all(not(target_os = "macos"), not(feature = "abi-7-10")))]
const INIT_FLAGS: u64 = FUSE_ASYNC_READ;
#[cfg(all(not(target_os = "macos"), feature = "abi-7-10"))]
const INIT_FLAGS: u64 = FUSE_ASYNC_READ | FUSE_BIG_WRITES;
// TODO: Add FUSE_EXPORT_SUPPORT

/// On macOS, we additionally support case insensitiveness, volume renames and xtimes
/// TODO: we should eventually let the filesystem implementation decide which flags to set
#[cfg(target_os = "macos")]
const INIT_FLAGS: u64 = FUSE_ASYNC_READ | FUSE_CASE_INSENSITIVE | FUSE_VOL_RENAME | FUSE_XTIMES;
// TODO: Add FUSE_EXPORT_SUPPORT and FUSE_BIG_WRITES (requires ABI 7.10)

const fn default_init_flags(#[allow(unused_variables)] capabilities: u64) -> u64 {
    #[cfg(not(feature = "abi-7-28"))]
    {
        INIT_FLAGS
    }

    #[cfg(feature = "abi-7-28")]
    {
        let mut flags = INIT_FLAGS;
        if capabilities & FUSE_MAX_PAGES != 0 {
            flags |= FUSE_MAX_PAGES;
        }
        flags
    }
}

/// Configuration of the fuse kernel module connection
#[derive(Debug)]
pub struct KernelConfig {
    capabilities: u64,
    requested: u64,
    max_readahead: u32,
    max_max_readahead: u32,
    #[cfg(feature = "abi-7-13")]
    max_background: u16,
    #[cfg(feature = "abi-7-13")]
    congestion_threshold: Option<u16>,
    max_write: u32,
    #[cfg(feature = "abi-7-23")]
    time_gran: Duration,
    #[cfg(feature = "abi-7-40")]
    max_stack_depth: u32,
}

impl KernelConfig {
    fn new(capabilities: u64, max_readahead: u32) -> Self {
        Self {
            capabilities,
            requested: default_init_flags(capabilities),
            max_readahead,
            max_max_readahead: max_readahead,
            #[cfg(feature = "abi-7-13")]
            max_background: 16,
            #[cfg(feature = "abi-7-13")]
            congestion_threshold: None,
            // use a max write size that fits into the session's buffer
            max_write: MAX_WRITE_SIZE as u32,
            // 1ns means nano-second granularity.
            #[cfg(feature = "abi-7-23")]
            time_gran: Duration::new(0, 1),
            #[cfg(feature = "abi-7-40")]
            max_stack_depth: 0,
        }
    }

    /// Set the maximum stacking depth of the filesystem
    ///
    /// This has to be at least 1 to support passthrough to backing files.  Setting this to 0 (the
    /// default) effectively disables support for passthrough.
    ///
    /// With max_stack_depth > 1, the backing files can be on a stacked fs (e.g. overlayfs)
    /// themselves and with max_stack_depth == 1, this FUSE filesystem can be stacked as the
    /// underlying fs of a stacked fs (e.g. overlayfs).
    ///
    /// The kernel currently has a hard maximum value of 2.  Anything higher won't work.
    ///
    /// On success, returns the previous value.  On error, returns the nearest value which will succeed.
    #[cfg(feature = "abi-7-40")]
    pub fn set_max_stack_depth(&mut self, value: u32) -> Result<u32, u32> {
        // https://lore.kernel.org/linux-fsdevel/CAOYeF9V_n93OEF_uf0Gwtd=+da0ReX8N2aaT6RfEJ9DPvs8O2w@mail.gmail.com/
        const FILESYSTEM_MAX_STACK_DEPTH: u32 = 2;

        if value > FILESYSTEM_MAX_STACK_DEPTH {
            return Err(FILESYSTEM_MAX_STACK_DEPTH);
        }

        let previous = self.max_stack_depth;
        self.max_stack_depth = value;
        Ok(previous)
    }

    /// Set the timestamp granularity
    ///
    /// Must be a power of 10 nanoseconds. i.e. 1s, 0.1s, 0.01s, 1ms, 0.1ms...etc
    ///
    /// On success returns the previous value. On error returns the nearest value which will succeed
    #[cfg(feature = "abi-7-23")]
    pub fn set_time_granularity(&mut self, value: Duration) -> Result<Duration, Duration> {
        if value.as_nanos() == 0 {
            return Err(Duration::new(0, 1));
        }
        if value.as_secs() > 1 || (value.as_secs() == 1 && value.subsec_nanos() > 0) {
            return Err(Duration::new(1, 0));
        }
        let mut power_of_10 = 1;
        while power_of_10 < value.as_nanos() {
            if value.as_nanos() < power_of_10 * 10 {
                // value must not be a power of ten, since power_of_10 < value < power_of_10 * 10
                return Err(Duration::new(0, power_of_10 as u32));
            }
            power_of_10 *= 10;
        }
        let previous = self.time_gran;
        self.time_gran = value;
        Ok(previous)
    }

    /// Set the maximum write size for a single request
    ///
    /// On success returns the previous value. On error returns the nearest value which will succeed
    pub fn set_max_write(&mut self, value: u32) -> Result<u32, u32> {
        if value == 0 {
            return Err(1);
        }
        if value > MAX_WRITE_SIZE as u32 {
            return Err(MAX_WRITE_SIZE as u32);
        }
        let previous = self.max_write;
        self.max_write = value;
        Ok(previous)
    }

    /// Set the maximum readahead size
    ///
    /// On success returns the previous value. On error returns the nearest value which will succeed
    pub fn set_max_readahead(&mut self, value: u32) -> Result<u32, u32> {
        if value == 0 {
            return Err(1);
        }
        if value > self.max_max_readahead {
            return Err(self.max_max_readahead);
        }
        let previous = self.max_readahead;
        self.max_readahead = value;
        Ok(previous)
    }

    /// Add a set of capabilities.
    ///
    /// On success returns Ok, else return bits of capabilities not supported when capabilities you provided are not all supported by kernel.
    pub fn add_capabilities(&mut self, capabilities_to_add: u64) -> Result<(), u64> {
        if capabilities_to_add & self.capabilities != capabilities_to_add {
            return Err(capabilities_to_add - (capabilities_to_add & self.capabilities));
        }
        self.requested |= capabilities_to_add;
        Ok(())
    }

    /// Set the maximum number of pending background requests. Such as readahead requests.
    ///
    /// On success returns the previous value. On error returns the nearest value which will succeed
    #[cfg(feature = "abi-7-13")]
    pub fn set_max_background(&mut self, value: u16) -> Result<u16, u16> {
        if value == 0 {
            return Err(1);
        }
        let previous = self.max_background;
        self.max_background = value;
        Ok(previous)
    }

    /// Set the threshold of background requests at which the kernel will consider the filesystem
    /// request queue congested. (it may then switch to sleeping instead of spin-waiting, for example)
    ///
    /// On success returns the previous value. On error returns the nearest value which will succeed
    #[cfg(feature = "abi-7-13")]
    pub fn set_congestion_threshold(&mut self, value: u16) -> Result<u16, u16> {
        if value == 0 {
            return Err(1);
        }
        let previous = self.congestion_threshold();
        self.congestion_threshold = Some(value);
        Ok(previous)
    }

    #[cfg(feature = "abi-7-13")]
    fn congestion_threshold(&self) -> u16 {
        match self.congestion_threshold {
            // Default to a threshold of 3/4 of the max background threads
            None => (self.max_background as u32 * 3 / 4) as u16,
            Some(value) => min(value, self.max_background),
        }
    }

    #[cfg(feature = "abi-7-28")]
    fn max_pages(&self) -> u16 {
        ((max(self.max_write, self.max_readahead) - 1) / page_size::get() as u32) as u16 + 1
    }
}

/// Mount the given filesystem to the given mountpoint. This function will
/// not return until the filesystem is unmounted.
///
/// Note that you need to lead each option with a separate `"-o"` string.
#[deprecated(note = "use mount2() instead")]
pub fn mount<FS: Filesystem, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[&OsStr],
) -> io::Result<()> {
    let options = parse_options_from_args(options)?;
    mount2(filesystem, mountpoint, options.as_ref())
}

/// Mount the given filesystem to the given mountpoint. This function will
/// not return until the filesystem is unmounted.
///
/// NOTE: This will eventually replace mount(), once the API is stable
pub fn mount2<FS: Filesystem, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[MountOption],
) -> io::Result<()> {
    check_option_conflicts(options)?;
    Session::new(filesystem, mountpoint.as_ref(), options).and_then(|se| se.run())
}

/// Mount the given filesystem to the given mountpoint. This function spawns
/// a background thread to handle filesystem operations while being mounted
/// and therefore returns immediately. The returned handle should be stored
/// to reference the mounted filesystem. If it's dropped, the filesystem will
/// be unmounted.
#[deprecated(note = "use spawn_mount2() instead")]
pub fn spawn_mount<'a, FS: Filesystem + Send + 'static + 'a, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[&OsStr],
) -> io::Result<BackgroundSession> {
    let options: Option<Vec<_>> = options
        .iter()
        .map(|x| Some(MountOption::from_str(x.to_str()?)))
        .collect();
    let options = options.ok_or(ErrorKind::InvalidData)?;
    Session::new(filesystem, mountpoint.as_ref(), options.as_ref()).and_then(|se| se.spawn())
}

/// Mount the given filesystem to the given mountpoint. This function spawns
/// a background thread to handle filesystem operations while being mounted
/// and therefore returns immediately. The returned handle should be stored
/// to reference the mounted filesystem. If it's dropped, the filesystem will
/// be unmounted.
///
/// NOTE: This is the corresponding function to mount2.
pub fn spawn_mount2<'a, FS: Filesystem + Send + 'static + 'a, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[MountOption],
) -> io::Result<BackgroundSession> {
    check_option_conflicts(options)?;
    Session::new(filesystem, mountpoint.as_ref(), options).and_then(|se| se.spawn())
}
