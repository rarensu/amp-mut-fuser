# FUSE (Filesystem in Userspace) for Rust

![CI](https://github.com/cberner/fuser/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/fuser.svg)](https://crates.io/crates/fuser)
[![Documentation](https://docs.rs/fuser/badge.svg)](https://docs.rs/fuser)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/cberner/fuser/blob/master/LICENSE.md)
[![dependency status](https://deps.rs/repo/github/cberner/fuser/status.svg)](https://deps.rs/repo/github/cberner/fuser)

## About

**FUSE-Rust** is a [Rust] library crate for easy implementation of [FUSE filesystems][FUSE for Linux] in userspace.

FUSE-Rust does not just provide bindings, it is a rewrite of the original FUSE C library to fully take advantage of Rust's architecture.

This library was originally forked from the [`fuse` crate](https://github.com/zargony/fuse-rs) with the intention
of continuing development. In particular adding features from ABIs after 7.19

## Available APIs

This crate offers three different `Filesystem` traits that can be implemented to create a FUSE filesystem. They offer different trade-offs between ease of use, performance, and control.

*   **`trait_sync::Filesystem`**: A synchronous API for single-threaded applications. This is the **recommended trait for most new filesystems**, because it is the easiest to use. The API is currently considered experimental.
*   **`trait_async::Filesystem`**: An `async/await`-based API for integration with asynchronous runtimes like Tokio. It supports single-threaded and multi-threaded applications. The API is currently consdiered experimental.
*   **`trait_legacy::Filesystem`**: The original, callback-based API. It is on track to become deprecated and removed in a future release. It should not be used for new projects.

## Documentation

[FUSE-Rust reference][Documentation]

## Details

A working FUSE filesystem consists of three parts:

1. The **kernel driver** that registers as a filesystem and forwards operations into a communication channel to a userspace process that handles them.
1. The **userspace library** (libfuse) that helps the userspace process to establish and run communication with the kernel driver.
1. The **userspace implementation** that actually processes the filesystem operations.

The kernel driver is provided by the FUSE project, the userspace implementation needs to be provided by the developer. FUSE-Rust provides a replacement for the libfuse userspace library between these two. This way, a developer can fully take advantage of the Rust type interface and runtime features when building a FUSE filesystem in Rust.

Except for a single setup (mount) function call and a final teardown (umount) function call to libfuse, everything runs in Rust, and on Linux these calls to libfuse are optional. They can be removed by building without the "libfuse" feature flag.

## Dependencies

FUSE must be installed to build or run programs that use FUSE-Rust (i.e. kernel driver and libraries. Some platforms may also require userland utils like `fusermount`). A default installation of FUSE is usually sufficient.

To build FUSE-Rust or any program that depends on it, `pkg-config` needs to be installed as well.

### Linux

[FUSE for Linux] is available in most Linux distributions and usually called `fuse` or `fuse3` (this crate is compatible with both). To install on a Debian based system:

```sh
sudo apt-get install fuse3 libfuse3-dev
```

Install on CentOS:

```sh
sudo yum install fuse
```

To build, FUSE libraries and headers are required. The package is usually called `libfuse-dev` or `fuse-devel`. Also `pkg-config` is required for locating libraries and headers.

```sh
sudo apt-get install libfuse-dev pkg-config
```

```sh
sudo yum install fuse-devel pkgconfig
```

### macOS

Install [FUSE for macOS], which can be obtained from their website or installed using the Homebrew or Nix package managers. macOS version 10.9 or later is required. If you are using a Mac with Apple Silicon, you must also [enable support for third party kernel extensions][enable kext].

Note: Testing on macOS is only done infrequently. If you experience difficulties, please create an issue. 

#### To install using Homebrew

```sh
brew install macfuse pkgconf
```

#### To install using Nix

``` sh
nix-env -iA nixos.macfuse-stubs nixos.pkg-config
```

When using `nix` it is required that you specify `PKG_CONFIG_PATH` environment variable to point at where `macfuse` is installed:

``` sh
export PKG_CONFIG_PATH=${HOME}/.nix-profile/lib/pkgconfig
```

### FreeBSD

Install packages `fusefs-libs` and `pkgconf`.

```sh
pkg install fusefs-libs pkgconf
```

## Usage

```sh
cargo add fuser
```

or put this in your `Cargo.toml`:

```toml
[dependencies]
fuser = "0.15"
```

To create a new filesystem, we recommend implementing the `fuser::trait_sync::Filesystem` trait. See the [documentation] for details. The `examples` directory contains several basic filesystems, including at least one example of each trait type.

A minimal `sync` filesystem looks like this:

```rust
use fuser::trait_sync::Filesystem;

struct MyFS;

impl Filesystem for MyFS {
    // implement methods here
}

fn main() {
    let fs = MyFS;
    let dir = "/tmp/mnt";
    let opts = &[];
    // The `from` trait automatically converts `MyFS` in a matching `AnyFS` enum value.
    fuser::mount2(fs.into(), dir, opts).unwrap();
}
```

### Feature Gates

The crate uses feature gates to manage optional functionality and dependencies. Some key features include:
*   **`abi-7-x`**: A set of features to select the FUSE protocol version. Recommended to select the highest version.
*   **`threaded`**: Enable support for multithreaded applications and threaded i/o (default).
*   **`tokio`**: Enable support for tokio asynchronous runtime. Integrates with fuser's internal threaded i/o.
*   **`no-rc`**: Disable support for non-thread-safe structs `Rc`/`RefCell` (default).
*   **`locking`**: Enable support for locking structs `Mutex`/`RwLock`/`RefCell`.
*   **`libfuse`**: Use libfuse bindings for some very low-level operations. An older alternative to the newer Rust-native implementations.
*   **`serializable`**: Enable conversion between `fuser` data structures and raw bytes, for saving to disk or transmission over a network.

## To Do

Most features of libfuse up to 3.10.3 are implemented. Feel free to contribute. See the [list of issues][issues] on GitHub and search the source files for comments containing "`TODO`" or "`FIXME`" to see what's still missing.

## Compatibility

Developed and automatically tested on Linux. Tested under [Linux][FUSE for Linux] and [FreeBSD][FUSE for FreeBSD] using stable [Rust] (see CI for details). Infrequently, manually tested for MacFUSE on MacOS.

## License

Licensed under [MIT License](LICENSE.md), except for those files in `examples/` that explicitly contain a different license.

## Contributing

Fork, hack, submit pull request. Make sure to make it useful for the target audience, keep the project's philosophy and Rust coding standards in mind. For larger or essential changes, you may want to open an issue for discussion first. Also remember to update the [Changelog] if your changes are relevant to the users.

### Concepts

A brief overview of Fuser concepts for new contributors.

* **`Session`**: The core struct which saves configuration options. Its provides methods to start and end event handling loops.
* **`RequestHandler`** and **`ReplyHandler`**: These structures represents one FUSE operation initiated by the Kernel. The RequestHandler methods handle unpacks this message, and directs it to the filesystem. The `ReplyHandler` passes the response back to the kernel.
* **`Notification`**: This struct represents one FUSE operation initiated by the User application (i.e., not in response to a `Request`).
* **`Filesystem`**: User application code, which implements one specific Filesystem trait.
* **`AnyFS`**: A simple `enum` which enables the `Session` struct to natively accept any of the three `Filesystem` trait variants, always using the matching `dispatch` method at run-time.

### Subdirectories

A bried overview of repository organization for new contributors. 

*   **`src/trait_*/`**: These three modules (`trait_legacy`, `trait_sync`, `trait_async`) contain the definitions for the three `Filesystem` traits, trait-specific `Session` methods, and the `dispatch()` method that directs a `Request` to the appropriate `Filesystem` method.
*   **`src/container/`**: Custom data structures that are generic over their storage, including borrowed (`'static`), owned, and shared types, (similar to the `bytes::Bytes` struct) to encourage no-copy solutions. It is currently used for the `DirentList` and `DirentPlusList` types which return `readdir()` and `readdirplus()`.
*   **`src/mnt/`**: Code for establishing communication with the fuse device, which is called mounting.
*   **`src/ll/`**: The low-level FUSE message interface. This module contains the raw FUSE ABI definitions and is responsible for the translating between Rust-based data structures and byte-based fuse kernel messages. It is not recommended for applications to use this code directly.

[Rust]: https://rust-lang.org
[Homebrew]: https://brew.sh
[Changelog]: https://keepachangelog.com/en/1.0.0/

[FUSE-Rust]: https://github.com/cberner/fuser
[issues]: https://github.com/cberner/fuser/issues
[Documentation]: https://docs.rs/fuser

[FUSE for Linux]: https://github.com/libfuse/libfuse/
[FUSE for macOS]: https://macfuse.github.io
[enable kext]: https://github.com/macfuse/macfuse/wiki/Getting-Started#enabling-support-for-third-party-kernel-extensions-apple-silicon-macs
[FUSE for FreeBSD]: https://wiki.freebsd.org/FUSEFS