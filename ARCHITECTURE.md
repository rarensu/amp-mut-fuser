# fuser Architecture

This document provides a high-level overview of the `fuser` crate's architecture. It is intended for new contributors who want to understand how the different components of the library work together.

## The Three Filesystem Traits

The `fuser` crate provides three different `Filesystem` traits that can be implemented to create a FUSE filesystem:

*   **`trait_legacy::Filesystem`**: The original, callback-based API from the `fuse` crate. It is on track to become deprecated and should not be used for new projects.
*   **`trait_sync::Filesystem`**: A synchronous API for single-threaded applications. This is the recommended trait for most new filesystems because it is the easiest to use.
*   **`trait_async::Filesystem`**: An `async/await`-based API for integration with asynchronous runtimes like Tokio. It supports single-threaded and multi-threaded applications.

## The Dispatch Mechanism

A key feature of the `fuser` crate is its ability to handle any of the three filesystem traits with the same `mount2` function. This is achieved through a combination of the `AnyFS` enum and dispatch logic within the `Session` object.

### `AnyFS`: The Filesystem Wrapper

The `AnyFS` enum is a simple wrapper that can hold an instance of any of the three filesystem traits.

```rust
pub enum AnyFS<L, S, A> {
    Legacy(L),
    Sync(S),
    Async(A),
}
```

When you call `fuser::mount2`, you pass your filesystem struct. The `From` trait is implemented for each of the filesystem traits, so you can convert your filesystem struct into an `AnyFS` by calling `.into()`. This allows `mount2` to accept any of the three kinds of filesystems in a type-safe way.

### `Session`: Native Dispatch

The `Session` object is the core of the FUSE event loop. It is responsible for reading requests from the kernel and dispatching them to the appropriate filesystem implementation.

Crucially, the `Session` handles each `AnyFS` variant **natively**. It does *not* convert between them. Inside the `Session`'s main loop, there is logic that matches on the `AnyFS` enum:

*   If the filesystem is `AnyFS::Legacy`, the `Session` calls the methods of the `trait_legacy::Filesystem` trait directly.
*   If the filesystem is `AnyFS::Sync`, the `Session` uses the dispatch logic in `src/trait_sync/dispatch.rs` to call the methods of the `trait_sync::Filesystem` trait.
*   If the filesystem is `AnyFS::Async`, the `Session` uses the dispatch logic in `src/trait_async/dispatch.rs` to call the methods of the `trait_async::Filesystem` trait.

This design allows the crate to support three distinct filesystem APIs without needing a complex internal adaptation layer. Each API has its own optimized dispatch path within the `Session`.

### Optional External Adapters

While `fuser` itself does not provide adapters to convert between the different filesystem traits, the architecture makes it possible to write such adapters externally. An adapter is a struct that wraps a filesystem of one type and implements the `Filesystem` trait of another type. This can be useful for developers who want to mix and match filesystem implementations. For example, one could write an adapter that takes a `trait_sync::Filesystem` and exposes it as a `trait_async::Filesystem`. These adapters, however, are not part of the `fuser` crate itself.
