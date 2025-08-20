//! Filesystem operation request handler
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! The handler ensures the private data remains owned while the request is being processed.

#[allow(unused_imports)]
use log::{debug, error, info, warn};
use std::convert::Into;

use crate::channel::Channel;
use crate::ll::{AnyRequest, Request as RequestTrait};
use crate::reply::ReplyHandler;
#[cfg(feature = "abi-7-40")]
use crate::passthrough::BackingHandler;
#[cfg(feature = "abi-7-11")]
use crate::notify::NotificationHandler;

/// Request data structure
#[derive(Debug)]
pub(crate) struct RequestHandler {
    /// Parsed request
    pub request: AnyRequest,
    /// Request metadata
    pub meta: RequestMeta,
    /// Closure-like object to guarantee a response is sent
    pub replyhandler: ReplyHandler,
    #[cfg(feature = "abi-7-40")]
    /// Closure-like object to enable opening and closing of Backing Id.
    pub ch_main: Channel,
    #[cfg(feature = "abi-7-11")]
    /// Closure-like object to enable sending of notifications.
    pub notificationhandler: crate::trait_legacy::LegacyNotifier,
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
    /// Create a new request from the given data, and a Channel to receive the reply
    pub(crate) fn new(ch_main: Channel, data: Vec<u8>) -> Option<RequestHandler> {
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
        #[cfg(feature = "abi-7-11")]
        let notificationhandler = crate::trait_legacy::LegacyNotifier::new(sender.clone());
        #[cfg(feature = "abi-7-40")]
        let another_ch_main = ch_main.clone();
        let replyhandler = ReplyHandler::new(request.unique().into(), ch_main);        
        Some(Self {
            request,
            meta,
            replyhandler,
            #[cfg(feature = "abi-7-40")]
            ch_main,
            #[cfg(feature = "abi-7-11")]
            notificationhandler,
        })
    }
}

#[cfg(feature = "abi-7-40")]
macro_rules! get_backing_handler {
    ($me:ident) => {
        crate::passthrough::BackingHandler::new($me.ch_main, $me.queue)
    }
}
#[cfg(feature = "abi-7-40")]
pub(crate) use get_backing_handler;