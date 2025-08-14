#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::sync::atomic::Ordering::Relaxed;

use crate::ll::{self, Operation, Request as AnyRequest};
use crate::session::{Session};
use crate::request::Request;
use crate::{ll::Errno, KernelConfig,};

#[cfg(feature = "abi-7-24")]
use super::ReplyLseek;
#[cfg(target_os = "macos")]
use super::ReplyXTimes;
use super::callback::DirectoryHandler;
use super::{
    Filesystem, ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyLock, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
};
#[cfg(feature = "abi-7-11")]
use super::{PollHandle, ReplyIoctl, ReplyPoll};
#[cfg(feature = "abi-7-21")]
use super::{ReplyDirectoryPlus, callback::DirectoryPlusHandler};

impl Request<'_> {
    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub(crate) fn dispatch_legacy<FS: Filesystem>(mut self, se: &mut Session<FS>) {
        debug!("{}", self.request);
        let unique = self.request.unique();

    
        let op = self.request.operation().map_err(|_| Errno::ENOSYS)?;
        // Implement allow_root & access check for auto_unmount
        if (se.allowed == SessionACL::RootAndOwner
            && self.request.uid() != se.session_owner
            && self.request.uid() != 0)
            || (se.allowed == SessionACL::Owner && self.request.uid() != se.session_owner)
        {
            #[cfg(feature = "abi-7-21")]
            {
                match op {
                    // Only allow operations that the kernel may issue without a uid set
                    ll::Operation::Init(_)
                    | ll::Operation::Destroy(_)
                    | ll::Operation::Read(_)
                    | ll::Operation::ReadDir(_)
                    | ll::Operation::ReadDirPlus(_)
                    | ll::Operation::BatchForget(_)
                    | ll::Operation::Forget(_)
                    | ll::Operation::Write(_)
                    | ll::Operation::FSync(_)
                    | ll::Operation::FSyncDir(_)
                    | ll::Operation::Release(_)
                    | ll::Operation::ReleaseDir(_) => {}
                    _ => {
                        return Err(Errno::EACCES);
                    }
                }
            }
            #[cfg(all(feature = "abi-7-16", not(feature = "abi-7-21")))]
            {
                match op {
                    // Only allow operations that the kernel may issue without a uid set
                    ll::Operation::Init(_)
                    | ll::Operation::Destroy(_)
                    | ll::Operation::Read(_)
                    | ll::Operation::ReadDir(_)
                    | ll::Operation::BatchForget(_)
                    | ll::Operation::Forget(_)
                    | ll::Operation::Write(_)
                    | ll::Operation::FSync(_)
                    | ll::Operation::FSyncDir(_)
                    | ll::Operation::Release(_)
                    | ll::Operation::ReleaseDir(_) => {}
                    _ => {
                        return Err(Errno::EACCES);
                    }
                }
            }
            #[cfg(not(feature = "abi-7-16"))]
            {
                match op {
                    // Only allow operations that the kernel may issue without a uid set
                    ll::Operation::Init(_)
                    | ll::Operation::Destroy(_)
                    | ll::Operation::Read(_)
                    | ll::Operation::ReadDir(_)
                    | ll::Operation::Forget(_)
                    | ll::Operation::Write(_)
                    | ll::Operation::FSync(_)
                    | ll::Operation::FSyncDir(_)
                    | ll::Operation::Release(_)
                    | ll::Operation::ReleaseDir(_) => {}
                    _ => {
                        return Err(Errno::EACCES);
                    }
                }
            }
        }
        match op {
            // Filesystem initialization
            Operation::Init(x) => {
                // We don't support ABI versions before 7.6
                let v = x.version();
                if v < ll::Version(7, 6) {
                    error!("Unsupported FUSE ABI version {v}");
                    self.reply().error(Errno::EPROTO);
                    return;
                }
                // Remember ABI version supported by kernel
                se.proto_major = v.major();
                se.proto_minor = v.minor();
                let mut config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to
                // propose a different config or return an error
                match se.filesystem.init(&self, &mut config) {
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
                        se.initialized = true;
                        self.reply().config(x.capabilities(), config);
                    }
                    Err(errno) => {
                        // Filesystem refused the config.
                        self.reply().error(Errno::from_i32(errno));
                    }
                }
            }
            // Any operation is invalid before initialization
            _ if !se.initialized => {
                warn!("Ignoring FUSE operation before init: {}", self.request);
                self.reply().error(Errno::EIO);
            }
            // Filesystem destroyed
            Operation::Destroy(_x) => {
                se.filesystem.destroy();
                se.destroyed = true;
                self.reply().ok();
            }
            // Any operation is invalid after destroy
            _ if se.destroyed => {
                warn!("Ignoring FUSE operation after destroy: {}", self.request);
                self.reply().error(Errno::EIO);
            }
            Operation::Interrupt(_) => {
                // TODO: handle FUSE_INTERRUPT
                self.reply().error(Errno::ENOSYS);
            }
            /* ------ Regular Operations ------ */
            Operation::Lookup(x) => {
                let reply = ReplyEntry::new(Box::new(Some(self.reply())));
                se.filesystem.lookup(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Forget(x) => {
                se.filesystem.forget(&self, self.request.nodeid().into(), x.nlookup()); // no response
                self.reply().disable(); // no reply
            }
            Operation::GetAttr(_attr) => {
                let reply = ReplyAttr::new(Box::new(Some(self.reply())));
                #[cfg(feature = "abi-7-9")]
                se.filesystem.getattr(
                    &self,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(Into::into),
                    reply,
                    /* blank space */
                );
                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
                se.filesystem.getattr(
                    &self,
                    self.request.nodeid().into(),
                    None,
                    reply,
                    /* blank space */
                );
            }
            Operation::SetAttr(x) => {
                let reply = ReplyAttr::new(Box::new(Some(self.reply())));
                se.filesystem.setattr(
                    &self,
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
                let reply = ReplyData::new(Box::new(Some(self.reply())));
                se.filesystem.readlink(&self, self.request.nodeid().into(), reply);
            }
            Operation::MkNod(x) => {
                let reply = ReplyEntry::new(Box::new(Some(self.reply())));
                se.filesystem.mknod(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.rdev(),
                    reply,
                );
            }
            Operation::MkDir(x) => {
                let reply = ReplyEntry::new(Box::new(Some(self.reply())));
                se.filesystem.mkdir(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    reply,
                );
            }
            Operation::Unlink(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.unlink(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                    /* blank space */
                );
            }
            Operation::RmDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.rmdir(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    reply,
                );
            }
            Operation::SymLink(x) => {
                let reply = ReplyEntry::new(Box::new(Some(self.reply())));
                se.filesystem.symlink(
                    &self,
                    self.request.nodeid().into(),
                    x.link_name().as_os_str(),
                    x.target(),
                    reply,
                );
            }
            Operation::Rename(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.rename(
                    &self,
                    self.request.nodeid().into(),
                    x.src().name.as_os_str(),
                    x.dest().dir.into(),
                    x.dest().name.as_os_str(),
                    0,
                    reply,
                );
            }
            Operation::Link(x) => {
                let reply = ReplyEntry::new(Box::new(Some(self.reply())));
                se.filesystem.link(
                    &self,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name.as_os_str(),
                    reply,
                );
            }
            Operation::Open(x) => {
                let reply = ReplyOpen::new(Box::new(Some(self.reply())));
                se.filesystem.open(&self, self.request.nodeid().into(), x.flags(), reply);
            }
            Operation::Read(x) => {
                let reply = ReplyData::new(Box::new(Some(self.reply())));
                se.filesystem.read(
                    &self,
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
                let reply = ReplyWrite::new(Box::new(Some(self.reply())));
                se.filesystem.write(
                    &self,
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
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.flush(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    reply,
                );
            }
            Operation::Release(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.release(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    x.flush(),
                    reply,
                );
            }
            Operation::FSync(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.fsync(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            Operation::OpenDir(x) => {
                let reply = ReplyOpen::new(Box::new(Some(self.reply())));
                se.filesystem.opendir(
                    &self,
                    self.request.nodeid().into(),
                    x.flags(),
                    reply,
                    /* blank space */
                );
            }
            Operation::ReadDir(x) => {
                let container = DirectoryHandler::new(x.size() as usize, self.reply());
                let reply = ReplyDirectory::new(Box::new(container));
                se.filesystem.readdir(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.ofself.fset(),
                    reply,
                );
            }
            Operation::ReleaseDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.releasedir(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    reply,
                );
            }
            Operation::FSyncDir(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.fsyncdir(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            Operation::StatFs(_) => {
                let reply = ReplyStatfs::new(Box::new(Some(self.reply())));
                se.filesystem.statfs(&self, self.request.nodeid().into(), reply);
            }
            Operation::SetXAttr(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.setxattr(
                    &self,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position(),
                    reply,
                );
            }
            Operation::GetXAttr(x) => {
                let reply = ReplyXattr::new(Box::new(Some(self.reply())));
                se.filesystem.getxattr(
                    &self,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32(),
                    reply,
                );
            }
            Operation::ListXAttr(x) => {
                let reply = ReplyXattr::new(Box::new(Some(self.reply())));
                se.filesystem.listxattr(
                    &self,
                    self.request.nodeid().into(),
                    x.size(),
                    reply,
                    /* blank space */
                );
            }
            Operation::RemoveXAttr(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.removexattr(
                    &self,
                    self.request.nodeid().into(),
                    x.name(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Access(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.access(
                    &self,
                    self.request.nodeid().into(),
                    x.mask(),
                    reply,
                    /* blank space */
                );
            }
            Operation::Create(x) => {
                let reply = ReplyCreate::new(Box::new(Some(self.reply())));
                se.filesystem.create(
                    &self,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.flags(),
                    reply,
                );
            }
            Operation::GetLk(x) => {
                let reply = ReplyLock::new(Box::new(Some(self.reply())));
                se.filesystem.getlk(
                    &self,
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
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.setlk(
                    &self,
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
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.setlk(
                    &self,
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
                let reply = ReplyBmap::new(Box::new(Some(self.reply())));
                se.filesystem.bmap(
                    &self,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block(),
                    reply,
                );
            }

            #[cfg(feature = "abi-7-11")]
            Operation::IoCtl(x) => {
                if x.unrestricted() {
                    self.reply().error(Errno::ENOSYS);
                } else {
                    let reply = ReplyIoctl::new(Box::new(Some(self.reply())));
                    se.filesystem.ioctl(
                        &self,
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
                let reply = ReplyPoll::new(Box::new(Some(self.reply())));
                se.filesystem.poll(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    PollHandle(x.kernel_handle()),
                    x.events(),
                    x.flags(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-15")]
            Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                self.reply().error(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            Operation::BatchForget(x) => {
                se.filesystem.batch_forget(&self, x.nodes()); // no response
                self.reply().disable(); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            Operation::FAllocate(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.fallocate(
                    &self,
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
                let mut callback = DirectoryPlusHandler::new(x.size() as usize, self.reply());
                let reply = ReplyDirectoryPlus::new(Box::new(callback));
                se.filesystem.readdirplus(
                    &self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-23")]
            Operation::Rename2(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.rename(
                    &self,
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
                let reply = ReplyLseek::new(Box::new(Some(self.reply())));
                se.filesystem.lseek(
                    &self,
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
                let reply = ReplyWrite::new(Box::new(Some(self.reply())));
                se.filesystem.copy_file_range(
                    &self,
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
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.setvolname(
                    &self,
                    x.name(),
                    reply,
                    /* blank space */
                );
            }
            #[cfg(target_os = "macos")]
            Operation::GetXTimes(x) => {
                let reply = ReplyXTimes::new(Box::new(Some(self.reply())));
                se.filesystem.getxtimes(
                    &self,
                    x.nodeid().into(),
                    reply,
                    /* blank space */
                );
            }
            #[cfg(target_os = "macos")]
            Operation::Exchange(x) => {
                let reply = ReplyEmpty::new(Box::new(Some(self.reply())));
                se.filesystem.exchange(
                    &self,
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
                self.reply().error(Errno::ENOSYS);
            }
        }
    }
}