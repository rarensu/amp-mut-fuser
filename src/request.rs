//! Filesystem operation request
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! TODO: This module is meant to go away soon in favor of `ll::Request`.

use log::{debug, error, warn};
use std::convert::TryFrom;
#[cfg(feature = "abi-7-28")]
use std::convert::TryInto;

use crate::channel::ChannelSender;
use crate::ll::Request as _;
use crate::reply::ReplyHandler;
use crate::ll;

/// Request data structure
#[derive(Debug)]
pub struct Request<'a> {
    /// Request raw data
    #[allow(unused)]
    data: &'a [u8],
    /// Parsed request
    request: ll::AnyRequest<'a>,
    /// Closure-like object to guarantee a response is sent
    pub reply: ReplyHandler,
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
        // Create a reply object for this request that can be passed to the filesystem
        // implementation and makes sure that a request is replied exactly once
        let reply = ReplyHandler::new(request.unique().into(), ch);
        Some(Self { data, request, reply })
    }

    /// Returns the unique identifier of this request
    #[inline]
    pub fn unique(&self) -> u64 {
        self.request.unique().into()
    }

    /// Returns the uid of this request
    #[inline]
    pub fn uid(&self) -> u32 {
        self.request.uid()
    }

    /// Returns the gid of this request
    #[inline]
    pub fn gid(&self) -> u32 {
        self.request.gid()
    }

    /// Returns the pid of this request
    #[inline]
    pub fn pid(&self) -> u32 {
        self.request.pid()
    }
}
