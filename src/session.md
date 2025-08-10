# Notes on `src/session.rs`

## First Impression

- **Legacy Code:** The `Session` struct and its `run` method are clearly implementations for the **old, callback-based API**. The `run` loop's call to a free-standing `dispatch` function that passes a `Reply` object is the definitive giveaway. The `FS: Filesystem` generic bound on `Session` refers to the legacy `Filesystem` trait.
- **Major Source of User Confusion:** This is a very significant problem for discoverability. Any developer, new or experienced, would naturally assume that `fuser::Session` is the primary entry point for running a filesystem. If they have followed the crate's advice and implemented the new `trait_sync::Filesystem`, they will be unable to use it with this `Session` struct, which would be incredibly confusing.
- **Parallel Implementations:** It's now clear that the codebase contains parallel implementations for the session loop. The legacy logic is in this top-level `session.rs`, while the logic for the new APIs must be located elsewhere (as the `grep` results suggested, in `trait_sync/run.rs`).

## Questions

1.  **How should a user run a `sync::Filesystem`?** Since `fuser::Session` is the wrong tool, there must be a different entry point. The `mount2` function in `lib.rs` is a likely candidate, which probably delegates to the logic in `trait_sync/run.rs` under the hood. This flow is not obvious from the top-level documentation.
2.  **Is this structure intentional?** Is the long-term vision for the crate to maintain separate, parallel session implementations for each API flavor (`legacy`, `sync`, `async`)? Or is there a plan to eventually unify them? This architectural choice is not explained.

## Next File to Read

The `grep` results pointed to `src/trait_sync/run.rs` as the call site for `dispatch_sync`. Having confirmed that `session.rs` is for the legacy API, it is now almost certain that **`src/trait_sync/run.rs`** contains the session loop for the new synchronous API. This is the last major piece of the puzzle. I will read it next.
