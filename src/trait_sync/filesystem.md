# Notes on `src/trait_sync/filesystem.rs`

## First Impression

- **Huge Improvement:** The API design, which returns a `Result<T, Errno>`, is a significant improvement over the legacy callback-based approach. It feels much more idiomatic to Rust and is easier to reason about.
- **Thread-Safety by Default:** The use of `&self` for all trait methods is a strong and clear design choice. The documentation rightly points out that this encourages using interior mutability (like `Mutex` or `RwLock`) for state changes, making it well-suited for multi-threaded filesystems from the get-go.
- **Sensible Defaults:** Providing default implementations that return `ENOSYS` (operation not supported) is excellent. This significantly lowers the barrier to entry, as a developer only needs to implement the FUSE operations they care about.
- **Good Documentation:** The documentation for the trait and the module is generally helpful, explaining the design philosophy behind `&self` and the default method implementations.

## Questions

1.  **`getattr` Return Value:** The `getattr` method returns `Result<(FileAttr, Option<Duration>), Errno>`. I assume the `Option<Duration>` is for the attribute timeout (TTL), but this is just a guess as the method's doc comment is missing. This should be explicitly documented.
2.  **`forget` Method:** The `forget` method has no return value. This makes sense for a "best-effort" notification from the kernel, but it stands in contrast to the other methods that return a `Result`. It's a minor point but worth noting.
3.  **Typo in Docs:** The module-level documentation starts with `//! EWOULDBLOCK,`. This appears to be a copy-paste error or some leftover text and should be removed.
4.  **`RequestMeta` vs `Request`:** The method signatures use a `RequestMeta` type. The legacy example used a `&Request` type. What is the difference between them? `RequestMeta` sounds like it might contain less information. Is the full request data still accessible somehow, or does this new API intentionally abstract some of it away?

## Next File to Read

To answer my question about `RequestMeta`, I need to see its definition. The `use` statements at the top of the file show that it comes from `crate::request`. Therefore, the next file I will read is **`src/request.rs`**. I hope this will clarify what request information is provided to the filesystem implementation in the new synchronous API.
