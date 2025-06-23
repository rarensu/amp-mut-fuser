## Refactoring Plan

This plan outlines the steps to refactor the `fuser` crate, focusing on changing how FUSE operations return their results.

**Phase 1: Core Data Structures and Request Handling**

1.  **Implement `RequestMeta` Struct:** Create the `RequestMeta` struct in `src/request.rs` with the specified fields (`unique`, `uid`, `gid`, `pid`) and derive the `Copy`, `Clone`, and `Debug` traits.
2.  **Update `Request` Struct:** Modify the `Request` struct in `src/request.rs` to include the `meta: RequestMeta` and `replyhandler: ReplyHandler` fields.
3.  **Update `Request::new`:** Update the `Request::new` constructor in `src/request.rs` to initialize the `meta` and `replyhandler` fields. This involves extracting the necessary metadata from the `request` object and creating a new `ReplyHandler` instance.

**Phase 2: Filesystem Trait and `ReplyHandler` Modifications**

4.  **Modify Filesystem Trait Methods (like `lookup`):** Modify the signatures of methods in the `Filesystem` trait (e.g., `lookup` in `src/lib.rs`) to:
    *   Replace `_req: &Request<'_>` or `req: &Request` with `_req: RequestMeta` or `req: RequestMeta`.
    *   Remove `reply: ReplyXXX` parameters.
    *   Add a return type `Result<CorrectEntryType, Errno>`, where `CorrectEntryType` is a new struct (like `Entry` for `lookup`) holding the success data.
5.  **Update `ReplyHandler`:**
    *   Rename `ReplyRaw` to `ReplyHandler` in `src/reply.rs`.
    *   Remove `impl Reply for ReplyHandler`. Make `new` an inherent method.
    *   Add new methods like `reply_entry(self, entry_struct: EntryStruct)` for each `Entry` struct, which will call the appropriate `self.send_ll(&ll::Response::new_xxx(...))` using data from `entry_struct`.

**Phase 3: Operation-Specific Changes and Cleanup**

6.  **Define `Entry` Structs:** For each operation type that returns data (e.g., `lookup`), define a corresponding public struct (e.g., `Entry` for `lookup`) in `src/reply.rs` to hold the data fields previously passed to `reply.xxx_method()`.
7.  **Update `Request::dispatch`:** For each dispatched operation in `src/request.rs`:
    *   Call the modified filesystem method, passing `self.meta`.
    *   Store the `Result`.
    *   Use a `match` on the `Result`:
        *   `Ok(entry_data)` => `self.replyhandler.reply_xxx(entry_data)` (e.g., `reply_entry`).
        *   `Err(error_code)` => `self.replyhandler.error(error_code.into())`.
8.  **Remove Old Code:**
    *   Remove the generic `Reply` trait (if no longer used elsewhere) in `src/reply.rs`.
    *   Remove old `ReplyEntry` (and similar reply-specific structs) in `src/reply.rs`.
    *   Remove `Request::reply()` method in `src/request.rs`.
    *   Remove metadata accessor methods (`unique`, `uid`, `gid`, `pid`) from `Request` in `src/request.rs`.

**Phase 4: Examples and Testing**

9.  **Update Examples:** Modify the examples (e.g., `examples/simple.rs`) to align with the refactored API. This includes updating the `lookup` function signature and implementation to return a `Result` and handle the `RequestMeta` object.
10. **Run Tests:** Execute the existing tests to ensure that the refactoring has not introduced any regressions.
