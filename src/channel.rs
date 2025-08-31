use std::{fs::File, io, ops::{Deref, DerefMut}, os::{fd::{AsFd, BorrowedFd}, unix::prelude::AsRawFd}, sync::Arc};

use libc::{c_int, c_void, size_t};

use crate::ll::fuse_abi;
#[cfg(feature = "side-channel")]
use crate::ll::fuse_ioctl::ioctl_clone_fuse_fd;

pub const FUSE_HEADER_ALIGNMENT: usize = std::mem::align_of::<fuse_abi::fuse_in_header>();

pub(crate) struct AlignedBuffer {
    buf: Vec<u8>,
    offset: usize,
    data_len: usize,
    data_max: usize,
}

impl AlignedBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        // Add some extra capacity to account for the alignment offset
        let buf = vec![0; capacity + 4096];
        let offset = FUSE_HEADER_ALIGNMENT - (buf.as_ptr() as usize) % FUSE_HEADER_ALIGNMENT;
        let data_max = capacity + 4096 - offset;
        AlignedBuffer {
            buf,
            offset,
            data_len: 0,
            data_max,
        }
    }
}

impl Deref for AlignedBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        let start = self.offset;
        let end = self.offset + self.data_len;
        &self.buf[start..end]
    }
}
impl DerefMut for AlignedBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let start = self.offset;
        let end = self.offset + self.data_len;
        &mut self.buf[start..end] 
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
    pub fn receive(&self, buffer: &mut AlignedBuffer) -> io::Result<usize> {
        // reset the buffer to its max length
        buffer.data_len = buffer.data_max;
        let rc = unsafe {
            libc::read(
                self.raw_fd,
                buffer.as_ptr() as *mut c_void,
                buffer.len() as size_t,
            )
        };
        if rc < 0 {
            // store 0 length and return the error
            buffer.data_len = 0;
            Err(io::Error::last_os_error())
        } else {
            // store and return the length of the read
            buffer.data_len = rc as usize;
            Ok(rc as usize)
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
