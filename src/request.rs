//! Filesystem operation request
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! TODO: This module is meant to go away soon in favor of `ll::Request`.

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::convert::Into;

use crate::channel::Channel;
use crate::ll::{AnyRequest, Operation, Request as RequestTrait};
use crate::reply::ReplyHandler;
use crate::session::{SessionACL, SessionMeta};

/// Request data structure
#[derive(Debug)]
pub(crate) struct RequestHandler {
    /// Parsed request
    pub request: AnyRequest,
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
    pub pid: u32,
}

#[derive(Copy, Clone, Debug, PartialEq)]
/// Target of a `forget` or `batch_forget` operation.
pub struct Forget {
    /// Inode of the file to be forgotten.
    pub ino: u64,
    /// The number of times the file has been looked up (and not yet forgotten).
    /// When a `forget` operation is received, the filesystem should typically
    /// decrement its internal reference count for the inode by `nlookup`.
    pub nlookup: u64,
}

impl RequestHandler {
    /// Create a new request from the given data
    pub(crate) fn new(ch: Channel, data: Vec<u8>) -> Option<RequestHandler> {
        let request = match AnyRequest::try_from(data) {
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
            pid: request.pid(),
        };
        let replyhandler = ReplyHandler::new(request.unique().into(), ch);
        Some(Self {
            request,
            meta,
            replyhandler,
        })
    }

    /// Implementation to allow_root & access check for auto_unmount
    pub(crate) fn access_denied(&self, op: &Operation<'_>, se_meta: &SessionMeta) -> bool {
        if (se_meta.allowed == SessionACL::RootAndOwner
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
        }
    }
}
