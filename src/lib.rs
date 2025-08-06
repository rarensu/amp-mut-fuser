//! FUSE userspace library implementation
//!
//! This is an improved rewrite of the FUSE userspace library (lowlevel interface) to fully take
//! advantage of Rust's architecture. The only thing we rely on in the real libfuse are mount
//! and unmount calls which are needed to establish a fd to talk to the kernel driver.

#![warn(missing_docs, missing_debug_implementations, rust_2018_idioms)]

#[allow(unused_imports)]
use log::{debug, info, warn, error};
use mnt::mount_options::parse_options_from_args;
/* 
#[cfg(feature = "serializable")]
use serde::de::value::F64Deserializer;
#[cfg(feature = "serializable")]
use serde::{Deserialize, Serialize};
*/
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};
use std::convert::AsRef;
#[cfg(feature = "threaded")]
use std::io::ErrorKind;
#[allow(clippy::wildcard_imports)] // avoid duplicating feature gates
use crate::ll::fuse_abi::consts::*;
pub use crate::ll::fuse_abi::FUSE_ROOT_ID;
pub use crate::ll::{fuse_abi::consts, TimeOrNow};
pub use ll::Errno;
use crate::mnt::mount_options::check_option_conflicts;
use crate::session::MAX_WRITE_SIZE;
pub use mnt::mount_options::MountOption;
#[cfg(feature = "abi-7-11")]
pub use notify::{Notification, Poll};
#[cfg(feature = "abi-7-12")]
pub use notify::{InvalEntry, InvalInode};
#[cfg(feature = "abi-7-15")]
pub use notify::Store;
#[cfg(feature = "abi-7-18")]
pub use notify::Delete;
#[cfg(feature = "abi-7-11")]
pub use reply::Ioctl;
#[cfg(target_os = "macos")]
pub use reply::XTimes;
pub use bytes::Bytes;
pub use reply::{Dirent, DirentList, DirentPlusList, Entry, FileAttr, FileType, Open, Statfs, Xattr, Lock};
pub use request::RequestMeta;
pub use session::{Session, SessionACL, SessionUnmounter};
#[cfg(feature = "threaded")]
pub use session::BackgroundSession;
pub use container::{Container, SafeBorrow};
#[cfg(feature = "abi-7-28")]
use std::cmp::max;
#[cfg(feature = "abi-7-13")]
use std::cmp::min;

mod channel;
mod container;
mod ll;
mod mnt;
#[cfg(feature = "abi-7-11")]
mod notify;
mod reply;
mod request;
mod session;
/// middle ground. syncronous but no callbacks
pub mod t_sync;
/// asyncronous
pub mod t_async;
pub use t_async::Filesystem;

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

