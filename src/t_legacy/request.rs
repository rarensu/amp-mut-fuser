use crate::RequestMeta;

#[derive(Debug)]
/// Userspace metadata for a given request
pub struct Request<'r> {
    meta: &'r RequestMeta,
}

impl<'r> Request<'r> {
    /// Creates a legacy-compatible Request from the given RequestMeta
    pub fn new(meta: &'r RequestMeta) -> Self {
        Request { meta }
    }
}

// Helper functions for compatibility with LegacyFilesystem
impl Request<'_> {
    /// Returns the unique identifier of this request
    #[inline]
    pub fn unique(&self) -> u64 {
        self.meta.unique
    }

    /// Returns the uid of this request
    #[inline]
    pub fn uid(&self) -> u32 {
        self.meta.uid
    }

    /// Returns the gid of this request
    #[inline]
    pub fn gid(&self) -> u32 {
        self.meta.gid
    }

    /// Returns the pid of this request
    #[inline]
    pub fn pid(&self) -> u32 {
        self.meta.pid
    }
}