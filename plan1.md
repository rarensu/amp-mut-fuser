# Fuser Crate Experimental Modification Plan

## 1. Project Context (From Fuse Project Memory Bank)
The Fuse Project aims to create a FUSE filesystem that utilizes containerization and dynamic mounting to provide isolated and configurable workspace environments. Key goals include intercepting filesystem calls, dynamically mounting filesystems based on TOML configuration, and ensuring no elevated privileges (no sudo) are required. The project follows a component-based architecture with clear separation of concerns, migrating from script-based components to Rust for improved performance and maintainability.

Relevant components include:
- **fuse_interceptor**: Intercepts filesystem calls and provides passthrough functionality.
- Other components like `config_scanner`, `mount_manager`, `container_component`, and `workspace_manager` support the broader system.

### 1.1 Mandatory System Patterns
The Fuse Project adheres to several mandatory system patterns to ensure consistency and reliability across its components, which should guide the modifications to the fuser crate:
- **Component-Based Architecture**: The system is divided into distinct components, each responsible for a specific functionality, promoting modularity and ease of testing.
- **Filesystem in Userspace (FUSE)**: Utilizes FUSE for intercepting filesystem operations, which is central to the fuser crate's role in the project.
- **Configuration-Driven Behavior**: System behavior, including filesystem mounting, is driven by TOML configurations, ensuring flexibility and user control.
- **Dependency Injection**: Components are designed to receive dependencies rather than hardcoding them, facilitating testing and flexibility.
- **Test File Naming Convention**: All files containing tests must include the word "test" in the filename, and files with "test" in the name are expected to contain tests.
- **Test File Structure in Rust Source Directories**: Test files may follow the pattern `src/**/{mod.rs,tests_good.rs,tests_bad.rs}`, where `mod.rs` declares submodules for passing (`tests_good.rs`) and error scenarios (`tests_bad.rs`).
- **Development Practices**: Testing one functionality at a time to isolate issues, using `RUST_LOG=debug` for detailed logging during tests, and following consistent patterns for path-based methods to check for branch points before forwarding operations.

These patterns ensure that modifications to the fuser crate align with the broader architectural principles of the Fuse Project, maintaining consistency in design and implementation.

The fuser crate, used within this project, is a Rust library for FUSE filesystem implementation, providing high-level abstractions for filesystem operations. This experimental variant at `experiment/fuser` focuses on enhancing the `Filesystem` trait to address user feedback on performance issues with current return types.

## 2. Motivation for Modification
Feedback from users of the fuser crate indicates that the current return types, particularly `Result<Vec<u8>, Errno>` for methods like `readlink` and `read`, and `Result<Vec<DirEntry>, Errno>` for `readdir`, are suboptimal due to the potential for large vectors impacting performance. The goal is to modify the `Filesystem` trait method signatures to return more flexible types that support zero-copy operations and efficient data handling, allowing users to choose the most appropriate data representation for their use case.

## 3. Research and Design Concept (From 'research/rust return enum.md')
The proposed solution is based on a custom enum named `ByteBox` to handle different ownership models for byte data, ensuring flexibility and performance. The design includes:

- **ByteBox Enum Definition**:
  ```rust
  pub enum ByteBox<'a> {
      Borrowed(&'a [u8]),        // Zero-copy reference to existing data
      Owned(Box<[u8]>),     // Uniquely owned heap-allocated data
      Shared(Arc<[u8]>),     // Shared ownership with atomic reference counting
  }
  ```
  This enum allows users to provide data optimally based on its source and ownership, supporting zero-copy for static/cached data, efficient moves for new allocations, and shared ownership for cached or multi-threaded scenarios.

- **From Trait Implementations**:
  Implementations of `From` for common types (`&[u8]`, `Box<[u8]>`, `Arc<[u8]>`, `Vec<u8>`) to convert into `ByteBox`, making the API ergonomic for users with `.into()` calls.

- **Benefits**:
  - Optimal performance with zero-copy options.
  - Maximum flexibility for implementers to choose data representations.
  - Clear ownership semantics for library and user understanding.
  - Future extensibility for additional variants like streaming data.

