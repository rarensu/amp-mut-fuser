# Notes on `src/request.rs`

## First Impression

- **Clean and Minimal:** The `RequestMeta` struct is refreshingly simple. It contains only the essential metadata for a request: a unique ID, UID, GID, and PID. This is a very clear and focused design.
- **Vastly Improved API Design:** This file clarifies the design difference between the new and legacy APIs. The old `Request` struct was a complex enum that had to represent the arguments for every possible FUSE operation. The new approach is much better: `RequestMeta` provides the common request context, and operation-specific arguments are passed directly to the `Filesystem` trait methods. This is more explicit, more type-safe, and easier to follow.
- **`Forget` Struct:** The `Forget` struct is also defined here. It's a simple, self-contained data structure for the `forget` operation, which is a clean way to handle it.

## Questions

1.  **Documentation Phrasing:** The file-level documentation states, "A request represents a single FUSE operation." This is true but a bit generic. It could be improved by being more specific, for example: "This module defines `RequestMeta`, which holds the common metadata (like UID, GID) for a FUSE operation. The operation-specific arguments are passed directly to the filesystem trait methods."
2.  **Where is `RequestMeta` created?** How is a `RequestMeta` instance constructed from a raw FUSE request? My assumption is that this happens within the session handling or dispatching logic. This isn't a major source of confusion, but understanding this flow would provide a more complete picture of the crate's architecture.

## Next File to Read

To understand how incoming FUSE requests are processed and mapped to the new synchronous `Filesystem` trait methods, I'll look at **`src/trait_sync/dispatch.rs`**. The name suggests it contains the dispatch logic, which should bridge the gap between the low-level protocol and the high-level trait implementation. I expect to see how `RequestMeta` is created there.
