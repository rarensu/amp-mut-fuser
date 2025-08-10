# Notes on `src/trait_legacy/dispatch.rs`

## First Impression

- **The "Before" Picture:** This file is a perfect illustration of the legacy, callback-based API. It clearly shows the design pattern that the new `sync` and `async` APIs are meant to replace.
- **Callback-Oriented Design:** The logic for every FUSE operation follows a distinct two-step pattern:
    1.  A `Reply` object is instantiated (e.g., `ReplyEntry::new(...)`, `ReplyAttr::new(...)`). This object serves as a one-time callback, holding the logic to send a reply back to the kernel.
    2.  This `Reply` object is then passed as an argument to the corresponding method on the legacy `Filesystem` trait.
- **Indirect Control Flow:** This pattern is noticeably more complex than the new `Result`-based API. The control flow is less direct, as the dispatch function finishes its work by handing off the `Reply` callback to the filesystem implementation.
- **Clean Compatibility Adapter:** The file defines a local `Request` struct that wraps the new `RequestMeta` struct. This is a well-designed compatibility layer that allows the new unified request-parsing logic to be used with the old `Filesystem` trait without breaking its public API.

## Points of Feedback

1.  **A Powerful Comparison:** This file is an excellent tool for demonstrating the value of the refactoring. Placing the code from `dispatch_legacy` next to the code for `dispatch_sync` would be a very effective way to show, not just tell, why the new APIs are an improvement in terms of code clarity and ergonomics.
2.  **Correctly Located:** This file is in `src/trait_legacy/`, which is exactly where a developer would expect to find it. This is a good example of clear file organization, which contrasts with some of the other confusingly placed files I found during my review.

## Summary

This file is a well-structured implementation of the legacy dispatch logic. It does not create confusion on its own, but rather serves as a clear example of the old API. Its existence helps to contextualize the improvements made in the new `sync` and `async` traits. The real confusion for a new developer arose from finding a similar-looking file in the `trait_sync` directory, which this file helps to identify as a misplaced artifact.
