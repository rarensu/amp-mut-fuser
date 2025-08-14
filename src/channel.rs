use std::{
    fs::File,
    io,
    os::unix::prelude::AsRawFd,
    sync::Arc,
};

use libc::{c_int, c_void, size_t};

pub const FUSE_HEADER_ALIGNMENT: usize = std::mem::align_of::<crate::ll::fuse_abi::fuse_in_header>();

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
}

use std::os::fd::{AsFd, BorrowedFd};
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
        Self { owned_fd, raw_fd }
    }
    // Create a new communication channel to the kernel driver.
    // The argument is a `Arc<File>` opened on a fuse device.
    #[allow(dead_code)]
    pub fn from_shared(device: &Arc<File>) -> Self {
        let raw_fd = device.as_raw_fd().as_raw_fd();
        let owned_fd = device.clone();
        Self { raw_fd, owned_fd }
    }

    /// Receives data up to the capacity of the given buffer (can block).
    /// Populates data into the buffer starting from the point of alignment
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<Vec<u8>> {
        log::debug!("about to try a blocking read on fd {:?}", self.raw_fd);
        log::debug!(
            "about to try a blocking read on with buffer {:?}, {:?}",
            buffer.len(),
            buffer.get(0..20)
        );
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

    #[cfg(feature = "abi-7-40")]
    fn open_backing(&self, fd: BorrowedFd<'_>) -> io::Result<BackingId> {
        BackingId::create(&self.0, fd)
    }
}
