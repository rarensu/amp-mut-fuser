//! Filesystem operation request
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! TODO: This module is meant to go away soon in favor of `ll::Request`.

use crate::ll::{fuse_abi as abi, Errno, Response};
use log::{debug, error, warn};
use std::convert::TryFrom;
#[cfg(feature = "abi-7-28")]
use std::convert::TryInto;
use std::path::Path;

use crate::channel::ChannelSender;
use crate::ll::Request as _;
#[cfg(feature = "abi-7-21")]
use crate::reply::ReplyDirectoryPlus;
use crate::reply::{Reply, ReplyDirectory, ReplySender};
use crate::session::{Session, SessionACL};
use crate::Filesystem;
#[cfg(feature = "abi-7-11")]
use crate::PollHandle;
use crate::{ll, KernelConfig};

/// Request data structure
#[derive(Debug)]
pub struct Request<'a> {
    /// Channel sender for sending the reply
    ch: ChannelSender,
    /// Request raw data
    #[allow(unused)]
    data: &'a [u8], // TODO Vec<u8>
    /// Parsed request
    request: ll::AnyRequest<'a>,
    /// Request metadata
    meta: RequestMeta,
    /// Closure-like object to guarantee a response is sent
    replyhandler: ReplyHandler,
}

/// Request metadata structure
#[derive(Copy, Clone, Debug)]
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

