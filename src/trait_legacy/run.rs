use libc::{EAGAIN, EINTR, ENODEV, ENOENT};
#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::io;

use crate::request::RequestHandler;
use crate::session::{BUFFER_SIZE, Session};

use crate::trait_legacy::Filesystem;

impl<FS: Filesystem> Session<FS> {
    /// Run the session loop that receives kernel requests and dispatches them to method
    /// calls into the filesystem. This read-dispatch-loop is non-concurrent to prevent
    /// having multiple buffers (which take up much memory), but the filesystem methods
    /// may run concurrent by spawning threads.
    pub fn run_legacy(&mut self) -> io::Result<()> {
        // Buffer for receiving requests from the kernel. Only one is allocated and
        // it is reused immediately after dispatching to conserve memory and allocations.
        let mut buffer = vec![0; BUFFER_SIZE];
        loop {
            // Read the next request from the given channel to kernel driver
            // The kernel driver makes sure that we get exactly one request per read
            match self.ch_main.receive(&mut buffer) {
                Ok(data) => {
                    match RequestHandler::new(
                        self.ch_main.clone(),
                        data
                    ) {
                        // Dispatch request
                        Some(req) => req.dispatch_legacy(&mut self.filesystem, &self.meta),
                        // Quit loop on illegal request
                        None => break,
                    }
                },
                Err(err) => match err.raw_os_error() {
                    // Operation interrupted. Accordingly to FUSE, this is safe to retry
                    Some(ENOENT) => continue,
                    // Interrupted system call, retry
                    Some(EINTR) => continue,
                    // Explicitly try again
                    Some(EAGAIN) => continue,
                    // Filesystem was unmounted, quit the loop
                    Some(ENODEV) => break,
                    // Unhandled error
                    _ => return Err(err),
                },
            }
        }
        Ok(())
    }
}
