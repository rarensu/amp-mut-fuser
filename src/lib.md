# Notes on `src/lib.rs`

## First Impression

- The crate-level documentation is great. It clearly explains the purpose of the crate, its transitional state from a legacy callback-based API to new sync and async APIs, and provides a getting-started guide.
- The example code uses the legacy API (`Filesystem` trait). This is a bit confusing given the stated goal of moving to the new APIs. It might be better to lead with a `sync` or `async` example and move the legacy one to a separate section.
- The module structure seems logical. The separation of `trait_legacy`, `trait_sync`, and `trait_async` is very clear. The `ll` (low-level) and `mnt` (mount) modules also make sense.
- There are a lot of `pub use` statements at the top level. This is helpful for making the most common types easily accessible.

## Questions

1.  The documentation describes `fuser` as a "low-level" library and recommends `fuse-mt` for a higher-level experience. What specifically makes `fuser` low-level? The example seems fairly straightforward, but I suspect the complexity is hidden in state management and handling more complex operations.
2.  The `main` function in the example uses `fuser::mount2`. The `pub use` statements show both `mount2` and `mount3`. What is the difference between them? Is it related to the FUSE protocol version? The docs don't seem to mention `mount3`.
3.  The crate documentation mentions `sync` and `async` modules, but the actual module structure in the code is `trait_sync` and `trait_async`. The docs seem to be slightly out of sync with the implementation. For example, the doc links point to `sync/index.html` which will likely not work.
4.  The example uses the legacy `Filesystem` trait. Where are the definitions for the new `sync` and `async` Filesystem traits? I assume they are in `src/trait_sync/filesystem.rs` and `src/trait_async/filesystem.rs` respectively.
5.  What are the `any.rs` and `container` modules for? They are not mentioned in the main documentation, so their purpose isn't immediately obvious.

## Next File to Read

To understand the new synchronous API, which seems like the natural next step after seeing the legacy one, I'll read **`src/trait_sync/filesystem.rs`**. I expect to find the definition of the new synchronous `Filesystem` trait there, which should answer some of my questions about how the new API improves on the old one.
