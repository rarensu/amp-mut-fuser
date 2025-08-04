// A small client to exercise the ioctl example filesystem.
// Assumes the file "fioc" exists in the current working directory mount
// and the filesystem was mounted with the abi-7-11 feature.
//
// Usage:
//   1) In one shell, mount the example:
//        cargo run --example ioctl --features abi-7-11 /tmp/fioc
//   2) In another shell, cd to the mountpoint:
//        cd /tmp/fioc
//   3) Run this client to test setting/getting size:
//        cargo run --example ioctl_client
//
// It will:
//   - GET size (expect something, prints it)
//   - SET size to 4096
//   - GET size (expect 4096)
//   - SET size to 0
//   - GET size (expect 0)

#![allow(clippy::cast_possible_truncation)]

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;

 // Generate wrappers matching examples/ioctl.rs
 nix::ioctl_read!(ioctl_get_size, 'E', 0, usize);
 nix::ioctl_write_ptr!(ioctl_set_size, 'E', 1, usize);

fn main() -> io::Result<()> {
    // Open the example file in the current directory mount
    let f = OpenOptions::new().read(true).write(true).open("fioc")?;
    let fd = f.as_raw_fd();

    // Helper to GET size
    let get_size = || -> nix::Result<usize> {
        let mut out: usize = 0;
        unsafe {
            ioctl_get_size(fd, &mut out as *mut usize)?;
        }
        Ok(out)
    };

    // Helper to SET size
    let set_size = |sz: usize| -> nix::Result<()> {
        let mut val = sz;
        unsafe {
            ioctl_set_size(fd, &mut val as *mut usize)?;
        }
        Ok(())
    };

    // 1) Read current size
    match get_size() {
        Ok(sz) => println!("Initial size: {sz} bytes"),
        Err(e) => {
            eprintln!("FIOC_GET_SIZE failed: {e}");
            return Err(io::Error::new(io::ErrorKind::Other, format!("ioctl get failed: {e}")));
        }
    }

    // 2) Set size to 4096
    if let Err(e) = set_size(4096) {
        eprintln!("FIOC_SET_SIZE(4096) failed: {e}");
        return Err(io::Error::new(io::ErrorKind::Other, format!("ioctl set failed: {e}")));
    } else {
        println!("Set size to 4096 bytes");
    }

    // 3) Get size and expect 4096
    match get_size() {
        Ok(sz) => {
            println!("After set(4096), size: {sz} bytes");
            if sz != 4096 {
                eprintln!("Unexpected size after set(4096): got {sz}, expected 4096");
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("size mismatch after set(4096): {sz}"),
                ));
            }
        }
        Err(e) => {
            eprintln!("FIOC_GET_SIZE failed after set(4096): {e}");
            return Err(io::Error::new(io::ErrorKind::Other, format!("ioctl get failed: {e}")));
        }
    }

    // 4) Set size to 0
    if let Err(e) = set_size(0) {
        eprintln!("FIOC_SET_SIZE(0) failed: {e}");
        return Err(io::Error::new(io::ErrorKind::Other, format!("ioctl set failed: {e}")));
    } else {
        println!("Set size to 0 bytes");
    }

    // 5) Get size and expect 0
    match get_size() {
        Ok(sz) => {
            println!("After set(0), size: {sz} bytes");
            if sz != 0 {
                eprintln!("Unexpected size after set(0): got {sz}, expected 0");
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("size mismatch after set(0): {sz}"),
                ));
            }
        }
        Err(e) => {
            eprintln!("FIOC_GET_SIZE failed after set(0): {e}");
            return Err(io::Error::new(io::ErrorKind::Other, format!("ioctl get failed: {e}")));
        }
    }

    // Optionally write a newline to stdout to flush buffered output in some environments
    io::stdout().flush().ok();

    println!("ioctl_client completed successfully.");
    Ok(())
}
