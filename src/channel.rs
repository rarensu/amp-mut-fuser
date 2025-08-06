use std::{
    fs::File,
    io::{self, IoSlice},
    os::unix::prelude::AsRawFd,
    sync::Arc,
};
use smallvec::SmallVec;

use libc::{c_int, c_void, size_t};
use crate::ll::fuse_abi;
use crate::ll::ioctl::ioctl_clone_fuse_fd;
#[cfg(feature = "abi-7-40")]
use crate::ll::ioctl::{ioctl_close_backing, ioctl_open_backing};

pub const SYNC_SLEEP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);
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
}


use std::os::fd::{BorrowedFd, AsFd};
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
        let raw_fd= owned_fd.as_raw_fd();
        Self {owned_fd, raw_fd}
    }
    // Create a new communication channel to the kernel driver. 
    // The argument is a `Arc<File>` opened on a fuse device.
    #[allow(dead_code)] 
    pub fn from_shared(device: &Arc<File>) -> Self {
        let raw_fd= device.as_raw_fd().as_raw_fd();
        let owned_fd = device.clone();
        Self {raw_fd, owned_fd}
    }

    /// Receives data up to the capacity of the given buffer (can block).
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<usize> {
        log::debug!("about to try a blocking read on fd {:?}", self.raw_fd);
        log::debug!("about to try a blocking read on with buffer {:?}, {:?}", buffer.len(), buffer.get(0..20));
        let rc = unsafe {
            libc::read(
                self.raw_fd,
                buffer.as_ptr() as *mut c_void,
                buffer.len() as size_t,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc as usize)
        }
    }

    /// Receives data up to the capacity of the given buffer (can block).
    pub async fn receive_later(&self, mut buffer: Vec<u8>) -> (io::Result<usize>, Vec<u8>) { 
        let thread_ch = self.clone();  
        tokio::task::spawn_blocking(move || {
            let mut buf = aligned_sub_buf(
                &mut buffer,
                FUSE_HEADER_ALIGNMENT,
            );
            let res = thread_ch.receive(&mut buf);
            (res, buffer)
        }).await.expect("Unable to recover worker i/o thread")
    }

    /// Polls the kernel to determine if a request is ready for reading (does not block).
    /// This method is used in the synchronous notifications execution model.
    pub fn ready_read(&self) -> io::Result<bool> {
        let mut buf = [libc::pollfd {
            fd: self.raw_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let rc = unsafe {
            libc::poll(
                buf.as_mut_ptr(),
                1,
                0, // ms; Non-blocking poll
            )
        };
        match rc {
            -1 => {
                Err(io::Error::last_os_error())
            }
            0 => {
                // Timeout with no events on FUSE FD.
                Ok(false)
            }
            _ => {
                // ret > 0, events are available
                if (buf[0].revents & libc::POLLIN) != 0 {
                    // FUSE FD is ready to read.
                    Ok(true)
                } else {
                    // Handling unexpected events
                    if (buf[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                        // Probably very bad
                        Err(io::Error::other(format!("Poll error, revents: {:#x}.", buf[0].revents)))
                    } else {
                        // Probably fine
                        Ok(false)
                    }
                }
            }
        }
    }
    /// Polls the kernel to determine if channel is ready to accept a notification (does not block).
    /// This method is used in the synchronous notifications execution model.
    pub fn ready_write(&self) -> io::Result<bool> {
        let mut buf = [libc::pollfd {
            fd: self.raw_fd,
            events: libc::POLLOUT,
            revents: 0,
        }];
        let rc = unsafe {
            libc::poll(
                buf.as_mut_ptr(),
                1,
                0, // ms; Non-blocking poll
            )
        };
        match rc {
            -1 => {
                Err(io::Error::last_os_error())
            }
            0 => {
                // Timeout with no events on FUSE FD.
                Ok(false)
            }
            _ => {
                // ret > 0, events are available
                if (buf[0].revents & libc::POLLOUT) != 0 {
                    // FUSE FD is ready to write.
                    Ok(true)
                } else {
                    // Handling unexpected events
                    if (buf[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                        // Probably very bad
                        Err(io::Error::other(format!("Poll error, revents: {:#x}.", buf[0].revents)))
                    } else {
                        // Probably fine
                        Ok(false)
                    }
                }
            }
        }
    }

    pub fn send(&self, bufs: &[io::IoSlice<'_>]) -> io::Result<()> {
        log::debug!("about to try a blocking write on fd {}", self.raw_fd);
        for x in bufs { log::debug!("the buf has length {}", x.len()); }
        let rc = unsafe {
            libc::writev(
                self.raw_fd,
                bufs.as_ptr().cast::<libc::iovec>(),
                bufs.len() as c_int,
            )
        };
        log::debug!("just done a blocking write on {}: {rc}", self.raw_fd);
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            debug_assert_eq!(bufs.iter().map(|b| b.len()).sum::<usize>(), rc as usize);
            Ok(())
        }
    }

    pub async fn send_later(&self, bufs: SmallVec<[Vec<u8>; 4]>) -> io::Result<()> {
        let thread_sender = self.clone();
        tokio::task::spawn_blocking(move || {
            let bufs = bufs.iter().map(|v| {IoSlice::new(v)}).collect::<Vec<IoSlice<'_>>>();
            thread_sender.send(&bufs)
        }).await.expect("Unable to recover worker i/o thread")
    }

    /// ?
    pub fn new_fuse_worker(&self, main_fuse_fd: u32) -> std::io::Result<()> {
        ioctl_clone_fuse_fd(self.raw_fd, main_fuse_fd)
    }
    /// Registers a file descriptor with the kernel.
    /// If the kernel accepts, it returns a backing ID.
    #[cfg(feature = "abi-7-40")]
    pub fn open_backing(&self, backing_fd: u32) -> std::io::Result<u32> {
        ioctl_open_backing(self.raw_fd, backing_fd)
    }

    /// Deregisters a backing ID.
    #[cfg(feature = "abi-7-40")]
    pub fn close_backing(&self, backing_id: u32) -> std::io::Result<u32> {
        ioctl_close_backing(self.raw_fd, backing_id)
    }
}
