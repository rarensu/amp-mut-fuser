#[allow(unused_imports)]
use log::{debug, error, info, warn};

use crate::ll::{self, Request as AnyRequest};
use crate::request::Request;
use crate::session::{SessionACL, Session};
use crate::{KernelConfig, ll::Errno};

use super::Filesystem;
#[cfg(feature = "abi-7-21")]
use super::ReplyDirectoryPlus;
#[cfg(feature = "abi-7-24")]
use super::ReplyLseek;
#[cfg(target_os = "macos")]
use super::ReplyXTimes;
use super::{
    ReplyAttr, ReplyBmap, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyIoctl, ReplyLock, ReplyOpen,  ReplyPoll, ReplyStatfs, ReplyWrite, ReplyXattr,
};

use super::PollHandle;

impl<'a> Request<'a> {
    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub(crate) fn dispatch<FS: Filesystem>(&self, se: &mut Session<FS>) {
        debug!("{}", self.request);
        let unique = self.request.unique();

        let res = match self.dispatch_req(se) {
            Ok(Some(resp)) => resp,
            Ok(None) => return,
            Err(errno) => self.request.reply_err(errno),
        }
        .with_iovec(unique, |iov| self.ch.send(iov));

        if let Err(err) = res {
            warn!("Request {:?}: Failed to send reply: {}", unique, err)
        }
    }

