use crossbeam_channel::{Sender, Receiver, unbounded};
use std::fmt;

#[derive(Clone, Debug)]
pub struct InternalChannel {
    stop_out: Sender<()>,
    stop_in: Receiver<()>,
    #[cfg(feature = "abi-7-40")]
    open_backing_out: Sender<(u32, Sender<io::Result<BackingId>>)>,
    #[cfg(feature = "abi-7-40")]
    open_backing_in: Receiver<(u32, Sender<io::Result<BackingId>>)>,
    #[cfg(feature = "abi-7-40")]
    close_backing_out: Sender<(u32, Sender<io::Result<u32>>)>,
    #[cfg(feature = "abi-7-40")]
    close_backing_in: Receiver<(u32, Sender<io::Result<u32>>)>,
}

impl InternalChannel {
    pub fn new() -> Self {
        let (stop_out, stop_in) = unbounded();
        #[cfg(feature = "abi-7-40")]
        let (open_backing_out, open_backing_in) = unbounded();
        #[cfg(feature = "abi-7-40")]
        let (close_backing_out, close_backing_in) = unbounded();
        InternalChannel {
            stop_out,
            stop_in,
            #[cfg(feature = "abi-7-40")]
            open_backing_out,
            #[cfg(feature = "abi-7-40")]
            open_backing_in,
            #[cfg(feature = "abi-7-40")]
            close_backing_out,
            #[cfg(feature = "abi-7-40")]
            close_backing_in
        }
    }
}

pub trait SessionQueuer: Send + Sync + Unpin + 'static{
    fn stop(&self);
}

impl SessionQueuer for InternalChannel {
    fn stop(&self) {
        self.stop_out.send(());
    }
}

impl fmt::Debug for Box<dyn SessionQueuer> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Box<SessionQueuer>")
    }
}

#[derive(Debug)]
pub struct SessionHandler {
    queuer: Box<dyn SessionQueuer>,
}

impl SessionHandler {
    pub fn new<Q: SessionQueuer>(queuer: Q) -> Self {
        SessionHandler{queuer: Box::new(queuer)}
    }
    pub fn stop(&self) {
        self.queuer.stop();
    }
}