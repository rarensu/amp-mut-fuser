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

use crate::channel::ChannelSender;
use crate::ll::Request as _;
use crate::reply::ReplyHandler;
use crate::session::{SessionACL, SessionMeta};
use crate::Filesystem;
use crate::{ll, Forget, KernelConfig};

/// Request data structure
#[derive(Debug)]
pub struct RequestHandler {
    /// Request raw data
    #[allow(unused)]
    // data: Vec<u8>,
    /// Parsed request
    request: ll::AnyRequest,
    /// Request metadata
    meta: RequestMeta,
    /// Closure-like object to guarantee a response is sent
    replyhandler: ReplyHandler,
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
    pub(crate) fn new(ch: ChannelSender, data: Vec<u8>) -> Option<RequestHandler> {
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
        let replyhandler = ReplyHandler::new(request.unique().into(), ch.clone());
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
                let response = fs.lookup(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                match response {
                    Ok(entry) => {
                        self.replyhandler.entry(entry);
                    },
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Forget(x) => {
                let target = Forget {
                    ino: self.request.nodeid().into(),
                    nlookup: x.nlookup(),
                };
                fs.forget(self.meta, target).await; // no response
                self.replyhandler.no_reply(); // no reply
            }
            ll::Operation::GetAttr(_attr) => {
                #[cfg(feature = "abi-7-9")]
                let response = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(Into::into)
                ).await;
                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
                let response = fs.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    None,
                ).await;
                match response {
                    Ok((attr,ttl))=> {
                        self.replyhandler.attr(attr, ttl);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::SetAttr(x) => {
                let response = fs.setattr(
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
                match response {
                    Ok((attr, ttl))=> {
                        self.replyhandler.attr(attr, ttl);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::ReadLink(_) => {
                let response = fs.readlink(
                    self.meta,
                     self.request.nodeid().into()
                ).await;
                match response {
                    Ok(data)=> {
                        self.replyhandler.data(data);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::MkNod(x) => {
                let response = fs.mknod(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask(),
                    x.rdev()
                ).await;
                match response {
                    Ok(entry)=> {
                        self.replyhandler.entry(entry);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::MkDir(x) => {
                let response = fs.mkdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask()
                ).await;
                match response {
                    Ok(entry)=> {
                        self.replyhandler.entry(entry);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Unlink(x) => {
                let response = fs.unlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::RmDir(x) => {
                let response = fs.rmdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::SymLink(x) => {
                let response = fs.symlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.link_name(),
                    x.target()
                ).await;
                match response {
                    Ok(entry)=> {
                        self.replyhandler.entry(entry);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Rename(x) => {
                let response = fs.rename(
                    self.meta,
                    self.request.nodeid().into(),
                    x.src().name,
                    x.dest().dir.into(),
                    x.dest().name,
                    0
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Link(x) => {
                let response = fs.link(
                    self.meta,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name
                ).await;
                match response {
                    Ok(entry)=> {
                        self.replyhandler.entry(entry);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Open(x) => {
                let response = fs.open(
                    self.meta,
                    self.request.nodeid().into(), 
                    x.flags()
                ).await;
                match response {
                    Ok(open)=> {
                        self.replyhandler.opened(open);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Read(x) => {
                let response = fs.read(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(Into::into)
                ).await;
                match response {
                    Ok(data)=> {
                        self.replyhandler.data(data);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Write(x) => {
                let response = fs.write(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(Into::into)
                ).await;
                match response {
                    Ok(size)=> {
                        self.replyhandler.written(size);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Flush(x) => {
                let response = fs.flush(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Release(x) => {
                let response = fs.release(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(Into::into),
                    x.flush()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::FSync(x) => {
                let response = fs.fsync(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::OpenDir(x) => {
                let response = fs.opendir(
                    self.meta,
                    self.request.nodeid().into(), 
                    x.flags()
                ).await;
                match response {
                    Ok(open)=> {
                        self.replyhandler.opened(open);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::ReadDir(x) => {
                let response = fs.readdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size()
                ).await;
                match response {
                    Ok(dirent_list)=> {
                        self.replyhandler.dir(
                            &dirent_list,
                            x.size() as usize,
                            x.offset(),
                        );
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::ReleaseDir(x) => {
                let response = fs.releasedir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::FSyncDir(x) => {
                let response = fs.fsyncdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::StatFs(_) => {
                let response = fs.statfs(
                    self.meta,
                    self.request.nodeid().into()
                ).await;
                match response {
                    Ok(statfs)=> {
                        self.replyhandler.statfs(statfs);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::SetXAttr(x) => {
                let response = fs.setxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::GetXAttr(x) => {
                let response = fs.getxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32()
                ).await;
                match response {
                    Ok(xattr)=> {
                        self.replyhandler.xattr(xattr);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::ListXAttr(x) => {
                let response = fs.listxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.size()
                ).await;
                match response {
                    Ok(xattr)=> {
                        self.replyhandler.xattr(xattr);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::RemoveXAttr(x) => {
                let response = fs.removexattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Access(x) => {
                let response = fs.access(
                    self.meta,
                    self.request.nodeid().into(),
                    x.mask()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::Create(x) => {
                let response = fs.create(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.mode(),
                    x.umask(),
                    x.flags()
                ).await;
                match response {
                    Ok((entry, open))=> {
                        self.replyhandler.created(entry, open);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::GetLk(x) => {
                let response = fs.getlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid
                ).await;
                match response {
                    Ok(lock)=> {
                        self.replyhandler.locked(lock);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::SetLk(x) => {
                let response = fs.setlk(
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
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::SetLkW(x) => {
                let response = fs.setlk(
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
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            ll::Operation::BMap(x) => {
                let response = fs.bmap(
                    self.meta,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block()
                ).await;
                match response {
                    Ok(block)=> {
                        self.replyhandler.bmap(block);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }

            #[cfg(feature = "abi-7-11")]
            ll::Operation::IoCtl(x) => {
                if x.unrestricted() {
                    self.replyhandler.error(Errno::ENOSYS);
                } else {
                    let response = fs.ioctl(
                        self.meta,
                        self.request.nodeid().into(),
                        x.file_handle().into(),
                        x.flags(),
                        x.command(),
                        x.in_data(),
                        x.out_size()
                    ).await;
                    match response {
                        Ok(ioctl)=> {
                            self.replyhandler.ioctl(ioctl);
                        }
                        Err(err)=> {
                            self.replyhandler.error(err);
                        }
                    }
                }
            }
            #[cfg(feature = "abi-7-11")]
            ll::Operation::Poll(x) => {
                let response = fs.poll(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.kernel_handle(),
                    x.events(),
                    x.flags()
                ).await;
                match response {
                    Ok(ready_events)=> {
                        self.replyhandler.poll(ready_events);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(feature = "abi-7-15")]
            ll::Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                self.replyhandler.error(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            ll::Operation::BatchForget(x) => {
                fs.batch_forget(self.meta, x.into()).await; // no response
                self.replyhandler.no_reply(); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            ll::Operation::FAllocate(x) => {
                let response = fs.fallocate(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(feature = "abi-7-21")]
            ll::Operation::ReadDirPlus(x) => {
                let response = fs.readdirplus(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size()
                ).await;
                match response {
                    Ok(dirent_plus_list)=> {
                        self.replyhandler.dirplus(
                            &dirent_plus_list,
                            x.size() as usize,
                            x.offset()
                        );
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(feature = "abi-7-23")]
            ll::Operation::Rename2(x) => {
                let response = fs.rename(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name,
                    x.to().dir.into(),
                    x.to().name,
                    x.flags()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok();
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(feature = "abi-7-24")]
            ll::Operation::Lseek(x) => {
                let response = fs.lseek(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence()
                ).await;
                match response {
                    Ok(offset)=> {
                        self.replyhandler.offset(offset);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(feature = "abi-7-28")]
            ll::Operation::CopyFileRange(x) => {
                let (i, o) = (x.src(), x.dest());
                let response = fs.copy_file_range(
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
                match response {
                    Ok(written)=> {
                        self.replyhandler.written(written);
                    }
                    Err(err)=> {
                        self.replyhandler.error(err);
                    }
                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::SetVolName(x) => {
                let response = fs.setvolname(
                    self.meta,
                    x.name()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok()
                    }
                    Err(err)=> {
                        self.replyhandler.error(err)
                    }
                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::GetXTimes(x) => {
                let response = fs.getxtimes(
                    self.meta,
                    x.nodeid().into()
                ).await;
                match response {
                    Ok(xtimes)=> {
                        self.replyhandler.xtimes(xtimes)
                    }
                    Err(err)=> {
                        self.replyhandler.error(err)
                    }
                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::Exchange(x) => {
                let response = fs.exchange(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name,
                    x.to().dir.into(),
                    x.to().name,
                    x.options()
                ).await;
                match response {
                    Ok(())=> {
                        self.replyhandler.ok()
                    }
                    Err(err)=> {
                        self.replyhandler.error(err)
                    }
                }
            }

            #[cfg(feature = "abi-7-12")]
            ll::Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                self.replyhandler.error(Errno::ENOSYS);
            }
        }
    }
}
