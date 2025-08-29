use std::{fs::File, io, os::{unix::prelude::AsRawFd, fd::{AsFd, BorrowedFd}}, sync::Arc};

use libc::{c_int, c_void, size_t};

use crate::ll::fuse_abi;
#[cfg(feature = "side-channel")]
use crate::ll::fuse_ioctl::ioctl_clone_fuse_fd;

pub const FUSE_HEADER_ALIGNMENT: usize = std::mem::align_of::<fuse_abi::fuse_in_header>();

pub(crate) fn aligned_sub_buf(buf: &mut [u8], alignment: usize) -> &mut [u8] {
    let off = alignment - (buf.as_ptr() as usize) % alignment;
    if off == alignment {
        buf
    } else {
        &mut buf[off..]
    }
}

/// A raw communication channel to the FUSE kernel driver.
/// May be cloned and sent to other threads.
#[derive(Clone, Debug)]
pub(crate) struct Channel {
    owned_fd: Arc<File>,
    pub raw_fd: i32,
    #[cfg(feature = "side-channel")]
    is_main: bool,
}

impl AsFd for Channel {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.owned_fd.as_fd()
    }
}

impl Channel {
    // Create a new communication channel to the kernel driver.
    // The argument is a `File` opened on a fuse device.
    pub fn new(device: File) -> Self {
        let owned_fd = Arc::new(device);
        let raw_fd = owned_fd.as_raw_fd();
        Self {
            owned_fd,
            raw_fd,
            #[cfg(feature = "side-channel")]
            is_main: true,
        }
    }
    // Create a new communication channel to the kernel driver.
    // The argument is a `Arc<File>` opened on a fuse device.
    #[allow(dead_code)]
    pub fn from_shared(device: &Arc<File>) -> Self {
        let raw_fd = device.as_raw_fd().as_raw_fd();
        let owned_fd = device.clone();
        Self {
            owned_fd,
            raw_fd,
            #[cfg(feature = "side-channel")]
            is_main: true,
        }
    }

    /// Receives data up to the capacity of the given buffer (can block).
    /// Populates data into the buffer starting from the point of alignment
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<Vec<u8>> {
        let buf_aligned = aligned_sub_buf(buffer, FUSE_HEADER_ALIGNMENT);
        let rc = unsafe {
            libc::read(
                self.raw_fd,
                buf_aligned.as_ptr() as *mut c_void,
                buf_aligned.len() as size_t,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            let data = Vec::from(&buf_aligned[..rc as usize]);
            buf_aligned[..rc as usize].fill(0);
            Ok(data)
        }
    }
    /// Writes data from the owned buffer.
    /// Blocks the current thread.
    pub fn send(&self, bufs: &[io::IoSlice<'_>]) -> io::Result<()> {
        let rc = unsafe {
            libc::writev(
                self.raw_fd,
                bufs.as_ptr().cast::<libc::iovec>(),
                bufs.len() as c_int,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            debug_assert_eq!(bufs.iter().map(|b| b.len()).sum::<usize>(), rc as usize);
            Ok(())
        }
    }

    #[cfg(feature = "side-channel")]
    /// Creates a new fuse worker channel. Self should be the main channel.
    /// # Errors
    /// Propagates underlying errors.
    pub fn fork(&self) -> std::io::Result<Self> {
        if !self.is_main {
            log::error!(
                "Attempted to create a new fuse worker from a fuse channel that is not main"
            );
        }
        let fuse_device_name = "/dev/fuse";
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(fuse_device_name)?;
        let raw_fd = file.as_raw_fd();
        ioctl_clone_fuse_fd(raw_fd, self.raw_fd as u32)?;
        Ok(Channel {
            owned_fd: Arc::new(file),
            raw_fd,
            is_main: false,
        })
    }
}
