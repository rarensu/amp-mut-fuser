# Why `FileSystem` Needs to be `Send`

Your hypothesis that the `FileSystem` trait needs to be `Send` because of background thread usage is correct. This report details the code path and reasoning behind this requirement.

## The `Send` Requirement and its Origin

The requirement for `FileSystem` to be `Send` is not universal; it's conditional. It only applies when you want to run the FUSE session in a background thread, which is a common pattern to avoid blocking your application's main thread.

The requirement is introduced at the public API level, specifically in functions designed to spawn a background session.

### 1. `fuser::spawn_mount2`

The trace begins in `src/lib.rs` with the `spawn_mount2` function. Its signature clearly introduces the `Send` trait bound on your `FileSystem` implementation (`FS`).

```rust
// In src/lib.rs
pub fn spawn_mount2<'a, FS: Filesystem + Send + 'static + 'a, P: AsRef<Path>>(
    filesystem: FS,
    mountpoint: P,
    options: &[MountOption],
) -> io::Result<BackgroundSession> {
    check_option_conflicts(options)?;
    Session::new(filesystem, mountpoint.as_ref(), options).and_then(|se| se.spawn())
}
```

### 2. `Session::spawn`

This function then calls `Session::spawn()`, which is defined in `src/session.rs`. This method continues to propagate the `Send` requirement.

```rust
// In src/session.rs
impl<FS: 'static + Filesystem + Send> Session<FS> {
    /// Run the session loop in a background thread
    pub fn spawn(self) -> io::Result<BackgroundSession> {
        BackgroundSession::new(self)
    }
}
```

## The Core Reason: `std::thread::spawn`

The `Session::spawn` method calls `BackgroundSession::new`, which is where the new thread is created. This is the technical reason for the `Send` requirement.

```rust
// In src/session.rs
impl BackgroundSession {
    pub fn new<FS: Filesystem + Send + 'static>(se: Session<FS>) -> io::Result<BackgroundSession> {
        // ...
        let guard = thread::spawn(move || {
            let mut se = se;
            se.run()
        });
        Ok(BackgroundSession {
            guard,
            // ...
        })
    }
}
```

1.  **`thread::spawn`**: This function from the Rust standard library creates a new OS thread.
2.  **`move` closure**: The `move` keyword gives the new thread ownership of the variables it uses, in this case, the `Session` instance `se`.
3.  **The `Send` Trait**: A fundamental rule in Rust's concurrency model is that any data transferred across a thread boundary must implement the `Send` trait. This is a compile-time guarantee that the type is safe to transfer.
4.  **`Session` struct**: The `Session` struct holds your `FileSystem` implementation. For `Session` to be `Send`, all of its fields must also be `Send`.

```rust
// In src/session.rs
pub struct Session<FS: Filesystem> {
    pub(crate) filesystem: FS,
    // ... other fields that are also Send
}
```

Therefore, `filesystem: FS` must be `Send`, which means your `FileSystem` implementation must be `Send`.

## Summary

*   **The `FileSystem` trait itself is not `Send`**.
*   The **`Send` requirement is conditional** on using background thread functionality (`spawn_mount2`, `Session::spawn`). Running the filesystem in the foreground does not require `Send`.
*   The requirement is enforced by the Rust compiler because of the call to **`std::thread::spawn`**, which moves the `Session` (containing the `FileSystem`) to a new thread.
*   This is a core feature of Rust's **thread safety guarantees**.