impl<'a> Request<'a> {
    /// Create a new request from the given data
    pub(crate) fn new(ch: ChannelSender, data: &'a [u8]) -> Option<Request<'a>> {
        let request = match ll::AnyRequest::try_from(data) {
            Ok(request) => request,
            Err(err) => {
                error!("{}", err);
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
        Some(Self { ch, data, request, meta, replyhandler })
    }

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

                let config = KernelConfig::new(x.capabilities(), x.max_readahead());
                // Call filesystem init method and give it a chance to 
                // propose a different config or return an error
                let config = se.filesystem
                    .init(self.meta, config)
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
                let response =
                se.filesystem.lookup(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name()
                );
                match response {
                    Ok(entry) => {
                        self.replyhandler.entry(entry)
                    },
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Forget(x) => {
                se.filesystem
                    .forget(self.meta, self.request.nodeid().into(), x.nlookup()); // no reply
            }
            ll::Operation::GetAttr(_attr) => {
                #[cfg(feature = "abi-7-9")]
                let response =
                se.filesystem.getattr(
                    self.meta,
                    self.request.nodeid().into(),
                    _attr.file_handle().map(|fh| fh.into()),
                    self.reply(),
                );
                match response {
                    Ok(attr)=> {

                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }

                // Pre-abi-7-9 does not support providing a file handle.
                #[cfg(not(feature = "abi-7-9"))]
let response =
                se.filesystem
                    .getattr(self.meta, self.request.nodeid().into(), None, self.reply());
            }
            ll::Operation::SetAttr(x) => {
                let response =
            
                se.filesystem.setattr(
                    self.meta,
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
                    self.reply(),
                );
                match response {
                    Ok()=> {

                    }
                    Err(err)=>{
                    self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::ReadLink(_) => {
                let response =
                se.filesystem
                    .readlink(self.meta, self.request.nodeid().into(), self.reply());
match response {
Ok()=> {
self.replyhandler
}
Err(err)=>{
self.replyhandler.error(error)
}
}
            }
            ll::Operation::MkNod(x) => {
                let response =

                se.filesystem.mknod(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    x.rdev(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::MkDir(x) => {
                let response =
                se.filesystem.mkdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                    self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Unlink(x) => {
                let response =
                se.filesystem.unlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    self.reply(),
                );
                match response {
                    Ok(attr)=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                    self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::RmDir(x) => {
                let response =
                se.filesystem.rmdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                    self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::SymLink(x) => {
                let response =
                se.filesystem.symlink(
                    self.meta,
                    self.request.nodeid().into(),
                    x.link_name().as_ref(),
                    Path::new(x.target()),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Rename(x) => {
                let response =
                se.filesystem.rename(
                    self.meta,
                    self.request.nodeid().into(),
                    x.src().name.as_ref(),
                    x.dest().dir.into(),
                    x.dest().name.as_ref(),
                    0,
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Link(x) => {
                let response =
                se.filesystem.link(
                    self.meta,
                    x.inode_no().into(),
                    self.request.nodeid().into(),
                    x.dest().name.as_ref(),
                    self.reply(),
                );
                match response {
                    Ok(attr)=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Open(x) => {
                let response =
                se.filesystem
                    .open(self.meta, self.request.nodeid().into(), x.flags(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Read(x) => {
                let response =
                se.filesystem.read(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.size(),
                    x.flags(),
                    x.lock_owner().map(|l| l.into()),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Write(x) => {
                let response =
                se.filesystem.write(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.data(),
                    x.write_flags(),
                    x.flags(),
                    x.lock_owner().map(|l| l.into()),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Flush(x) => {
                let response =
                se.filesystem.flush(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    self.reply(),
                );
                match response {
                    Ok(attr)=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Release(x) => {
                let response =
                se.filesystem.release(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    x.lock_owner().map(|x| x.into()),
                    x.flush(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::FSync(x) => {
                let response =
                se.filesystem.fsync(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::OpenDir(x) => {
                let response =
                se.filesystem
                    .opendir(self.meta, self.request.nodeid().into(), x.flags(), self.reply());
                match response {
                    Ok(attr)=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::ReadDir(x) => {
                let response =
                se.filesystem.readdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    ReplyDirectory::new(
                        self.request.unique().into(),
                        self.ch.clone(),
                        x.size() as usize,
                    ),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::ReleaseDir(x) => {
                let response =
                se.filesystem.releasedir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.flags(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::FSyncDir(x) => {
                let response =
                se.filesystem.fsyncdir(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.fdatasync(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::StatFs(_) => {
                let response =
                se.filesystem
                    .statfs(self.meta, self.request.nodeid().into(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::SetXAttr(x) => {
                let response =
                se.filesystem.setxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.value(),
                    x.flags(),
                    x.position(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::GetXAttr(x) => {
                let response =
                se.filesystem.getxattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    x.size_u32(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::ListXAttr(x) => {
                let response =
                se.filesystem
                    .listxattr(self.meta, self.request.nodeid().into(), x.size(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::RemoveXAttr(x) => {
                let response =
                se.filesystem.removexattr(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Access(x) => {
                let response =
                se.filesystem
                    .access(self.meta, self.request.nodeid().into(), x.mask(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::Create(x) => {
                let response =
                se.filesystem.create(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref(),
                    x.mode(),
                    x.umask(),
                    x.flags(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::GetLk(x) => {
                let response =
                se.filesystem.getlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::SetLk(x) => {
                let response =
                se.filesystem.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    false,
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::SetLkW(x) => {
                let response =
                se.filesystem.setlk(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.lock_owner().into(),
                    x.lock().range.0,
                    x.lock().range.1,
                    x.lock().typ,
                    x.lock().pid,
                    true,
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            ll::Operation::BMap(x) => {
                let response =
                se.filesystem.bmap(
                    self.meta,
                    self.request.nodeid().into(),
                    x.block_size(),
                    x.block(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }

            #[cfg(feature = "abi-7-11")]
            ll::Operation::IoCtl(x) => {
                if x.unrestricted() {
                    return Err(Errno::ENOSYS);
                } else {
                    let response =
                    se.filesystem.ioctl(
                        self.meta,
                        self.request.nodeid().into(),
                        x.file_handle().into(),
                        x.flags(),
                        x.command(),
                        x.in_data(),
                        x.out_size(),
                        self.reply(),
                    );
                    match response {
                        Ok(attr)=> {
                            self.replyhandler.
                        }
                        Err(err)=>{
                            self.replyhandler.error(error)
                        }
                    }
                }
            }
            #[cfg(feature = "abi-7-11")]
            ll::Operation::Poll(x) => {
                let ph = PollHandle::new(se.ch.sender(), x.kernel_handle());

                let response =
                se.filesystem.poll(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    ph,
                    x.events(),
                    x.flags(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(feature = "abi-7-15")]
            ll::Operation::NotifyReply(_) => {
                // TODO: handle FUSE_NOTIFY_REPLY
                return Err(Errno::ENOSYS);
            }
            #[cfg(feature = "abi-7-16")]
            ll::Operation::BatchForget(x) => {
                se.filesystem.batch_forget(self, x.nodes()); // no reply
            }
            #[cfg(feature = "abi-7-19")]
            ll::Operation::FAllocate(x) => {
                let response =
                se.filesystem.fallocate(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.len(),
                    x.mode(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(feature = "abi-7-21")]
            ll::Operation::ReadDirPlus(x) => {
                let response =
                se.filesystem.readdirplus(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    ReplyDirectoryPlus::new(
                        self.request.unique().into(),
                        self.ch.clone(),
                        x.size() as usize,
                    ),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(feature = "abi-7-23")]
            ll::Operation::Rename2(x) => {
                let response =
                se.filesystem.rename(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name.as_ref(),
                    x.to().dir.into(),
                    x.to().name.as_ref(),
                    x.flags(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(feature = "abi-7-24")]
            ll::Operation::Lseek(x) => {
                let response =
                se.filesystem.lseek(
                    self.meta,
                    self.request.nodeid().into(),
                    x.file_handle().into(),
                    x.offset(),
                    x.whence(),
                    self.reply(),
                );
                match response {
                    Ok(attr)=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(feature = "abi-7-28")]
            ll::Operation::CopyFileRange(x) => {
                let (i, o) = (x.src(), x.dest());
                let response =
                se.filesystem.copy_file_range(
                    self.meta,
                    i.inode.into(),
                    i.file_handle.into(),
                    i.offset,
                    o.inode.into(),
                    o.file_handle.into(),
                    o.offset,
                    x.len(),
                    x.flags().try_into().unwrap(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::SetVolName(x) => {
                let response =
                se.filesystem.setvolname(self.meta, x.name(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::GetXTimes(x) => {
                let response =
                se.filesystem
                    .getxtimes(self.meta, x.nodeid().into(), self.reply());
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }

                }
            }
            #[cfg(target_os = "macos")]
            ll::Operation::Exchange(x) => {
                let response =
                se.filesystem.exchange(
                    self.meta,
                    x.from().dir.into(),
                    x.from().name.as_ref(),
                    x.to().dir.into(),
                    x.to().name.as_ref(),
                    x.options(),
                    self.reply(),
                );
                match response {
                    Ok()=> {
                        self.replyhandler.
                    }
                    Err(err)=>{
                        self.replyhandler.error(error)
                    }
                }
            }

            #[cfg(feature = "abi-7-12")]
            ll::Operation::CuseInit(_) => {
                // TODO: handle CUSE_INIT
                return Err(Errno::ENOSYS);
            }
        }
        Ok(None)
    }
}
