use crate::trait_async::Filesystem as AsyncFS;
use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;

/// This enum is a wrapper that holds any trait variant of Filesystem.
#[derive(Debug)]
pub enum AnyFS<L, S, A>
where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS,
{
    /// Holds a variant of Filesystem that uses the Legacy API
    Legacy(L),
    /// Holds a variant of Filesystem that uses the Synchronous API
    Sync(S),
    /// Holds a variant of Filesystem that uses the Asynchronous API
    Async(A),
}

/* ------ Compiler trickery to enable useful blanket traits ------ */

// The `_Nl`, `_Ns`, and `_Na` structs are empty placeholder types that implement the
// respective `Filesystem` traits, which provides a fallback, concrete type for `AnyFS`'s
// type parameters.
#[derive(Debug)]
pub struct _Nl {}
impl LegacyFS for _Nl {}
#[derive(Debug)]
pub struct _Ns {}
impl SyncFS for _Ns {}
#[derive(Debug)]
pub struct _Na {}
impl AsyncFS for _Na {}

/* ------ Useful blanket traits ------ */

// The following `From` implementations wrap a filesystem struct in the `AnyFS` enum.
// This allows the `mount` and `Session` functions to directly accept Filesystems of any variant.
// For example, `MyLegacyFS.into()`` yields `AnyFS::Legacy(MyLegacyFS)`.
impl<L> From<L> for AnyFS<L, _Ns, _Na>
where
    L: LegacyFS,
{
    fn from(value: L) -> Self {
        AnyFS::Legacy(value)
    }
}
impl<S> From<S> for AnyFS<_Nl, S, _Na>
where
    S: SyncFS,
{
    fn from(value: S) -> Self {
        AnyFS::Sync(value)
    }
}
impl<A> From<A> for AnyFS<_Nl, _Ns, A>
where
    A: AsyncFS,
{
    fn from(value: A) -> Self {
        AnyFS::Async(value)
    }
}
