# Notes on `README.md`

## First Impression

- **Looks Professional:** At first glance, the README is well-structured. It has the expected sections: badges, an "About" section, installation guides for different platforms, and links to documentation. It projects an image of a stable, well-maintained open-source project.
- **Good Selling Point:** The document does a good job of emphasizing that this is a rewrite of the C `libfuse` library, not just simple bindings, which is a strong technical advantage.
- **Completely Silent on the Refactoring:** The most significant finding is that the README makes absolutely no mention of the major refactoring effort. It does not mention the new `sync` and `async` APIs, the fact that the old API is considered legacy, or the new recommended way to build a filesystem.

## Key Discrepancies and Points of Confusion

1.  **Misleading "Usage" Instructions:** The "Usage" section gives the directive: "To create a new filesystem, implement the trait `fuser::Filesystem`." A new user following this instruction will unknowingly adopt the old, callback-based API that is being phased out. This is a critical documentation flaw that actively misguides new users.
2.  **Outdated Examples:** The README points to the `examples` directory. It is almost certain that these examples use the legacy API, further cementing the outdated pattern for anyone trying to learn the crate. This creates a feedback loop where new users learn the old way because the examples and the README only show the old way.
3.  **Incomplete "To Do" Section:** The "To Do" section is very generic. It would be an excellent place to mention the ongoing refactoring to attract contributors who might be interested in helping with the new APIs, migrating examples, or improving documentation.
4.  **macOS Support "untested":** The header for the macOS installation section includes the note "(untested)". This could deter potential users on macOS. If this is no longer true, the note should be removed to reflect the project's actual capabilities.

## Summary

The `README.md` is the front door of the project, and right now it's leading all newcomers to the old, deprecated part of the house. It's arguably the most confusing file I've encountered, because its instructions are clear but they point in the exact opposite direction of the project's stated goals. Updating this file should be the highest priority.
