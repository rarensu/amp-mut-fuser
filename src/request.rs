//! Filesystem operation request handler
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! The handler ensures the private data remains owned while the request is being processed.

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::convert::Into;

use crate::channel::{ChannelSender};
use crate::ll::{AnyRequest, Request as RequestTrait};
use crate::reply::ReplyHandler;

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
    pub(crate) fn new(sender: ChannelSender, data: Vec<u8>) -> Option<RequestHandler> {
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
        let replyhandler = ReplyHandler::new(request.unique().into(), sender);
        Some(Self {
            request,
            meta,
            replyhandler,
        })
    }
}
