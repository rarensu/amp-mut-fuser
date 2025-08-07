//! Filesystem operation request
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! TODO: This module is meant to go away soon in favor of `ll::Request`.

use crate::ll::{fuse_abi as abi, Errno};
#[allow(unused_imports)]
use log::{debug, info, warn, error};
use std::convert::{TryFrom, Into};
#[cfg(feature = "abi-7-28")]
use std::convert::TryInto;
use std::sync::{Arc, Mutex};

use crate::channel::Channel;
use crate::ll::Request as _;
use crate::reply::ReplyHandler;
use crate::session::{SessionACL, SessionMeta};
use crate::Filesystem;
use crate::{ll, Forget, KernelConfig};

/// Request data structure
#[derive(Debug)]
pub(crate) struct RequestHandler {
    /// Parsed request
    pub request: ll::AnyRequest,
    /// Request metadata
    pub meta: RequestMeta,
    /// Closure-like object to guarantee a response is sent
    pub replyhandler: ReplyHandler,
}

/// Request metadata structure
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct RequestMeta {
    /// The unique identifier of this request
    pub unique: u64,
    /// The uid of this request
    pub uid: u32,
    /// The gid of this request
    pub gid: u32,
    /// The pid of this request
    pub pid: u32
}

impl RequestHandler {
    /// Create a new request from the given data
    pub(crate) fn new(ch: Channel, data: Vec<u8>) -> Option<RequestHandler> {
        let request = match ll::AnyRequest::try_from(data) {
            Ok(request) => request,
            Err(err) => {
                error!("{err}");
                return None;
            }
        };

        let meta = RequestMeta {
            unique: request.unique().into(),
            uid: request.uid(),
            gid: request.gid(),
            pid: request.pid()
        };
        let replyhandler = ReplyHandler::new(request.unique().into(), ch);
        Some(Self { request, meta, replyhandler })
    }

    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub(crate) async fn dispatch<FS: Filesystem>(self, fs: Arc<FS>, meta: Arc<Mutex<SessionMeta>>) {
        debug!("{}", self.request);
        let op_result = self.request.operation().map_err(|_| Errno::ENOSYS);

        if let Err(err) = op_result {
            self.replyhandler.error(err);
            return;
        }
        let op = op_result.unwrap();

        // Implement allow_root & access check for auto_unmount
        let access_denied = false; 
        {
            match meta.lock() {
                Ok(meta) => {
                    if (meta.allowed == SessionACL::RootAndOwner
                        && self.request.uid() != meta.session_owner
                        && self.request.uid() != 0)
                        || (meta.allowed == SessionACL::Owner && self.request.uid() != meta.session_owner)
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
                            | ll::Operation::ReleaseDir(_) => false,
                            #[cfg(feature = "abi-7-16")]
                            ll::Operation::BatchForget(_) => false,
                            #[cfg(feature = "abi-7-21")]
                            ll::Operation::ReadDirPlus(_) => false,
                            _ => true,
                        }
                    } else {
                        false
                    };
                }
                Err(e) => {
                    error!("{e:?}");
                    self.replyhandler.error(Errno::EDEADLK);
                    return;
                }
            }
        }
        if access_denied {
            self.replyhandler.error(Errno::EACCES);
            return;
        }
        // Gathering some additional info before matching the op code
        let (initialized, destroyed) = match meta.lock() {
            Ok(meta) => {
                (meta.initialized, meta.destroyed)
            }
            Err(e) => {
                error!("{e:?}");
                self.replyhandler.error(Errno::EDEADLK);
                return;
            }
        };

        match op {
            // Filesystem initialization
            ll::Operation::Init(x) => {
                // We don't support ABI versions before 7.6
                let v = x.version();
                if v < ll::Version(7, 6) {
                    error!("Unsupported FUSE ABI version {v}");
                    self.replyhandler.error(Errno::EPROTO);
                    return;
                }
                // Remember ABI version supported by kernel
                {
                    match meta.lock() {
                        Ok(mut meta) => {
                            meta.proto_major = v.major();
                            meta.proto_minor = v.minor();
                        }
                        Err(e) => {
                            error!("{e:?}");
                            self.replyhandler.error(Errno::EDEADLK);
                            return;
                        }
                    }
                }
                let config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to 
                // propose a different config or return an error
                match fs.init(self.meta, config).await {
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
                        {
                            match meta.lock() {
                                Ok(mut meta) => {
                                    meta.initialized = true;
                                }
                                Err(e) => {
                                    error!("{e:?}");
                                    self.replyhandler.error(Errno::EDEADLK);
                                    return;
                                }
                            }
                        }
                        self.replyhandler.config(x.capabilities(), config);
                    },
                    Err(errno) => {
                        // Filesystem refused the config.
                        self.replyhandler.error(errno);
                    }
                }
            }
            // Any operation is invalid before initialization
            _ if !initialized => {
                warn!("Ignoring FUSE operation before init: {}", self.request);
                self.replyhandler.error(Errno::EIO);
            }
            // Filesystem destroyed
            ll::Operation::Destroy(_x) => {
                fs.destroy().await;
                {
                    match meta.lock() {
                        Ok(mut meta) => {
                            meta.destroyed = true;
                        }
                        Err(e) => {
                            error!("{e:?}");
                            self.replyhandler.error(Errno::EDEADLK);
                            return;
                        }
                    }
                }
                self.replyhandler.ok();
            }
            // Any operation is invalid after destroy
            _ if destroyed => {
                warn!("Ignoring FUSE operation after destroy: {}", self.request);
                self.replyhandler.error(Errno::EIO);
            }
            ll::Operation::Interrupt(_) => {
                // TODO: handle FUSE_INTERRUPT
                self.replyhandler.error(Errno::ENOSYS);
            }

            ll::Operation::Lookup(x) => {
                let result = fs.lookup(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                self.replyhandler.entry_or_err(result);
            }
            ll::Operation::Forget(x) => {
                let target = Forget {
                    ino: self.request.nodeid().into(),
                    nlookup: x.nlookup(),
                };
                fs.forget(self.meta, target).await; // no response
                self.replyhandler.disable(); // no reply
            }
            ll::Operation::GetAttr(_attr) => {
                #[cfg(feature = "abi-7-9")]
                let result = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(Into::into)
                ).await;
                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
                let result = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    None,
                ).await;
                self.replyhandler.attr_or_err(result);
            }
            ll::Operation::SetAttr(x) => {
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
                    x.flags()
                ).await;
                self.replyhandler.attr_or_err(result);
            }
            ll::Operation::ReadLink(_) => {
                let result = fs.readlink(
                    self.meta,
                    self.request.nodeid().into()
                ).await;
                self.replyhandler.data_or_err(result);
            }
            ll::Operation::MkNod(x) => {
                let result = fs.mknod(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask(),
                    x.rdev()
                ).await;
                self.replyhandler.entry_or_err(result);
            }
            ll::Operation::MkDir(x) => {
                let result = fs.mkdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask()
                ).await;
                self.replyhandler.entry_or_err(result);
            }
            ll::Operation::Unlink(x) => {
                let result = fs.unlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::RmDir(x) => {
                let result = fs.rmdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::SymLink(x) => {
                let result = fs.symlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.link_name(),
                    x.target()
                ).await;
                self.replyhandler.entry_or_err(result);
            }
            ll::Operation::Rename(x) => {
                let result = fs.rename(
                    self.meta,
                    self.request.nodeid().into(),
                    x.src().name,
                    x.dest().dir.into(),
                    x.dest().name,
                    0
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::Link(x) => {
                let result = fs.link(
                    self.meta,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name
                ).await;
                self.replyhandler.entry_or_err(result);
            }
            ll::Operation::Open(x) => {
                let result = fs.open(
                    self.meta,
                    self.request.nodeid().into(), 
                    x.flags()
                ).await;
                self.replyhandler.opened_or_err(result);
            }
            ll::Operation::Read(x) => {
                let result = fs.read(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(Into::into)
                ).await;
                self.replyhandler.data_or_err(result);
            }
            ll::Operation::Write(x) => {
                let result = fs.write(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(Into::into)
                ).await;
                self.replyhandler.written_or_err(result);
            }
            ll::Operation::Flush(x) => {
                let result = fs.flush(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::Release(x) => {
                let result = fs.release(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    x.flush()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::FSync(x) => {
                let result = fs.fsync(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::OpenDir(x) => {
                let result = fs.opendir(
                    self.meta,
                    self.request.nodeid().into(), 
                    x.flags()
                ).await;
                self.replyhandler.opened_or_err(result);
            }
            ll::Operation::ReadDir(x) => {
                let result = fs.readdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size()
                ).await;
                self.replyhandler.dir_or_err(
                    result,
                    x.size() as usize,
                    x.offset(),
                );
            }
            ll::Operation::ReleaseDir(x) => {
                let result = fs.releasedir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::FSyncDir(x) => {
                let result = fs.fsyncdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::StatFs(_) => {
                let result = fs.statfs(
                    self.meta,
                    self.request.nodeid().into()
                ).await;
                self.replyhandler.statfs_or_err(result);
            }
            ll::Operation::SetXAttr(x) => {
                let result = fs.setxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::GetXAttr(x) => {
                let result = fs.getxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32()
                ).await;
                self.replyhandler.xattr_or_err(result);
            }
            ll::Operation::ListXAttr(x) => {
                let result = fs.listxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.size()
                ).await;
                self.replyhandler.xattr_or_err(result);
            }
            ll::Operation::RemoveXAttr(x) => {
                let result = fs.removexattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::Access(x) => {
                let result = fs.access(
                    self.meta,
                    self.request.nodeid().into(),
                    x.mask()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::Create(x) => {
                let result = fs.create(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask(),
                    x.flags()
                ).await;
                self.replyhandler.created_or_err(result);
            }
            ll::Operation::GetLk(x) => {
                let result = fs.getlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid
                ).await;
                self.replyhandler.locked_or_err(result);
            }
            ll::Operation::SetLk(x) => {
                let result = fs.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    false
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::SetLkW(x) => {
                let result = fs.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    true
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            ll::Operation::BMap(x) => {
                let result = fs.bmap(
                    self.meta,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block()
                ).await;
                self.replyhandler.bmap_or_err(result);
            }

            #[cfg(feature = "abi-7-11")]
            ll::Operation::IoCtl(x) => {
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
                        x.out_size()
                    ).await;
                    self.replyhandler.ioctl_or_err(result);
                }
            }
            #[cfg(feature = "abi-7-11")]
            ll::Operation::Poll(x) => {
                let result = fs.poll(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.kernel_handle(),
                    x.events(),
                    x.flags()
                ).await;
                self.replyhandler.poll_or_err(result);
            }
            #[cfg(feature = "abi-7-15")]
            ll::Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                self.replyhandler.error(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            ll::Operation::BatchForget(x) => {
                fs.batch_forget(self.meta, x.into()).await; // no response
                self.replyhandler.disable(); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            ll::Operation::FAllocate(x) => {
                let result = fs.fallocate(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(feature = "abi-7-21")]
            ll::Operation::ReadDirPlus(x) => {
                let result = fs.readdirplus(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size()
                ).await;
                self.replyhandler.dirplus_or_err(
                    result,
                    x.size() as usize,
                    x.offset()
                );
            }
            #[cfg(feature = "abi-7-23")]
            ll::Operation::Rename2(x) => {
                let result = fs.rename(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name,
                    x.to().dir.into(),
                    x.to().name,
                    x.flags()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(feature = "abi-7-24")]
            ll::Operation::Lseek(x) => {
                let result = fs.lseek(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence()
                ).await;
                self.replyhandler.offset_or_err(result);
            }
            #[cfg(feature = "abi-7-28")]
            ll::Operation::CopyFileRange(x) => {
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
                    x.flags().try_into().unwrap()
                ).await;
                self.replyhandler.written_or_err(result);
            }
            #[cfg(target_os = "macos")]
            ll::Operation::SetVolName(x) => {
                let result = fs.setvolname(
                    self.meta,
                    x.name()
                ).await;
                self.replyhandler.ok_or_err(result);
            }
            #[cfg(target_os = "macos")]
            ll::Operation::GetXTimes(x) => {
                let result = fs.getxtimes(
                    self.meta,
                    x.nodeid().into()
                ).await;
                self.replyhandler.xtimes_or_err(result);
            }
            #[cfg(target_os = "macos")]
            ll::Operation::Exchange(x) => {
                let result = fs.exchange(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name,
                    x.to().dir.into(),
                    x.to().name,
                    x.options()
                ).await;
                self.replyhandler.ok_or_err(result);
            }

            #[cfg(feature = "abi-7-12")]
            ll::Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                self.replyhandler.error(Errno::ENOSYS);
            }
        }
    }
}
