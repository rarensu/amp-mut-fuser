use crate::ll::{self, Errno, Operation, Request as AnyRequest, fuse_abi as abi};
use crate::request::RequestHandler;
use crate::session::SessionMeta;
use crate::{Forget, KernelConfig, SessionACL};
#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::convert::Into;
#[cfg(feature = "abi-7-28")]
use std::convert::TryInto;
use std::sync::atomic::Ordering::Relaxed;

use super::Filesystem;

impl RequestHandler {
    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub(crate) fn dispatch_sync<FS: Filesystem>(self, fs: &mut FS, se_meta: &SessionMeta) {
        debug!("{}", self.request);
        let op = if let Ok(op) = self.request.operation() {
            op
        } else {
            self.replyhandler.error(Errno::ENOSYS);
            return;
        };
        if self.access_denied(&op, se_meta) {
            self.replyhandler.error(Errno::EACCES);
            return;
        }
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
                // Encapsulate kernel capabilities into config object
                let config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to
                // propose a different config or return an error
                match fs.init(self.meta, config) {
                    Ok(config) => {
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
                        se_meta.initialized.store(true, Relaxed);
                        self.replyhandler.config(x.capabilities(), config);
                    }
                    Err(errno) => {
                        // Filesystem refused the config.
                        self.replyhandler.error(errno);
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
                let result = fs.lookup(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    /* blank space */
                );
                self.replyhandler.entry_or_err(result);
            }
            Operation::Forget(x) => {
                let target = Forget {
                    ino: self.request.nodeid().into(),
                    nlookup: x.nlookup(),
                };
                fs.forget(self.meta, target); // no response
                self.replyhandler.disable(); // no reply
            }
            Operation::GetAttr(_attr) => {
                #[cfg(feature = "abi-7-9")]
                let result = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(Into::into),
                    /* blank space */
                );
                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
                let result = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    None,
                    /* blank space */
                );
                self.replyhandler.attr_or_err(result);
            }
            Operation::SetAttr(x) => {
                let result = fs.setattr(
                    self.meta,
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
                );
                self.replyhandler.attr_or_err(result);
            }
            Operation::ReadLink(_) => {
                let result = fs.readlink(self.meta, self.request.nodeid().into());
                self.replyhandler.data_or_err(result);
            }
            Operation::MkNod(x) => {
                let result = fs.mknod(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.rdev(),
                );
                self.replyhandler.entry_or_err(result);
            }
            Operation::MkDir(x) => {
                let result = fs.mkdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                );
                self.replyhandler.entry_or_err(result);
            }
            Operation::Unlink(x) => {
                let result = fs.unlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    /* blank space */
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::RmDir(x) => {
                let result = fs.rmdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    /* blank space */
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::SymLink(x) => {
                let result = fs.symlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.link_name().as_os_str(),
                    x.target(),
                );
                self.replyhandler.entry_or_err(result);
            }
            Operation::Rename(x) => {
                let result = fs.rename(
                    self.meta,
                    self.request.nodeid().into(),
                    x.src().name.as_os_str(),
                    x.dest().dir.into(),
                    x.dest().name.as_os_str(),
                    0,
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::Link(x) => {
                let result = fs.link(
                    self.meta,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name.as_os_str(),
                );
                self.replyhandler.entry_or_err(result);
            }
            Operation::Open(x) => {
                let result = fs.open(
                    self.meta,
                    self.request.nodeid().into(),
                    x.flags(),
                    /* blank space */
                );
                self.replyhandler.opened_or_err(result);
            }
            Operation::Read(x) => {
                let result = fs.read(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                );
                self.replyhandler.data_or_err(result);
            }
            Operation::Write(x) => {
                let result = fs.write(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                );
                self.replyhandler.written_or_err(result);
            }
            Operation::Flush(x) => {
                let result = fs.flush(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::Release(x) => {
                let result = fs.release(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    x.flush(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::FSync(x) => {
                let result = fs.fsync(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::OpenDir(x) => {
                let result = fs.opendir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.flags(),
                    /* blank space */
                );
                self.replyhandler.opened_or_err(result);
            }
            Operation::ReadDir(x) => {
                let result = fs.readdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                );
                self.replyhandler.dir_or_err(
                    result,
                    x.offset(),
                    x.size() as usize,
                    /* blank space */
                );
            }
            Operation::ReleaseDir(x) => {
                let result = fs.releasedir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::FSyncDir(x) => {
                let result = fs.fsyncdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::StatFs(_) => {
                let result = fs.statfs(self.meta, self.request.nodeid().into());
                self.replyhandler.statfs_or_err(result);
            }
            Operation::SetXAttr(x) => {
                let result = fs.setxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position(),
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::GetXAttr(x) => {
                let result = fs.getxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32(),
                );
                self.replyhandler.xattr_or_err(result);
            }
            Operation::ListXAttr(x) => {
                let result = fs.listxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.size(),
                    /* blank space */
                );
                self.replyhandler.xattr_or_err(result);
            }
            Operation::RemoveXAttr(x) => {
                let result = fs.removexattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    /* blank space */
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::Access(x) => {
                let result = fs.access(
                    self.meta,
                    self.request.nodeid().into(),
                    x.mask(),
                    /* blank space */
                );
                match result {
                    Ok(()) => self.replyhandler.ok(),
                    Err(Errno::ENOSYS)
                        if (se_meta.allowed == SessionACL::Owner
                            || se_meta.allowed == SessionACL::RootAndOwner) =>
                    {
                        // Access was not denied (see above) so it is allowed.
                        self.replyhandler.ok();
                    }
                    Err(e) => self.replyhandler.error(e),
                }
            }
            Operation::Create(x) => {
                let result = fs.create(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_os_str(),
                    x.mode(),
                    x.umask(),
                    x.flags(),
                );
                self.replyhandler.created_or_err(result);
            }
            Operation::GetLk(x) => {
                let result = fs.getlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                );
                self.replyhandler.locked_or_err(result);
            }
            Operation::SetLk(x) => {
                let result = fs.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    false,
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::SetLkW(x) => {
                let result = fs.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    true,
                );
                self.replyhandler.ok_or_err(result);
            }
            Operation::BMap(x) => {
                let result = fs.bmap(
                    self.meta,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block(),
                );
                self.replyhandler.bmap_or_err(result);
            }

            #[cfg(feature = "abi-7-11")]
            Operation::IoCtl(x) => {
                if x.unrestricted() {
                    self.replyhandler.error(Errno::ENOSYS);
                } else {
                    let result = fs.ioctl(
                        self.meta,
                        self.request.nodeid().into(),
                        x.file_handle().into(),
                        x.flags(),
                        x.command(),
                        x.in_data(),
                        x.out_size(),
                    );
                    self.replyhandler.ioctl_or_err(result);
                }
            }
            #[cfg(feature = "abi-7-11")]
            Operation::Poll(x) => {
                let result = fs.poll(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.kernel_handle(),
                    x.events(),
                    x.flags(),
                );
                self.replyhandler.poll_or_err(result);
            }
            #[cfg(feature = "abi-7-15")]
            Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                self.replyhandler.error(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            Operation::BatchForget(x) => {
                fs.batch_forget(self.meta, x.into()); // no response
                self.replyhandler.disable(); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            Operation::FAllocate(x) => {
                let result = fs.fallocate(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode(),
                );
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(feature = "abi-7-21")]
            Operation::ReadDirPlus(x) => {
                let result = fs.readdirplus(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                );
                self.replyhandler.dirplus_or_err(
                    result,
                    x.offset(),
                    x.size() as usize,
                    /* blank space */
                );
            }
            #[cfg(feature = "abi-7-23")]
            Operation::Rename2(x) => {
                let result = fs.rename(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name.as_os_str(),
                    x.to().dir.into(),
                    x.to().name.as_os_str(),
                    x.flags(),
                );
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(feature = "abi-7-24")]
            Operation::Lseek(x) => {
                let result = fs.lseek(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence(),
                );
                self.replyhandler.offset_or_err(result);
            }
            #[cfg(feature = "abi-7-28")]
            Operation::CopyFileRange(x) => {
                let (i, o) = (x.src(), x.dest());
                let result = fs.copy_file_range(
                    self.meta,
                    i.inode.into(),
                    i.file_handle.into(),
                    i.offset,
                    o.inode.into(),
                    o.file_handle.into(),
                    o.offset,
                    x.len(),
                    x.flags().try_into().unwrap(),
                );
                self.replyhandler.written_or_err(result);
            }
            #[cfg(target_os = "macos")]
            Operation::SetVolName(x) => {
                let result = fs.setvolname(self.meta, x.name());
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(target_os = "macos")]
            Operation::GetXTimes(x) => {
                let result = fs.getxtimes(self.meta, x.nodeid().into());
                self.replyhandler.xtimes_or_err(result);
            }
            #[cfg(target_os = "macos")]
            Operation::Exchange(x) => {
                let result = fs.exchange(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name,
                    x.to().dir.into(),
                    x.to().name,
                    x.options(),
                );
                self.replyhandler.ok_or_err(result);
            }

            #[cfg(feature = "abi-7-12")]
            Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                self.replyhandler.error(Errno::ENOSYS);
            }
        }
    }
}
