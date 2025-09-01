#[allow(unused_imports)]
use log::{debug, error, info, warn};
#[cfg(feature = "abi-7-28")]
use std::cmp::max;
use std::cmp::min;
#[cfg(feature = "abi-7-23")]
use std::time::Duration;
use std::time::SystemTime;
#[cfg(feature = "serializable")]
use serde::{Deserialize, Serialize};

use crate::ll::fuse_abi::consts::*;
use crate::ll::fuse_abi::fuse_init_out;
use crate::session::MAX_WRITE_SIZE;

/* ------ FUSE configuration ------ */

/// We generally support async reads
#[cfg(not(target_os = "macos"))]
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
    pub(crate) capabilities: u64,
    pub(crate) requested: u64,
    pub(crate) max_readahead: u32,
    pub(crate) max_max_readahead: u32,
    pub(crate) max_background: u16,
    pub(crate) congestion_threshold: Option<u16>,
    pub(crate) max_write: u32,
    #[cfg(feature = "abi-7-23")]
    pub(crate) time_gran: Duration,
    #[cfg(feature = "abi-7-40")]
    pub(crate) max_stack_depth: u32,
}

impl KernelConfig {
    pub(crate) fn new(capabilities: u64, max_readahead: u32) -> Self {
        Self {
            capabilities,
            requested: default_init_flags(capabilities),
            max_readahead,
            max_max_readahead: max_readahead,
            max_background: 16,
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
    pub fn set_congestion_threshold(&mut self, value: u16) -> Result<u16, u16> {
        if value == 0 {
            return Err(1);
        }
        let previous = self.congestion_threshold();
        self.congestion_threshold = Some(value);
        Ok(previous)
    }

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

    pub(crate) fn requested(&self) -> u64 {
        self.requested
    }

    pub(crate) fn into_fuse_init_out(self, flags: u64) -> fuse_init_out {
        fuse_init_out {
            major: crate::ll::fuse_abi::FUSE_KERNEL_VERSION,
            minor: crate::ll::fuse_abi::FUSE_KERNEL_MINOR_VERSION,
            max_readahead: self.max_readahead,
            #[cfg(not(feature = "abi-7-36"))]
            flags: flags as u32,
            #[cfg(feature = "abi-7-36")]
            flags: (flags | FUSE_INIT_EXT) as u32,
            max_background: self.max_background,
            congestion_threshold: self.congestion_threshold(),
            max_write: self.max_write,
            #[cfg(feature = "abi-7-23")]
            time_gran: self.time_gran.as_nanos() as u32,
            #[cfg(all(feature = "abi-7-23", not(feature = "abi-7-28")))]
            reserved: [0; 9],
            #[cfg(feature = "abi-7-28")]
            max_pages: self.max_pages(),
            #[cfg(feature = "abi-7-28")]
            unused2: 0,
            #[cfg(all(feature = "abi-7-28", not(feature = "abi-7-36")))]
            reserved: [0; 8],
            #[cfg(feature = "abi-7-36")]
            flags2: (flags >> 32) as u32,
            #[cfg(all(feature = "abi-7-36", not(feature = "abi-7-40")))]
            reserved: [0; 7],
            #[cfg(feature = "abi-7-40")]
            max_stack_depth: self.max_stack_depth,
            #[cfg(feature = "abi-7-40")]
            reserved: [0; 6],
        }
    }
}

/* ------ Operation input/output types ------ */

/// File types
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serializable", derive(Serialize, Deserialize))]
pub enum FileType {
    /// Named pipe (S_IFIFO)
    NamedPipe,
    /// Character device (S_IFCHR)
    CharDevice,
    /// Block device (S_IFBLK)
    BlockDevice,
    /// Directory (S_IFDIR)
    Directory,
    /// Regular file (S_IFREG)
    RegularFile,
    /// Symbolic link (S_IFLNK)
    Symlink,
    /// Unix domain socket (S_IFSOCK)
    Socket,
}

/// File attributes
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serializable", derive(Serialize, Deserialize))]
pub struct FileAttr {
    /// Inode number
    pub ino: u64,
    /// Size in bytes
    pub size: u64,
    /// Size in blocks
    pub blocks: u64,
    /// Time of last access
    pub atime: SystemTime,
    /// Time of last modification
    pub mtime: SystemTime,
    /// Time of last change
    pub ctime: SystemTime,
    /// Time of creation (macOS only)
    pub crtime: SystemTime,
    /// Kind of file (directory, file, pipe, etc)
    pub kind: FileType,
    /// Permissions
    pub perm: u16,
    /// Number of hard links
    pub nlink: u32,
    /// User id
    pub uid: u32,
    /// Group id
    pub gid: u32,
    /// Rdev
    pub rdev: u32,
    /// Block size
    pub blksize: u32,
    /// Flags (macOS only, see chflags(2))
    pub flags: u32,
}