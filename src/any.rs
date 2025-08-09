use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

/// This enum holds any of these variants of Filesystem:
/// `::Legacy(fuser::trait_legacy::Filesystem)`,
/// `::Sync(fuser::trait_sync::Filesystem)`, or
/// `::Async(fuser::trait_sync::Filesystem)`.
#[derive(Debug)]
pub enum AnyFS<L, S, A> where 
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS
{
    Legacy(L),
    Sync(S),
    Async(A),
}

// Compiler trickery to enable useful blanket traits
pub(crate) struct _Nl {}
impl LegacyFS for _Nl {}
pub(crate) struct _Ns {}
impl SyncFS for _Ns {}
pub(crate) struct _Na {}
impl AsyncFS for _Na {}

// Useful blanket traits
impl<L> From<L> for AnyFS<L, _Ns, _Na> where
    L: LegacyFS
{
    fn from(value: L) -> Self {
        AnyFS::Legacy(value)
    }
}
impl<S> From<S> for AnyFS<_Nl, S, _Na> where
    S: SyncFS,
{
    fn from(value: S) -> Self {
        AnyFS::Sync(value)
    }
}
impl<A> From<A> for AnyFS<_Nl, _Ns, A> where
    A: AsyncFS 
{
    fn from(value: A) -> Self {
        AnyFS::Async(value)
    }
}



