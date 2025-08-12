use std::{
    fs::File,
    io::{self, IoSlice},
    os::unix::prelude::AsRawFd,
    sync::Arc,
};

use crate::ll::fuse_abi;
use crate::ll::ioctl::ioctl_clone_fuse_fd;
#[cfg(feature = "abi-7-40")]
use crate::ll::ioctl::{ioctl_close_backing, ioctl_open_backing};
use libc::{c_int, c_void, size_t};

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

    #[cfg(not(feature = "tokio"))]
    /// Receives data up to the capacity of the given buffer.
    /// Can be awaited: blocks on a dedicated thread.
    /// Populates data into the buffer starting from the point of alignment
    #[allow(unused)] // this stub is a placeholder for future non-tokio async i/o
    pub async fn receive_async(&self, mut _buffer: Vec<u8>) -> (io::Result<Vec<u8>>, Vec<u8>) {
        let _thread_ch = self.clone();
        /*
        ??.spawn_blocking(move || {
            let res = thread_ch.receive(&mut buffer);
            (res, buffer)
        }).await.expect("Unable to recover worker i/o thread")
        */
        unimplemented!("non-tokio async i/o not implemented")
    }

    #[cfg(feature = "tokio")]
    /// Receives data up to the capacity of the given buffer.
    /// Can be awaited: blocks on a dedicated thread.
    /// Populates data into the buffer starting from the point of alignment
    pub async fn receive_async(&self, mut buffer: Vec<u8>) -> (io::Result<Vec<u8>>, Vec<u8>) {
        let thread_ch = self.clone();
        tokio::task::spawn_blocking(move || {
            let res = thread_ch.receive(&mut buffer);
            (res, buffer)
        })
        .await
        .expect("Unable to recover worker i/o thread")
    }

    /// Polls the kernel to determine if a request is ready for reading (does not block).
    /// This method is used in a synchronous execution model.
    pub fn poll_read(&self) -> io::Result<bool> {
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
            -1 => Err(io::Error::last_os_error()),
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
                        Err(io::Error::other(format!(
                            "Poll error, revents: {:#x}.",
                            buf[0].revents
                        )))
                    } else {
                        // Probably fine
                        Ok(false)
                    }
                }
            }
        }
    }

    pub fn try_receive(&self, buffer: &mut [u8]) -> io::Result<Option<Vec<u8>>> {
        if self.poll_read()? {
            Ok(Some(self.receive(buffer)?))
        } else {
            Ok(None)
        }
    }

    /// Reads a request, but only if a request is ready for immediate reading (does not block).
    /// This method is used in an asynchronous execution model.
    /// Always returns the owned buffer for later re-use.
    /// On read: returns (Ok(Some(data)), buffer)
    /// On no read: returns  (Ok(None), buffer)
    /// On error: returns  (Err, buffer)
    #[allow(unreachable_code, dead_code, unused_variables)] // The non-tokio portion of this function is a TODO item.
    pub async fn try_receive_async(
        &self,
        buffer: Vec<u8>,
    ) -> (io::Result<Option<Vec<u8>>>, Vec<u8>) {
        if match self.poll_read() {
            Err(e) => {
                return (Err(e), buffer);
            }
            Ok(ready) => ready,
        } {
            // TODO: non-tokio implementation
            #[cfg(not(feature = "tokio"))]
            let (res, new_buffer) = (Ok(vec![]), buffer);
            #[cfg(not(feature = "tokio"))]
            unimplemented!("non-tokio async i/o not implemented");
            // Tokio implementation
            #[cfg(feature = "tokio")]
            let (res, new_buffer) = self.receive_async(buffer).await;
            (res.map(|data| Some(data)), new_buffer)
        } else {
            (Ok(None), buffer)
        }
    }

    /// Polls the kernel to determine if channel is ready to accept a notification (does not block).
    /// This method is used in the synchronous notifications execution model.
    #[allow(unused)] // This function is reserved for future non-blocking synchronous writes
    pub fn poll_write(&self) -> io::Result<bool> {
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
            -1 => Err(io::Error::last_os_error()),
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
                        Err(io::Error::other(format!(
                            "Poll error, revents: {:#x}.",
                            buf[0].revents
                        )))
                    } else {
                        // Probably fine
                        Ok(false)
                    }
                }
            }
        }
    }
    /// Writes data from the owned buffer.
    /// Blocks the current thread.
    pub fn send(&self, bufs: &[IoSlice<'_>]) -> io::Result<()> {
        log::debug!("about to try a blocking write on fd {}", self.raw_fd);
        for x in bufs {
            log::debug!("the buf has length {}", x.len());
        }
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

    #[cfg(not(feature = "tokio"))]
    /// Writes data from the owned buffer.
    /// Can be awaited: blocks on a dedicated thread.
    #[allow(unused)] // this stub is a placeholder for future non-tokio async i/o
    pub async fn send_async(&self, bufs: &[IoSlice<'_>]) -> io::Result<()> {
        let bufs = bufs
            .iter()
            .map(|v| Vec::from(v.as_ref()))
            .collect::<Vec<Vec<u8>>>();
        let thread_sender = self.clone();
        std::thread::spawn(move || {
            let bufs = bufs
                .iter()
                .map(|v| IoSlice::new(v))
                .collect::<Vec<IoSlice<'_>>>();
            thread_sender.send(&bufs)
        });
        unimplemented!("non-tokio async i/o not implemented");
        Ok(())
    }
    #[cfg(feature = "tokio")]
    /// Writes data from the owned buffer.
    /// Can be awaited: blocks on a dedicated thread.
    #[allow(unused)] // this stub is a placeholder for future tokio async i/o
    pub async fn send_async(&self, bufs: &[IoSlice<'_>]) -> io::Result<()> {
        let bufs = bufs
            .iter()
            .map(|v| Vec::from(v.as_ref()))
            .collect::<Vec<Vec<u8>>>();
        let thread_sender = self.clone();
        tokio::task::spawn_blocking(move || {
            let bufs = bufs
                .iter()
                .map(|v| IoSlice::new(v))
                .collect::<Vec<IoSlice<'_>>>();
            thread_sender.send(&bufs)
        }); //.await.expect("Unable to recover worker i/o thread")
        Ok(())
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
