use super::fuse_abi as abi;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::sync::Arc;

pub(crate) fn register_backing_id(
        channel: &Arc<File>,
        fd: u32,
    ) -> std::io::Result<u32> {
    let map = abi::fuse_backing_map_out {
        fd,
        flags: 0,
        padding: 0,
    };
    let id = unsafe { abi::fuse_dev_ioc_backing_open(channel.as_raw_fd(), &map) }
    ?;
    Ok(id as u32)
}

pub(crate) fn deregister_backing_id(
        channel: &Arc<File>,
        backing_id: u32,
    ) -> std::io::Result<u32> {
    let code = unsafe { abi::fuse_dev_ioc_backing_close(channel.as_raw_fd(), &backing_id) }
    ?;
    Ok(code as u32)
}