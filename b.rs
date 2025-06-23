<!----- src/lib.rs ----->
fn lookup(&mut self, _req: RequestMeta, parent: u64, name: OsStr) -> Result<Entry, Errno> {
  return Err(ENOSYS);
}

<!----- examples/simple.rs ----->
fn lookup(&mut self, req: RequestMeta, parent: u64, name: OsStr) -> Result<Entry, Errno> {
  return Ok(Entry(attrs.into(), Duration::new(0, 0), 0))
}

<!----- src/reply.rs ----->






#[derive(Debug)]
pub(crate) struct ReplyHandler {
    /// Unique id of the request to reply to
    unique: ll::RequestId,
    /// Closure to call for sending the reply
    sender: Option<Box<dyn ReplySender>>,
}

impl ReplyHandler {
    fn new<S: ReplySender>(unique: u64, sender: S) -> ReplyHandler {
        let sender = Box::new(sender);
        ReplyHandler {
            unique: ll::RequestId(unique),
            sender: Some(sender),
        }
    }
}

impl ReplyHandler {
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

#[derive(Debug)]
pub struct Entry {
    /// Reply to a request with the given entry
    pub attr: FileAttr, 
    pub ttl: Duration, 
    pub generation: u64
}







impl ReplyHandler{
    /// Reply to a request with the given entry
    pub fn reply_entry(self, entry: Entry)  {
        self.send_ll(&ll::Response::new_entry(
            ll::INodeNo(entry.attr.ino),
            ll::Generation(entry.generation),
            &entry.attr.into(),
            *entry.ttl, 
            *entry.ttl,
        ));
    }
}

impl Drop for ReplyHandler {    
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
    /// Request metadata
    meta: RequestMeta,
    /// Closure-like object to guarantee sends a response
    replyhandler: ReplyHandler, // lifetime?
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
        let meta = RequestMeta{
            request.unique().into(),
            request.uid(),
            request.gid(),
            request.pid() 
        }
        let rs= ReplyHandler::new(request.unique().into(), ch)
        Some(Self { ch, data, request, meta, rs })
    }
    
    pub(crate) fn dispatch<FS: Filesystem>(&self, se: &mut Session<FS>) {    
        ...
        ll::Operation::Lookup(x) => {
            let reply = 
            se.filesystem.lookup(
                    self.meta,
                    self.request.nodeid().into(),
                    x.name().as_ref()
                    
            );
            match reply{
            Ok(entry) => self.replyhandler.reply_entry(entry),
            Err(errno) => self.replyhandler.reply_error(errno),
            }
        }
        ...
    }


/// Code section regarding Request metadata
}

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