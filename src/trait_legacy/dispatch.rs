#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::sync::atomic::Ordering::Relaxed;

use crate::ll::{self, Operation, Request as AnyRequest};
#[cfg(feature = "abi-7-40")]
use crate::request::get_backing_handler;
use crate::request::{RequestHandler, RequestMeta};
use crate::session::{SessionACL, SessionMeta};
use crate::{KernelConfig, ll::Errno};

#[cfg(feature = "abi-7-24")]
use super::ReplyLseek;
#[cfg(target_os = "macos")]
use super::ReplyXTimes;
use super::callback::{DirectoryHandler, OpenHandler};
use super::{
    Filesystem, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyLock, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
};
#[cfg(feature = "abi-7-11")]
use super::{PollHandler, ReplyIoctl, ReplyPoll};
#[cfg(feature = "abi-7-21")]
use super::{ReplyDirectoryPlus, callback::DirectoryPlusHandler};


#[derive(Debug)]
/// Userspace metadata for a given request
pub struct Request<'r> {
    meta: &'r RequestMeta,
}

impl<'r> Request<'r> {
    /// Creates a legacy-compatible Request from the given RequestMeta
    pub fn new(meta: &'r RequestMeta) -> Self {
        Request { meta }
    }
}

// Helper functions for compatibility with Legacy Filesystem
impl Request<'_> {
    /// Returns the unique identifier of this request
    #[inline]
    pub fn unique(&self) -> u64 {
        self.meta.unique
    }

    /// Returns the uid of this request
    #[inline]
    pub fn uid(&self) -> u32 {
        self.meta.uid
    }

    /// Returns the gid of this request
    #[inline]
    pub fn gid(&self) -> u32 {
        self.meta.gid
    }

    /// Returns the pid of this request
    #[inline]
    pub fn pid(&self) -> u32 {
        self.meta.pid
    }

    /// Returns a copy of this Request's data as a RequestMeta
    pub fn meta(&self) -> RequestMeta {
        *self.meta
    }
}

