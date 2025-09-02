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
use crate::notify::NotificationHandler;
use crate::reply::ReplyHandler;

/// Request data structure
#[derive(Debug)]
pub(crate) struct RequestHandler {
    /// Parsed request
    pub request: AnyRequest,
    /// Request metadata
    pub meta: RequestMeta,
    /// Closure-like object to guarantee a response is sent
    pub replyhandler: ReplyHandler<Channel>,
    #[cfg(feature = "abi-7-40")]
    /// Closure-like object to enable opening and closing of Backing Id.
    pub ch_main: Channel,
    /// Closure-like object to enable sending of notifications.
    pub notificationhandler: NotificationHandler<Channel>,
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
        let notificationhandler = NotificationHandler::new(ch_main.clone());
        #[cfg(feature = "abi-7-40")]
        let ch_main_clone = ch_main.clone();
        let replyhandler = ReplyHandler::new(request.unique().into(), ch_main);
        Some(Self {
            request,
            meta,
            replyhandler,
            #[cfg(feature = "abi-7-40")]
            ch_main: ch_main_clone,
            notificationhandler,
        })
    }
}

#[cfg(feature = "abi-7-40")]
macro_rules! get_backing_handler {
    ($me:ident) => {
        crate::passthrough::BackingHandler::new($me.ch_main)
    };
}
#[cfg(feature = "abi-7-40")]
pub(crate) use get_backing_handler;
