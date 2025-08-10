use crate::trait_legacy::Filesystem as LegacyFS;
use crate::trait_sync::Filesystem as SyncFS;
use crate::trait_async::Filesystem as AsyncFS;

/// A wrapper for any of the three `Filesystem` traits.
///
/// `AnyFS` is an enum that can hold a `trait_legacy::Filesystem`, a `trait_sync::Filesystem`,
/// or a `trait_async::Filesystem`. It is used by the `mount2` function to accept any of these
/// three traits.
///
/// The `From` trait is implemented for each of the filesystem traits, so you can convert
/// your filesystem struct into an `AnyFS` by calling `.into()`.
///
/// For example:
/// ```rust,ignore
/// let fs = MySyncFilesystem;
/// fuser::mount2(fs.into(), "/mnt/fuse", &[]).unwrap();
/// ```
#[derive(Debug)]
pub enum AnyFS<L, S, A> where
    L: LegacyFS,
    S: SyncFS,
    A: AsyncFS
{
    /// Holds a variant of Filesystem that uses the Legacy API
    Legacy(L),
    /// Holds a variant of Filesystem that uses the Synchronous API
    Sync(S),
    /// Holds a variant of Filesystem that uses the Asynchronous API
    Async(A),
}

// Compiler trickery to enable useful blanket traits
#[derive(Debug)]
pub struct _Nl {}
impl LegacyFS for _Nl {}
#[derive(Debug)]
pub struct _Ns {}
impl SyncFS for _Ns {}
#[derive(Debug)]
pub struct _Na {}
impl AsyncFS for _Na {}

// The following `From` implementations wrap a filesystem struct in the `AnyFS` enum.
// This allows the `mount2` function to be generic over the three filesystem traits.
// The `_Nl`, `_Ns`, and `_Na` structs are empty placeholder types that implement the
// respective `Filesystem` traits, so that we can have a concrete type for the `AnyFS`
// enum.

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