impl RequestHandler {
    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub(crate) fn dispatch_legacy<FS: Filesystem>(mut self, fs: &mut FS, se_meta: &SessionMeta) {
        debug!("{}", self.request);
        let op = if let Ok(op) = self.request.operation() {
            op
        } else {
            self.replyhandler.error(Errno::ENOSYS);
            return;
        };
        // Implementation of `--allow-root` & required access check for `--auto-unmount`
        let access_denied = if (se_meta.allowed == SessionACL::RootAndOwner
            && self.request.uid() != se_meta.session_owner
            && self.request.uid() != 0)
            || (se_meta.allowed == SessionACL::Owner && self.request.uid() != se_meta.session_owner)
        {
            match op {
                // Only allow operations that the kernel may issue without a uid set
                Operation::Init(_)
                | Operation::Destroy(_)
                | Operation::Read(_)
                | Operation::ReadDir(_)
                | Operation::Forget(_)
                | Operation::Write(_)
                | Operation::FSync(_)
                | Operation::FSyncDir(_)
                | Operation::Release(_)
                | Operation::ReleaseDir(_) => false,
                #[cfg(feature = "abi-7-16")]
                Operation::BatchForget(_) => false,
                #[cfg(feature = "abi-7-21")]
                Operation::ReadDirPlus(_) => false,
                _ => true,
            }
        } else {
            false
        };
        if access_denied {
            self.replyhandler.error(Errno::EACCES);
            return;
        }
        let req = Request::new(&self.meta);
        match op {
            // Filesystem initialization
            Operation::Init(x) => {
                // We don't support ABI versions before 7.6
                let v = x.version();
                if v < ll::Version(7, 6) {
                    error!("Unsupported FUSE ABI version {v}");
                    self.replyhandler.error(Errno::EPROTO);
                    return;
                }
                // Remember ABI version supported by kernel
                se_meta.proto_major.store(v.major(), Relaxed);
                se_meta.proto_minor.store(v.minor(), Relaxed);
                let mut config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to
                // propose a different config or return an error
                match fs.init(&req, &mut config) {
                    Ok(()) => {
                        // Reply with our desired version and settings. If the kernel supports a
                        // larger major version, it'll re-send a matching init message. If it
                        // supports only lower major versions, we replied with an error above.
                        debug!(
                            "INIT response: ABI {}.{}, flags {:#x}, max readahead {}, max write {}",
                            ll::fuse_abi::FUSE_KERNEL_VERSION,
                            ll::fuse_abi::FUSE_KERNEL_MINOR_VERSION,
                            x.capabilities() & config.requested,
                            config.max_readahead,
                            config.max_write
                        );
                        se_meta.initialized.store(true, Relaxed);
                        self.replyhandler.config(x.capabilities(), config);
                    }
                    Err(errno) => {
                        // Filesystem refused the config.
                        self.replyhandler.error(Errno::from_i32(errno));
                    }
                }
            }
            // Any operation is invalid before initialization
            _ if !se_meta.initialized.load(Relaxed) => {
                warn!("Ignoring FUSE operation before init: {}", self.request);
                self.replyhandler.error(Errno::EIO);
            }
            // Filesystem destroyed
            Operation::Destroy(_x) => {
                fs.destroy();
                se_meta.destroyed.store(true, Relaxed);
                self.replyhandler.ok();
            }
            // Any operation is invalid after destroy
            _ if se_meta.destroyed.load(Relaxed) => {
                warn!("Ignoring FUSE operation after destroy: {}", self.request);
                self.replyhandler.error(Errno::EIO);
            }
            Operation::Interrupt(_) => {
                // TODO: handle FUSE_INTERRUPT
                self.replyhandler.error(Errno::ENOSYS);
            }
            /* ------ Regular Operations ------ */
            Operation::Lookup(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyEntry::new(Box::new(Some(self.replyhandler)));
                fs.lookup(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Forget(x) => {
                fs.forget(&req, self.request.nodeid().into(), x.nlookup()); // no response
                self.replyhandler.disable(); // no reply
            }
            Operation::GetAttr(_attr) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyAttr::new(Box::new(Some(self.replyhandler)));
                #[cfg(feature = "abi-7-9")]
                fs.getattr(
                    &req,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(Into::into),
                    reply,
                    /* blank space */
                );
                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
                fs.getattr(
                    &req,
                    self.request.nodeid().into(),
                    None,
                    reply,
                    /* blank space */
                );
            }
            Operation::SetAttr(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyAttr::new(Box::new(Some(self.replyhandler)));
                fs.setattr(
                    &req,
                    self.request.nodeid().into(),
                    x.mode(),
                    x.uid(),
                    x.gid(),
                    x.size(),
                    x.atime(),
                    x.mtime(),
                    x.ctime(),
                    x.file_handle().map(Into::into),
                    x.crtime(),
                    x.chgtime(),
                    x.bkuptime(),
                    x.flags(),
                    reply,
                );
            }
            Operation::ReadLink(_) => {
                let reply = ReplyData::new(Box::new(Some(self.replyhandler)));
                fs.readlink(&req, self.request.nodeid().into(), reply);
            }
            Operation::MkNod(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyEntry::new(Box::new(Some(self.replyhandler)));
                fs.mknod(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.rdev(),
                    reply,
                );
            }
            Operation::MkDir(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyEntry::new(Box::new(Some(self.replyhandler)));
                fs.mkdir(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    reply,
                );
            }
            Operation::Unlink(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.unlink(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                    /* blank space */
                );
            }
            Operation::RmDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.rmdir(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                );
            }
            Operation::SymLink(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyEntry::new(Box::new(Some(self.replyhandler)));
                fs.symlink(
                    &req,
                    self.request.nodeid().into(),
                    x.link_name().as_os_str(),
                    x.target(),
                    reply,
                );
            }
            Operation::Rename(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.rename(
                    &req,
                    self.request.nodeid().into(),
                    x.src().name.as_os_str(),
                    x.dest().dir.into(),
                    x.dest().name.as_os_str(),
                    0,
                    reply,
                );
            }
            Operation::Link(x) => {
                if se_meta.allowed == SessionACL::RootAndOwner {
                    self.replyhandler.attr_ttl_override();
                }
                let reply = ReplyEntry::new(Box::new(Some(self.replyhandler)));
                fs.link(
                    &req,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name.as_os_str(),
                    reply,
                );
            }
            Operation::Open(x) => {
                #[cfg(feature = "abi-7-40")]
                let backinghandler = get_backing_handler!(self);
                #[cfg(feature = "abi-7-40")]
                let callback = OpenHandler::new(self.replyhandler, backinghandler);
                #[cfg(not(feature = "abi-7-40"))]
                let callback = OpenHandler::new(self.replyhandler);
                let reply = ReplyOpen::new(Box::new(callback));
                fs.open(&req, self.request.nodeid().into(), x.flags(), reply);
            }
            Operation::Read(x) => {
                let reply = ReplyData::new(Box::new(Some(self.replyhandler)));
                fs.read(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    reply,
                );
            }
            Operation::Write(x) => {
                let reply = ReplyWrite::new(Box::new(Some(self.replyhandler)));
                fs.write(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    reply,
                );
            }
            Operation::Flush(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.flush(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    reply,
                );
            }
            Operation::Release(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.release(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    x.flush(),
                    reply,
                );
            }
            Operation::FSync(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.fsync(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            Operation::OpenDir(x) => {
                #[cfg(feature = "abi-7-40")]
                let backinghandler = get_backing_handler!(self);
                #[cfg(feature = "abi-7-40")]
                let callback = OpenHandler::new(self.replyhandler, backinghandler);
                #[cfg(not(feature = "abi-7-40"))]
                let callback = OpenHandler::new(self.replyhandler);
                let reply = ReplyOpen::new(Box::new(callback));
                fs.opendir(
                    &req,
                    self.request.nodeid().into(),
                    x.flags(),
                    reply,
                    /* blank space */
                );
            }
            Operation::ReadDir(x) => {
                let callback = DirectoryHandler::new(x.size() as usize, self.replyhandler);
                let reply = ReplyDirectory::new(Box::new(callback));
                fs.readdir(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    reply,
                );
            }
            Operation::ReleaseDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.releasedir(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    reply,
                );
            }
            Operation::FSyncDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.fsyncdir(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            Operation::StatFs(_) => {
                let reply = ReplyStatfs::new(Box::new(Some(self.replyhandler)));
                fs.statfs(&req, self.request.nodeid().into(), reply);
            }
            Operation::SetXAttr(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.setxattr(
                    &req,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position(),
                    reply,
                );
            }
            Operation::GetXAttr(x) => {
                let reply = ReplyXattr::new(Box::new(Some(self.replyhandler)));
                fs.getxattr(
                    &req,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32(),
                    reply,
                );
            }
            Operation::ListXAttr(x) => {
                let reply = ReplyXattr::new(Box::new(Some(self.replyhandler)));
                fs.listxattr(
                    &req,
                    self.request.nodeid().into(),
                    x.size(),
                    reply,
                    /* blank space */
                );
            }
            Operation::RemoveXAttr(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.removexattr(
                    &req,
                    self.request.nodeid().into(),
                    x.name(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Access(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.access(
                    &req,
                    self.request.nodeid().into(),
                    x.mask(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Create(x) => {
                let reply = ReplyCreate::new(Box::new(Some(self.replyhandler)));
                fs.create(
                    &req,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.flags(),
                    reply,
                );
            }
            Operation::GetLk(x) => {
                let reply = ReplyLock::new(Box::new(Some(self.replyhandler)));
                fs.getlk(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    reply,
                );
            }
            Operation::SetLk(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.setlk(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    false,
                    reply,
                );
            }
            Operation::SetLkW(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.setlk(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    true,
                    reply,
                );
            }
            Operation::BMap(x) => {
                let reply = ReplyBmap::new(Box::new(Some(self.replyhandler)));
                fs.bmap(
                    &req,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-11")]
            Operation::IoCtl(x) => {
                if x.unrestricted() {
                    self.replyhandler.error(Errno::ENOSYS);
                } else {
                    let reply = ReplyIoctl::new(Box::new(Some(self.replyhandler)));
                    fs.ioctl(
                        &req,
                        self.request.nodeid().into(),
                        x.file_handle().into(),
                        x.flags(),
                        x.command(),
                        x.in_data(),
                        x.out_size(),
                        reply,
                    );
                }
            }
            #[cfg(feature = "abi-7-11")]
            Operation::Poll(x) => {
                let reply = ReplyPoll::new(Box::new(Some(self.replyhandler)));
                fs.poll(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    PollHandler::new(
                        self.queue,
                        x.kernel_handle(),
                    ),
                    x.events(),
                    x.flags(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-15")]
            Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                self.replyhandler.error(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            Operation::BatchForget(x) => {
                fs.batch_forget(&req, x.nodes()); // no response
                self.replyhandler.disable(); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            Operation::FAllocate(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.fallocate(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-21")]
            Operation::ReadDirPlus(x) => {
                let mut callback = DirectoryPlusHandler::new(x.size() as usize, self.replyhandler);
                if se_meta.allowed == SessionACL::RootAndOwner {
                    callback.attr_ttl_override();
                }
                let reply = ReplyDirectoryPlus::new(Box::new(callback));
                fs.readdirplus(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-23")]
            Operation::Rename2(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.rename(
                    &req,
                    x.from().dir.into(),
                    x.from().name.as_os_str(),
                    x.to().dir.into(),
                    x.to().name.as_os_str(),
                    x.flags(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-24")]
            Operation::Lseek(x) => {
                let reply = ReplyLseek::new(Box::new(Some(self.replyhandler)));
                fs.lseek(
                    &req,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-28")]
            Operation::CopyFileRange(x) => {
                let (i, o) = (x.src(), x.dest());
                let reply = ReplyWrite::new(Box::new(Some(self.replyhandler)));
                fs.copy_file_range(
                    &req,
                    i.inode.into(),
                    i.file_handle.into(),
                    i.offset,
                    o.inode.into(),
                    o.file_handle.into(),
                    o.offset,
                    x.len(),
                    x.flags().try_into().unwrap(),
                    reply,
                );
            }
            #[cfg(target_os = "macos")]
            Operation::SetVolName(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.setvolname(
                    &req,
                    x.name(),
                    reply,
                    /* blank space */
                );
            }
            #[cfg(target_os = "macos")]
            Operation::GetXTimes(x) => {
                let reply = ReplyXTimes::new(Box::new(Some(self.replyhandler)));
                fs.getxtimes(
                    &req,
                    x.nodeid().into(),
                    reply,
                    /* blank space */
                );
            }
            #[cfg(target_os = "macos")]
            Operation::Exchange(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.replyhandler)));
                fs.exchange(
                    &req,
                    x.from().dir.into(),
                    x.from().name.as_os_str(),
                    x.to().dir.into(),
                    x.to().name.as_os_str(),
                    x.options(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-12")]
            Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                self.replyhandler.error(Errno::ENOSYS);
            }
        }
    }
}