#[allow(unused_variables)]
const fn default_init_flags(capabilities: u64) -> u64 {
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

#[derive(Copy, Clone, Debug, PartialEq)]
/// Target of a `forget` or `batch_forget` operation.
pub struct Forget {
    /// Inode of the file to be forgotten.
    pub ino: u64,
    /// The number of times the file has been looked up (and not yet forgotten).
    /// When a `forget` operation is received, the filesystem should typically
    /// decrement its internal reference count for the inode by `nlookup`.
    pub nlookup: u64
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
    /// With `max_stack_depth` > 1, the backing files can be on a stacked fs (e.g. overlayfs)
    /// themselves and with `max_stack_depth` == 1, this FUSE filesystem can be stacked as the
    /// underlying fs of a stacked fs (e.g. overlayfs).
    ///
    /// The kernel currently has a hard maximum value of 2.  Anything higher won't work.
    /// # Errors
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
    /// # Errors
    /// On success returns the previous value. On error returns the nearest value which will succeed.
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
    /// # Errors
    /// On success returns the previous value. On error returns the nearest value which will succeed.
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
    /// # Errors
    /// On success returns the previous value. On error returns the nearest value which will succeed.
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
    /// # Errors
    /// On success returns Ok. On error, returns the bits of capabilities not supported by kernel.
    pub fn add_capabilities(&mut self, capabilities_to_add: u64) -> Result<(), u64> {
        if capabilities_to_add & self.capabilities != capabilities_to_add {
            return Err(capabilities_to_add - (capabilities_to_add & self.capabilities));
        }
        self.requested |= capabilities_to_add;
        Ok(())
    }

    /// Set the maximum number of pending background requests. Such as readahead requests.
    /// # Errors
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
    /// # Errors
    /// On success returns the previous value. On error returns the nearest value which will succeed.
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
            None => (u32::from(self.max_background) * 3 / 4) as u16,
            Some(value) => min(value, self.max_background),
        }
    }

    #[cfg(feature = "abi-7-28")]
    #[allow(clippy::cast_possible_truncation)] // truncation is a feature of this computation
    fn max_pages(&self) -> u16 {
        ((max(self.max_write, self.max_readahead) - 1) / page_size::get() as u32) as u16 + 1
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
/// This enum is an optional way for the Filesystem to report its status to a Session thread.
pub enum FsStatus {
    /// Default may be used when the Filesystem does not implement a status
    Default,
    /// Ready indicates the Filesystem has no actions in progress
    Ready,
    /// Busy indicates the Filesytem has one or more actions in progress
    Busy,
    /// Stopped indicates that the Filesystem will not accept new requests
    Stopped
}

/// Mount the given filesystem to the given mountpoint. This function will
/// block until the filesystem is unmounted.
///
/// `filesystem`: The filesystem implementation.
/// `mountpoint`: The path to the mountpoint.
/// `options`: A slice of mount options. Each option needs to be a separate string,
/// typically starting with `"-o"`. For example: `&[OsStr::new("-o"), OsStr::new("auto_unmount")]`.
/// # Errors
/// Error if the mount does not succeed.
#[deprecated(note = "Use `mount2` instead, which takes a slice of `MountOption` enums for better type safety and clarity.")]
pub fn mount<FS: Filesystem + 'static, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[&OsStr],
) -> io::Result<()> {
    let options = parse_options_from_args(options)?;
    mount2(filesystem, mountpoint, options.as_ref())
}

/// Mount the given filesystem to the given mountpoint. This function will
/// block until the filesystem is unmounted.
///
/// `filesystem`: The filesystem implementation.
/// `mountpoint`: The path to the mountpoint.
/// `options`: A slice of `MountOption` enums specifying mount options.
///
/// This is the recommended way to mount a FUSE filesystem.
/// # Errors
/// Error if the mount does not succeed.
pub fn mount2<FS: Filesystem + 'static, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[MountOption],
) -> io::Result<()> {
    check_option_conflicts(options)?;
    Session::new(filesystem, mountpoint.as_ref(), options).and_then(
        |se| 
        futures::executor::block_on(
            se.run()
        )
    )
}

/// Mount the given filesystem to the given mountpoint in a background thread.
/// This function spawns a new thread to handle filesystem operations and returns
/// immediately. The returned `BackgroundSession` handle should be stored to
/// keep the filesystem mounted. When the handle is dropped, the filesystem will
/// be unmounted.
///
/// `filesystem`: The filesystem implementation. Must be `Send + 'static`.
/// `mountpoint`: The path to the mountpoint.
/// `options`: A slice of mount options. Each option needs to be a separate string,
/// typically starting with `"-o"`. For example: `&[OsStr::new("-o"), OsStr::new("auto_unmount")]`.
/// # Errors
/// Error if the session is not started.
#[cfg(feature = "threaded")]
#[deprecated(note = "Use `spawn_mount2` instead, which takes a slice of `MountOption` enums for better type safety and clarity.")]
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
    Session::new(filesystem, mountpoint.as_ref(), options.as_ref()).and_then(session::Session::spawn)
}

/// Mount the given filesystem to the given mountpoint in a background thread.
/// This function spawns a new thread to handle filesystem operations and returns
/// immediately. The returned `BackgroundSession` handle should be stored to
/// keep the filesystem mounted. When the handle is dropped, the filesystem will
/// be unmounted.
///
/// `filesystem`: The filesystem implementation. Must be `Send + 'static`.
/// `mountpoint`: The path to the mountpoint.
/// `options`: A slice of `MountOption` enums specifying mount options.
///
/// This is the recommended way to mount a FUSE filesystem in the background.
/// # Errors
/// Error if the session is not started.
#[cfg(feature = "threaded")]
pub fn spawn_mount2<'a, FS: Filesystem + Send + 'static + 'a, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[MountOption],
) -> io::Result<BackgroundSession> {
    check_option_conflicts(options)?;
    Session::new(filesystem, mountpoint.as_ref(), options).and_then(session::Session::spawn)
}