- **Example Implementations and Usage**:
  Below are examples adapted from the research to illustrate how users can implement the data provisioning and how the library can consume the provided data using the `ByteBox` enum.

  **User's Implementation Example**:
  This example shows how a user might implement the `Filesystem` trait to provide file content using different `ByteBox` variants.
  ```rust
  use your_crate::{Filesystem, ByteBox};
  use std::sync::Arc;

  struct MyUserFilesystem;

  impl Filesystem for MyUserFilesystem {
      fn read(&self, ino: u64, /* other parameters */) -> Result<ByteBox, Errno> {
          match backend_provider(ino) {
              // Case 1: Static/Embedded Data (Zero-Copy Borrow)
              "/readme.txt" => {
                  // Return a reference to static content.
                  // Uses `impl From<&'a [u8]> for ByteBox`.
                  Ok(b"Hello, this is a static readme file!\n".into())
              },
              // Case 2: Dynamically Loaded, Uniquely Owned Data (Efficient Move)
              "/config.json" => {
                  // Simulate loading data into a Box<[u8]> or converting from Vec<u8>.
                  // Uses `impl From<Box<[u8]>> for ByteBox`.
                  Ok(Box::new(*b"{ \"key\": \"value\" }").into())
              },
              // Case 3: Data from a Shared Cache (Efficient Move of Arc)
              "/log/latest.log" => {
                  // In a real app, this would come from a HashMap<_, Arc<[u8]>> cache.
                  // Uses `impl From<Arc<[u8]>> for ByteBox`.
                  Ok(Arc::new(*b"Important log entry: User logged in at 2025-07-02.\n").into())
              },
              // Case 4: Data from a Vec<u8> (Very Common)
              "/user/profile.bin" => {
                  // Users often work with Vec<u8>. It automatically converts to Box<[u8]>,
                  // then to ByteBox.
                  let user_data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
                  Ok(user_data.into()) // Chains Vec<u8> -> Box<[u8]> -> ByteBox
              },
              // Case 5: File Not Found / Default Content
              _ => {
                  // Fallback to a simple borrowed slice if the file is not explicitly handled.
                  // Uses `impl From<&'a [u8]> for ByteBox`.
                  Ok(b"File not found or not supported by this filesystem.".into())
              }
          }
      }
  }
  ```

  **Library Consumption Example**:
  This example demonstrates how the fuser library might internally process the `ByteBox` data provided by the user.
  ```rust
  fn simulate_fuse_read(fs_source: &impl Filesystem, ino: u64, path: &str) -> Result<(), Errno> {
      let data_provider = fs_source.read(ino, /* other parameters */)?;

      match data_provider {
          ByteBox::Borrowed(slice) => {
              println!("\nLibrary received BORROWED data for '{}': {}", path, String::from_utf8_lossy(slice));
          },
          ByteBox::Owned(b) => {
              println!("\nLibrary received UNIQUELY OWNED data for '{}': {}", path, String::from_utf8_lossy(&b));
              // Library can now use 'b' and it will be dropped when 'b' goes out of scope.
          },
          ByteBox::Shared(a) => {
              println!("\nLibrary received SHARED data for '{}': {}", path, String::from_utf8_lossy(&a));
              // Library can clone 'a' if it wants to keep a copy in its own cache.
              let _library_cache_copy = a.clone(); // O(1) operation
          },
      }
      Ok(())
  }
  ```

A similar approach is proposed for directory entry returns to address performance concerns with large vectors in `readdir` and `readdirplus`, where analogous enums like `DirEntryBox` and `DirEntryPlusBox` can be implemented with similar ownership models and usage patterns.

## 4. Target Methods for Modification in `Filesystem` Trait
The following methods in the `Filesystem` trait (defined in `src/lib.rs`) are targeted for modification to use flexible return types:

- **Byte Data Returns**:
  - `readlink`: Currently `Result<Vec<u8>, Errno>` - Change to `Result<ByteBox<'a>, Errno>`.
  - `read`: Currently `Result<Vec<u8>, Errno>` - Change to `Result<ByteBox<'a>, Errno>`.
  - `getxattr` and `listxattr`: Currently `Result<Xattr, Errno>` where `Xattr` includes `Vec<u8>` - Update `Xattr` to use `ByteBox<'a>` for data variant (e.g., `Xattr::Data(ByteBox<'a>)`).

- **Directory Entry Returns**:
  - `readdir`: Currently `Result<Vec<DirEntry>, Errno>` - Change to `Result<DirEntryBox<'a>, Errno>`.
  - `readdirplus`: Currently `Result<Vec<(DirEntry, Entry)>, Errno>` - Change to `Result<DirEntryPlusBox<'a>, Errno>`.

## 5. Implementation Strategy
1. **Define `ByteBox` Enum for Byte Data**:
   - Implement the `ByteBox` enum with variants `Borrowed`, `Owned`, and `Shared` as described.
   - Add `From` implementations for common byte data types to allow easy conversion to `ByteBox`.
   - Update `readlink`, `read`, `getxattr`, and `listxattr` to use `ByteBox<'a>` in their return types.

2. **Define Enums for Directory Entries**:
   - Create `DirEntryBox` enum with variants `Borrowed(&'a [DirEntry])`, `Owned(Box<[DirEntry]>)`, and `Shared(Arc<[DirEntry]>)` for `readdir`.
   - Create `DirEntryPlusBox` enum with variants `Borrowed(&'a [(DirEntry, Entry)])`, `Owned(Box<[(DirEntry, Entry)]>)`, and `Shared(Arc<[(DirEntry, Entry)]>)` for `readdirplus`.
   - Add `From` implementations for common collection types to convert to these enums.
   - Update `readdir` and `readdirplus` to use these new return types.

3. **Adjust Method Signatures**:
   - Modify the method signatures in the `Filesystem` trait to reflect the new return types.
   - Ensure that lifetime parameters are correctly managed in the trait definition to support borrowed data.

4. **Update Internal Handling**:
   - Adjust the internal logic of the fuser crate to handle the new enum types, matching on variants to process data appropriately (e.g., direct use of borrowed data, cloning Arc for caching).
   - Ensure that the library can convert or access the data as needed for FUSE kernel communication.

5. **Documentation and Testing**:
   - Update documentation for the modified methods to explain the new return types, ownership models, and performance benefits.
   - Revise existing tests to accommodate the new enum return types and add test cases for different data representations (borrowed, uniquely owned, shared) to verify flexibility and correctness.

## 6. Notes and Considerations
- **Backward Compatibility**: Not required as users have not yet adopted the current `Vec<>`-based variant, so these changes will not break existing implementations.
- **Input Data Flexibility**: Not addressed in this plan as users prefer simple input types like `&[u8]`, which can be revisited later if needed.
- **Future Extensibility**: The enum design allows for non-breaking additions of new variants (e.g., streaming data support) in future updates.

This plan provides a comprehensive guide for modifying the fuser crate to support flexible data return types, addressing user feedback on performance while maintaining clarity and extensibility in the API design.