    fn dispatch_req<FS: Filesystem>(
        &self,
        se: &mut Session<FS>,
    ) -> Result<Option<Response<'_>>, Errno> {
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
            #[cfg(not(feature = "abi-7-21"))]
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
        }
        match op {
            // Filesystem initialization
            ll::Operation::Init(x) => {
                // We don't support ABI versions before 7.6
                let v = x.version();
                if v < ll::Version(7, 6) {
                    error!("Unsupported FUSE ABI version {}", v);
                    return Err(Errno::EPROTO);
                }
                // Remember ABI version supported by kernel
                se.proto_major = v.major();
                se.proto_minor = v.minor();

                let mut config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to return an error
                se.filesystem
                    .init(self, &mut config)
                    .map_err(Errno::from_i32)?;

                // Reply with our desired version and settings. If the kernel supports a
                // larger major version, it'll re-send a matching init message. If it
                // supports only lower major versions, we replied with an error above.
                debug!(
                    "INIT response: ABI {}.{}, flags {:#x}, max readahead {}, max write {}",
                    abi::FUSE_KERNEL_VERSION,
                    abi::FUSE_KERNEL_MINOR_VERSION,
                    x.capabilities() & config.requested,
                    config.max_readahead,
                    config.max_write
                );
                se.initialized = true;
                return Ok(Some(x.reply(&config)));
            }
            // Any operation is invalid before initialization
            _ if !se.initialized => {
                warn!("Ignoring FUSE operation before init: {}", self.request);
                return Err(Errno::EIO);
            }
            // Filesystem destroyed
            ll::Operation::Destroy(x) => {
                se.filesystem.destroy();
                se.destroyed = true;
                return Ok(Some(x.reply()));
            }
            // Any operation is invalid after destroy
            _ if se.destroyed => {
                warn!("Ignoring FUSE operation after destroy: {}", self.request);
                return Err(Errno::EIO);
            }

            ll::Operation::Interrupt(_) => {
                // TODO: handle FUSE_INTERRUPT
                return Err(Errno::ENOSYS);
            }

            ll::Operation::Lookup(x) => {
                let reply = ReplyEntry::new(self.reply);
                se.filesystem.lookup(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    reply,
                );
            }
            ll::Operation::Forget(x) => {
                se.filesystem
                    .forget(self, self.request.nodeid().into(), x.nlookup()); // no reply
            }
            ll::Operation::GetAttr(_attr) => {
                let reply = ReplyAttr::new(self.reply);
                se.filesystem.getattr(
                    self,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(|fh| fh.into()),
                    reply,
                );
            }
            ll::Operation::SetAttr(x) => {
                let reply = ReplyAttr::new(self.reply);
                se.filesystem.setattr(
                    self,
                    self.request.nodeid().into(),
                    x.mode(),
                    x.uid(),
                    x.gid(),
                    x.size(),
                    x.atime(),
                    x.mtime(),
                    x.ctime(),
                    x.file_handle().map(|fh| fh.into()),
                    x.crtime(),
                    x.chgtime(),
                    x.bkuptime(),
                    x.flags(),
                    reply,
                );
            }
            ll::Operation::ReadLink(_) => {
                let reply = ReplyData::new(self.reply);
                se.filesystem
                    .readlink(self, self.request.nodeid().into(), reply);
            }
            ll::Operation::MkNod(x) => {
                let reply = ReplyEntry::new(self.reply);
                se.filesystem.mknod(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    x.rdev(),
                    reply,
                );
            }
            ll::Operation::MkDir(x) => {
                let reply = ReplyEntry::new(self.reply);
                se.filesystem.mkdir(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    reply,
                );
            }
            ll::Operation::Unlink(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.unlink(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    reply,
                );
            }
            ll::Operation::RmDir(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.rmdir(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    reply,
                );
            }
            ll::Operation::SymLink(x) => {
                let reply = ReplyEntry::new(self.reply);
                se.filesystem.symlink(
                    self,
                    self.request.nodeid().into(),
                    x.link_name().as_ref(),
                    Path::new(x.target()),
                    reply,
                );
            }
            ll::Operation::Rename(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.rename(
                    self,
                    self.request.nodeid().into(),
                    x.src().name.as_ref(),
                    x.dest().dir.into(),
                    x.dest().name.as_ref(),
                    0,
                    reply,
                );
            }
            ll::Operation::Link(x) => {
                let reply = ReplyEntry::new(self.reply);
                se.filesystem.link(
                    self,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name.as_ref(),
                    reply,
                );
            }
            ll::Operation::Open(x) => {
                let reply = ReplyOpen::new(self.reply);
                se.filesystem
                    .open(self, self.request.nodeid().into(), x.flags(), reply);
            }
            ll::Operation::Read(x) => {
                let reply = ReplyData::new(self.reply);
                se.filesystem.read(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(|l| l.into()),
                    reply,
                );
            }
            ll::Operation::Write(x) => {
                let reply = ReplyWrite::new(self.reply);
                se.filesystem.write(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(|l| l.into()),
                    reply,
                );
            }
            ll::Operation::Flush(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.flush(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    reply,
                );
            }
            ll::Operation::Release(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.release(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(|x| x.into()),
                    x.flush(),
                    reply,
                );
            }
            ll::Operation::FSync(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.fsync(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            ll::Operation::OpenDir(x) => {
                let reply = ReplyOpen::new(self.reply);
                se.filesystem
                    .opendir(self, self.request.nodeid().into(), x.flags(), reply);
            }
            ll::Operation::ReadDir(x) => {
                let reply = ReplyDirectory::new(
                        self.reply,
                        x.size() as usize,
                    );
                se.filesystem.readdir(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    reply,
                );
            }
            ll::Operation::ReleaseDir(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.releasedir(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    reply,
                );
            }
            ll::Operation::FSyncDir(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.fsyncdir(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    reply,
                );
            }
            ll::Operation::StatFs(_) => {
                let reply = ReplyStatfs::new(self.reply);
                se.filesystem
                    .statfs(self, self.request.nodeid().into(), reply);
            }
            ll::Operation::SetXAttr(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.setxattr(
                    self,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position(),
                    reply,
                );
            }
            ll::Operation::GetXAttr(x) => {
                let reply = ReplyXattr::new(self.reply);
                se.filesystem.getxattr(
                    self,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32(),
                    reply,
                );
            }
            ll::Operation::ListXAttr(x) => {
                let reply = ReplyXattr::new(self.reply);
                se.filesystem
                    .listxattr(self, self.request.nodeid().into(), x.size(), reply);
            }
            ll::Operation::RemoveXAttr(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.removexattr(
                    self,
                    self.request.nodeid().into(),
                    x.name(),
                    reply,
                );
            }
            ll::Operation::Access(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem
                    .access(self, self.request.nodeid().into(), x.mask(), reply);
            }
            ll::Operation::Create(x) => {
                let reply = ReplyCreate::new(self.reply);
                se.filesystem.create(
                    self,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    x.flags(),
                    reply,
                );
            }
            ll::Operation::GetLk(x) => {
                let reply = ReplyLock::new(self.reply);
                se.filesystem.getlk(
                    self,
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
            ll::Operation::SetLk(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.setlk(
                    self,
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
            ll::Operation::SetLkW(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.setlk(
                    self,
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
            ll::Operation::BMap(x) => {
                let reply = ReplyBmap::new(self.reply);
                se.filesystem.bmap(
                    self,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block(),
                    reply,
                );
            }

            ll::Operation::IoCtl(x) => {
                if x.unrestricted() {
                    return Err(Errno::ENOSYS);
                } else {
                    let reply = ReplyIoctl::new(self.reply);
                    se.filesystem.ioctl(
                        self,
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
            ll::Operation::Poll(x) => {
                let ph = PollHandle::new(se.ch.sender(), x.kernel_handle());
                let reply = ReplyPoll::new(self.reply);
                se.filesystem.poll(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    ph,
                    x.events(),
                    x.flags(),
                    reply,
                );
            }
            ll::Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                return Err(Errno::ENOSYS);
            }
            ll::Operation::BatchForget(x) => {
                se.filesystem.batch_forget(self, x.nodes()); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            ll::Operation::FAllocate(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.fallocate(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-21")]
            ll::Operation::ReadDirPlus(x) => {
                let reply = ReplyDirectoryPlus::new(self.reply);
                se.filesystem.readdirplus(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    ReplyDirectoryPlus::new(
                        self.request.unique().into(),
                        self.ch.clone(),
                        x.size() as usize,
                    ),
                );
            }
            #[cfg(feature = "abi-7-23")]
            ll::Operation::Rename2(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.rename(
                    self,
                    x.from().dir.into(),
                    x.from().name.as_ref(),
                    x.to().dir.into(),
                    x.to().name.as_ref(),
                    x.flags(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-24")]
            ll::Operation::Lseek(x) => {
                let reply = ReplyLseek::new(self.reply);
                se.filesystem.lseek(
                    self,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence(),
                    reply,
                );
            }
            #[cfg(feature = "abi-7-28")]
            ll::Operation::CopyFileRange(x) => {
                let reply = ReplyWrite::new(self.reply);
                let (i, o) = (x.src(), x.dest());
                se.filesystem.copy_file_range(
                    self,
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
            ll::Operation::SetVolName(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.setvolname(self, x.name(), reply);
            }
            #[cfg(target_os = "macos")]
            ll::Operation::GetXTimes(x) => {
                let reply = ReplyXTimes::new(self.reply);
                se.filesystem
                    .getxtimes(self, x.nodeid().into(), reply);
            }
            #[cfg(target_os = "macos")]
            ll::Operation::Exchange(x) => {
                let reply = ReplyEmpty::new(self.reply);
                se.filesystem.exchange(
                    self,
                    x.from().dir.into(),
                    x.from().name.as_ref(),
                    x.to().dir.into(),
                    x.to().name.as_ref(),
                    x.options(),
                    reply,
                );
            }

            ll::Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                return Err(Errno::ENOSYS);
            }
        }
        Ok(None)
    }
}