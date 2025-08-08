use crate::session::FilesystemExt;
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

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

pub(crate) struct _Nl {}
impl FilesystemExt for _Nl {}
impl LegacyFS for _Nl {}
pub(crate) struct _Ns {}
impl FilesystemExt for _Ns {}
impl SyncFS for _Ns {}
pub(crate) struct _Na {}
impl FilesystemExt for _Na {}
impl AsyncFS for _Na {}

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