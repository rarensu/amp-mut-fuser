<!----- src/lib.rs ----->
fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
  reply.error(ENOSYS);
}

<!----- examples/simple.rs ----->
fn lookup(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
  reply.entry(&Duration::new(0, 0), &attrs.into(), 0)
}

<!----- src/reply.rs ----->
/// Generic reply trait
pub trait Reply {
    /// Create a new reply for the given request
    fn new<S: ReplySender>(unique: u64, sender: S) -> Self;
}

#[derive(Debug)]
pub(crate) struct ReplyRaw {
    /// Unique id of the request to reply to
    unique: ll::RequestId,
    /// Closure to call for sending the reply
    sender: Option<Box<dyn ReplySender>>,
}

impl Reply for ReplyRaw {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyRaw {
        let sender = Box::new(sender);
        ReplyRaw {
            unique: ll::RequestId(unique),
            sender: Some(sender),
        }
    }
}

impl ReplyRaw {
    /// Reply to a request with the given error code and data. Must be called
    /// only once (the `ok` and `error` methods ensure this by consuming `self`)
    fn send_ll_mut(&mut self, response: &ll::Response<'_>) {
        assert!(self.sender.is_some());
        let sender = self.sender.take().unwrap();
        let res = response.with_iovec(self.unique, |iov| sender.send(iov));
        if let Err(err) = res {
            error!("Failed to send FUSE reply: {}", err);
        }
    }
    fn send_ll(mut self, response: &ll::Response<'_>) {
        self.send_ll_mut(response)
    }

    /// Reply to a request with the given error code
    pub fn error(self, err: c_int) {
        assert_ne!(err, 0);
        self.send_ll(&ll::Response::new_error(ll::Errno::from_i32(err)));
    }
}

/// Code section regarding Entry replies







impl Reply for ReplyEntry {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyEntry {
        ReplyEntry {
            reply: Reply::new(unique, sender),
        }
    }
}

impl ReplyEntry {
    /// Reply to a request with the given entry
    pub fn entry(self, ttl: &Duration, attr: &FileAttr, generation: u64) {
        self.reply.send_ll(&ll::Response::new_entry(
            ll::INodeNo(attr.ino),
            ll::Generation(generation),
            &attr.into(),
            *ttl,
            *ttl,
        ));
    }
}

impl Drop for ReplyRaw {
    fn drop(&mut self) {
        if self.sender.is_some() {
            warn!(
                "Reply not sent for operation {}, replying with I/O error",
                self.unique.0
            );
            self.send_ll_mut(&ll::Response::new_error(ll::Errno::EIO));
        }
    }
}

<!---- src/request.rs ---->
pub struct Request<'a> {
    /// Channel sender for sending the reply
    ch: ChannelSender,
    /// Request raw data
    #[allow(unused)]
    data: &'a [u8],
    /// Parsed request
    request: ll::AnyRequest<'a>,




}

impl<'a> Request<'a> {
    pub(crate) fn new(ch: ChannelSender, data: &'a [u8]) -> Option<Request<'a>> {
        let request = match ll::AnyRequest::try_from(data) {
            Ok(request) => request,
            Err(err) => {
                error!("{}", err);
                return None;
            }
        };







        Some(Self { ch, data, request })
    }
 
  pub(crate) fn dispatch<FS: Filesystem>(&self, se: &mut Session<FS>) {
    ...
      ll::Operation::Lookup(x) => {

         se.filesystem.lookup(
                self,
                self.request.nodeid().into(),
                x.name().as_ref(),
                self.reply(),
         );



      }
      ...
  }


  /// Code section regarding Request metadata

  fn reply<T: Reply>(&self) -> T {
      Reply::new(self.request.unique().into(), self.ch.clone())
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