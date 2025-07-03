# Plan for Migrating Poll Feature in Experimental Fuser Crate Variant

## Introduction
This document outlines a detailed plan for migrating the Poll feature in the experimental variant of the fuser crate located at `experiment/fuser`. The goal is to transition from non-idiomatic callback-based methods to a more idiomatic approach that supports back-and-forth communication between Filesystem and Session objects. This plan is designed to be self-contained, providing all necessary context for an agent to implement the migration without access to external memory banks, research files, or prior conversations. The focus is initially on Poll as a general case, with conceptual applications to other features like Notify and Passthrough.

## Objective
The primary objective is to replace the existing callback mechanism for Poll with a decoupled, asynchronous message-passing system using Multi-Producer Multi-Consumer (MPMC) channels. This approach addresses the limitation of single-use Result<> returns by enabling continuous communication, ensuring high concurrency, and maintaining resilience through dynamic channel re-establishment.

## Background Context
The fuser crate variant under `experiment/fuser` is part of a broader Fuse Project aimed at creating a FUSE filesystem that utilizes containerization and dynamic mounting to provide an isolated and configurable workspace environment. The project's goal is to intercept filesystem calls, dynamically mount filesystems based on TOML configuration, create user namespaces and pivot root for containerization, and set up shared memory areas to launch processes within the container.

### Project Overview (Summarized)
The Fuse Project aims to create a FUSE filesystem for isolated, configurable workspace environments using containerization and dynamic mounting. Non-Rust components and implementation details outside of the fuser crate are summarized to focus on the core migration task. The project emphasizes no elevated privileges (no sudo), component-based architecture, and seamless integration with development workflows. Key areas like containerization and configuration management exist but are not detailed here as they are not directly relevant to the Rust-based fuser migration.

### Rust Migration Strategy
The Fuse Project is migrating from script-based components to Rust implementations to improve performance, type safety, and maintainability while preserving the existing component architecture. The strategy includes:
1. Creating Rust equivalents of script-based components.
2. Implementing comprehensive tests for both implementations.
3. Gradually replacing script components with Rust implementations.
4. Ensuring backward compatibility during the transition.
5. Completing migration with full test coverage.
The experimental fuser crate variant under `experiment/fuser` is a key part of this migration, focusing on idiomatic Rust patterns for filesystem operations.

### Technologies and Development Setup (Rust Focus)
- **Rust**: Primary language for the fuser crate variant and new components, focusing on performance and type safety.
- **FUSE**: Filesystem in Userspace, central to filesystem interception in Rust implementations.
- **Cargo**: Used for building and testing Rust components like the fuser crate.
Other technologies, dependencies, and tools (e.g., TOML, C, Bash/Python, various build dependencies) exist but are not detailed here to maintain focus on Rust-specific migration aspects. Development follows Rust best practices with Cargo for dependency management and testing.

### Development Insights
- Comprehensive test suites (`tests_good.rs` and `tests_bad.rs`) are critical for validating both successful operations and error handling in filesystem implementations like `BranchFS`.
- Integration tests with layered filesystems (kernel -> BranchFS -> PassThroughFS) have proven successful, confirming the viability of the current implementation for file operations.
- Compilation issues can sometimes be resolved by cleaning the build cache, indicating potential build artifact inconsistencies.
- The `fuser` crate's API and feature flags (e.g., `abi-7-15`) can affect struct availability, requiring careful import management during development.

### Project Progress
#### What Works
- The `BranchFS` struct unifies multiple inner filesystems (branches) under a single namespace, with inode and path mapping between outer and inner views.
- A minimal `EmptyFS` implementation handles the root directory (inode 1), returning static directory attributes for stability and kernel compatibility.
- `BranchFS` initialization includes a default branch (ID 0) for the root directory using `EmptyFS`.
- The `find_child_branches` method identifies top-level branches for the root directory and child branches for other directories based on path hierarchy.
- The `readdir` method in `BranchFS` forwards directory listing requests to the appropriate branch and appends synthetic entries for child branches, ensuring visibility of mounted branches in directory listings.
- Implemented `create`, `unlink`, `mkdir`, and `rmdir` methods in `BranchFS` for file and directory operations, returning `EPERM` for branch points with a TODO for future improvements, and forwarding to the branch's filesystem otherwise.
- Comprehensive test suites are in place:
  - `tests_good.rs` covers passing scenarios for initialization, branch addition, lookup, readdir (root and branch), getattr, setattr, open/release, read, write, mkdir, and rmdir operations.
  - `tests_bad.rs` covers error handling for non-consecutive branch IDs, non-existent file/branch lookups, and invalid inode operations.
- Integration tests in `tests/fuse_mounted_tests.rs` confirm that file creation, writing, and deletion work correctly in a layered filesystem setup (kernel -> BranchFS -> PassThroughFS).
- Compilation issues with `FileAttr` were resolved by cleaning the build cache and rebuilding with `cargo clean && cargo build`, resulting in a successful build despite warnings about unused code.

#### What's Left to Build
- Implement remaining path-based methods (`rename`, `symlink`) in `BranchFS` following the established pattern of checking for branch points and returning `EPERM` if detected, or forwarding to the branch's filesystem.
- Extend test coverage in `tests_good.rs` and `tests_bad.rs` for newly implemented methods to validate successful operations and error handling.
- Address build warnings related to unused code in `src/branch/mod.rs`, such as `EmptyFS` not being constructed and unused fields in `CacheEntry`, to maintain code cleanliness.
- Enhance documentation and comments in the codebase to reflect the current implementation, design decisions, and test results for better maintainability.

### Current Status
- The project builds successfully after resolving past compilation issues.
- The `BranchFS` implementation has core functionality for root directory handling, branch integration, and file operations in place, with extensive test coverage for both passing and failing scenarios.
- Path-based methods `create`, `unlink`, `mkdir`, and `rmdir` are implemented and passing tests in a layered filesystem configuration.
- Testing is well-advanced, with many tests passing as confirmed by review of test files.

### Known Issues
- Build warnings indicate unused code in `src/branch/mod.rs`, which may need refactoring or commenting out to avoid confusion or future maintenance issues.
- Full implementation of some `Filesystem` trait methods in `BranchFS` (e.g., `rename`, `symlink`) is still pending, with potential for methods returning `ENOSYS` (not implemented) in certain cases.
- Current implementation returns `EPERM` for operations on branch points, with a TODO note for potential future enhancements to handle cross-branch operations.

The project adheres to principles such as no use of elevated privileges (no sudo), a component-based architecture, and clear separation of concerns. Git submodules manage external open-source projects as components. Within this context, the migration of the experimental fuser crate to idiomatic Rust patterns is critical for improving maintainability, type safety, and performance. The Poll feature, along with Notify and Passthrough, requires special attention due to the need for ongoing communication channels between layers.

## Architectural Plan for Poll
This section details the architectural plan for migrating the Poll feature in the experimental fuser crate variant to handle asynchronous notifications in a layered userspace FUSE filesystem. The approach addresses the challenge of propagating file readiness notifications from lower data-generating layers to an upper layer responsible for kernel interaction, without kernel mediation.

### Problem Statement
In a standard FUSE setup, the kernel acts as a central intermediary: when an application calls poll() on a FUSE-mounted file, the kernel invokes the FUSE daemon's poll operation, and when the daemon determines the file is ready, it calls fuse_notify_poll() to inform the kernel, which wakes up the waiting application. For the fuser crate variant, the goal is to create a stack of userspace FUSE layers where:
- Each layer is agnostic to whether it's communicating with a kernel or another userspace layer.
- Notifications about file readiness propagate up the userspace stack.
- The system supports both synchronous (pull-based) and asynchronous (push-based, multi-threaded) consumption of notifications.
The core difficulty is enabling a lower layer, which owns the data and knowledge of file readiness, to efficiently and asynchronously inform an upper layer responsible for the poll response to the kernel.

### Why Previous Approaches Were Refined
Initial ideas were explored and refined to address limitations:
- **Callbacks and Asynchronicity**: An initial thought was for the lower layer to directly call a callback function provided by the upper layer when a file becomes ready. This was refined due to the complexity of state management in multi-threaded, asynchronous environments, as callbacks tightly couple execution flow and complicate concurrency reasoning, especially with await or non-blocking operations. A decoupled, message-passing approach was preferred.
- **Coarse-Grained Shared Mutex**: Another idea was using a single Arc<Mutex<FSelData>> for all shared state (byte counts, poll handles, notification masks). This was refined because a coarse-grained mutex introduces significant contention; if the upper layer locks to process notifications, it blocks the lower layer from updates, reducing parallelism. Finer-grained locking was needed.

### Chosen Architecture: Concurrent Queues and Layered Ownership
The refined architecture leverages concurrent message queues and distinct ownership of shared state to achieve a highly decoupled, concurrent, and resilient layered system tailored for the fuser crate's Poll feature migration.

#### Core Principles
- **Decoupling via Message Passing**: Layers communicate readiness events using a multi-producer, multi-consumer (MPMC) channel, allowing the lower layer to "push" events without waiting for the upper layer, and the upper layer to "pull" events when ready.
- **Finer-Grained Locking**: Shared state is split into logical units, each protected by its own Arc<Mutex>, minimizing contention and allowing concurrent operation of different system parts.
- **Lower Layer Owns Poll Handles**: The lower layer, closer to data and events, stores the kernel-provided poll_handle for each file, using it directly in notification messages.
- **Dynamic Channel Re-establishment**: The system handles scenarios where the communication channel might break (e.g., if the upper layer's receiver is dropped) by allowing the lower layer to accept a new Sender dynamically.

#### Key Components & Shared State
The system comprises two main logical layers: LowerLayer (data generation, event detection) and UpperLayer (kernel interaction, notification processing), adapted for the fuser crate context.

##### LowerLayerData
This struct encapsulates state primarily owned by the lower layer, protected by Arc<Mutex> for safe, concurrent access from internal threads and upper layer delegations (e.g., poll or read requests).
- **Details Stubbed**: Specific internal fields (e.g., data arrays and state trackers) are not detailed here to maintain focus. For full implementation details, refer to the example in `experiment/fuser/examples/poll.rs`, which provides the basis for this structure.
- **ready_events_sender: Option<Sender<(u64, u32)>>**: Sending end of the MPMC channel, wrapped in Option for dynamic replacement or disconnection. The tuple represents (poll_handle, events_bitmask).

**Key Methods on LowerLayerData:**
- `new(initial_sender: Option<Sender<(u64, u32)>>)`: Constructor, optionally takes an initial sender.
- `set_ready_events_sender(&mut self, new_sender: Sender<(u64, u32)>)`: Allows replacing the sender for dynamic channel re-establishment.
- `register_poll_handle(&mut self, file_idx: usize, ph: u64, events_requested: u32)`: Stores the poll handle and checks if the file is already ready, sending an initial notification if so.
- `send_ready_event(&self, file_idx: usize, events: u32)`: Pushes a (poll_handle, events) tuple onto the channel, handling send errors gracefully.
- `increment_byte_count(&mut self, file_idx: usize)`: Simulates data generation, notifying if a poll handle is registered.
- `get_byte_count(&self, file_idx: usize)` and `consume_bytes(&mut self, file_idx: usize, amount: u64)`: Allow querying and modifying data.

##### LowerLayer
This struct is the public interface to LowerLayerData, holding an Arc<Mutex<LowerLayerData>> and exposing interaction methods.
- **data: Arc<Mutex<LowerLayerData>>**: Reference to shared lower layer data.

**Key Methods on LowerLayer:**
- `new(data: Arc<Mutex<LowerLayerData>>)`: Constructor.
- `set_notification_sender(&self, new_sender: Sender<(u64, u32)>)`: Updates the sender within LowerLayerData.
- `handle_poll_request(...)`: Called by UpperLayer for FUSE poll requests, registers the poll handle via LowerLayerData.
- `run_generation_loop()`: Spawns a thread to continuously update byte counts and send notifications.
- `read_bytes(...)`: Delegates read requests to LowerLayerData.

##### UpperLayer
This struct represents the kernel-facing layer, receiving FUSE requests, delegating poll requests to LowerLayer, and consuming readiness notifications to send fuse_reply_poll (simulated in the example, adapted for fuser).
- **lower_layer: Arc<LowerLayer>**: Reference to LowerLayer for delegating FUSE operations.
- **ready_events_receiver: Receiver<(u64, u32)>**: Receiving end of the MPMC channel.

**Key Methods on UpperLayer:**
- `new(lower_layer: Arc<LowerLayer>, receiver: Receiver<(u64, u32)>)`: Constructor.
- `handle_fuse_poll_request(...)`: Entry point for FUSE poll requests, delegates to LowerLayer.
- `run_notification_loop()`: Spawns a thread to wait on ready_events_receiver, processing messages to notify the kernel.
- `handle_fuse_read_request(...)`: Delegates read requests to LowerLayer.

##### Notification Channel (crossbeam_channel::unbounded)
- An unbounded MPMC channel is used, ensuring senders don't block, suitable for event notifications without immediate backpressure.
- Sender is held by LowerLayerData, Receiver by UpperLayer.
- Messages are (u64, u32) tuples representing (poll_handle, events_bitmask).

#### Detailed Interaction Flow
##### Initialization
1. Create a new crossbeam_channel pair (tx, rx).
2. Initialize LowerLayerData with Some(tx) (the sender), wrapped in Arc<Mutex>.
3. Create LowerLayer with a reference to Arc<Mutex<LowerLayerData>>.
4. Create UpperLayer with references to LowerLayer and rx (the receiver).
5. Call lower_layer.run_generation_loop() to spawn a data generation thread.
6. Call upper_layer.run_notification_loop() to spawn a notification processing thread.

##### Application Poll Request (Simulated for fuser Context)
1. An application (or FUSE kernel binding) makes a poll request for a file, providing a poll_handle (ph) and requested events.
2. UpperLayer::handle_fuse_poll_request() is invoked.
3. UpperLayer delegates to LowerLayer::handle_poll_request().
4. LowerLayer acquires the mutex on LowerLayerData.
5. LowerLayerData::register_poll_handle() stores ph in self.poll_handles[file_idx], checks if the file is ready (e.g., bytecnt > 0), and sends an initial notification via send_ready_event() if so.
6. Mutex is released, LowerLayer returns to UpperLayer, which returns to the "kernel."

##### Lower Layer Data Generation & Notification
1. LowerLayer's run_generation_loop thread periodically acquires the LowerLayerData mutex.
2. Calls LowerLayerData::increment_byte_count() for a file.
3. If a poll_handle is registered, send_ready_event(ph, POLLIN) is called.
4. send_ready_event() sends the (ph, POLLIN) tuple via ready_events_sender if present, logging errors if sending fails.
5. Mutex is released.

##### Upper Layer Notification Processing
1. UpperLayer's run_notification_loop thread blocks on ready_events_receiver.recv().
2. On receiving (ph, events), it processes the message.
3. Simulates fuse_reply_poll(ph, events) to notify the "kernel" (in fuser, this would be the actual kernel interaction).
4. Continues waiting for the next notification.

##### Handling Channel Disconnection and Re-establishment
1. **Detection**: If ready_events_receiver.recv() returns Err, indicating a dropped Sender or disconnection, UpperLayer logs the error and exits the loop.
2. **Re-establishment**:
   - A managing component (or UpperLayer) detects the broken connection.
   - Creates a new (Sender, Receiver) pair: let (tx_new, rx_new) = unbounded();.
   - Calls lower_layer.set_notification_sender(tx_new) to update the communication endpoint.
   - Updates ready_events_receiver to rx_new and restarts run_notification_loop (via new thread or internal swap).

#### Benefits of this Design
- **Strong Decoupling**: Layers communicate via messages, not direct calls, so the lower layer doesn't need to know how the upper layer processes notifications, only that it should send them.
- **High Concurrency**: Finer-grained mutexes minimize contention, allowing concurrent data generation and notification processing.
- **Asynchronous Compatibility**: The channel supports asynchronous operations; recv() can block a thread or be awaited in an async runtime.
- **Resilience**: Dynamic set_notification_sender allows channel re-establishment without rebuilding the lower layer.
- **Clear Ownership**: Poll handles are managed by the lower layer, which has direct knowledge of file state.
- **Maintainability**: Separation of concerns makes layers easier to understand, test, and modify independently.

#### Code Example for Implementation Guidance
Below is a simplified code snippet demonstrating the core structure and interaction for Poll notification handling, adapted for the fuser crate context. This uses `crossbeam_channel` for MPMC communication, which can be integrated into the fuser implementation. Specific internal details of data structures have been stubbed to maintain focus; for complete code, refer to the example in `experiment/fuser/examples/poll.rs`.

```rust
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use crossbeam_channel::{unbounded, Sender, Receiver};
use std::io::{self, ErrorKind};

type RequestMeta = ();
type Errno = io::Error;
const NUMFILES: u16 = 8;

struct LowerLayerData {
    // Internal fields stubbed; refer to experiment/fuser/examples/poll.rs for details
    ready_events_sender: Option<Sender<(u64, u32)>>,
}

impl LowerLayerData {
    fn new(initial_sender: Option<Sender<(u64, u32)>>) -> Self {
        LowerLayerData {
            // Internal initialization stubbed
            ready_events_sender: initial_sender,
        }
    }

    pub fn set_ready_events_sender(&mut self, new_sender: Sender<(u64, u32)>) {
        self.ready_events_sender = Some(new_sender);
    }

    fn register_poll_handle(&mut self, file_idx: usize, ph: u64, events_requested: u32) -> Result<(), Errno> {
        // Implementation details stubbed; refer to experiment/fuser/examples/poll.rs
        Ok(())
    }

    fn send_ready_event(&self, file_idx: usize, events: u32) {
        // Implementation details stubbed; refer to experiment/fuser/examples/poll.rs
    }

    fn increment_byte_count(&mut self, file_idx: usize) {
        // Implementation details stubbed; refer to experiment/fuser/examples/poll.rs
    }
}

struct LowerLayer {
    data: Arc<Mutex<LowerLayerData>>,
}

impl LowerLayer {
    fn new(data: Arc<Mutex<LowerLayerData>>) -> Self {
        LowerLayer { data }
    }

    pub fn set_notification_sender(&self, new_sender: Sender<(u64, u32)>) {
        self.data.lock().unwrap().set_ready_events_sender(new_sender);
    }

    pub fn handle_poll_request(&self, _req: RequestMeta, ino: u64, _fh: u64, ph: u64, events: u32, _flags: u32) -> Result<u32, Errno> {
        let file_idx = ino as usize;
        let mut d = self.data.lock().unwrap();
        d.register_poll_handle(file_idx, ph, events)?;
        Ok(events)
    }

    fn run_generation_loop(&self) {
        let data_clone = Arc::clone(&self.data);
        thread::spawn(move || {
            let mut current_file_idx = 0;
            loop {
                {
                    let mut d = data_clone.lock().unwrap();
                    d.increment_byte_count(current_file_idx);
                }
                current_file_idx = (current_file_idx + 1) % (NUMFILES as usize);
                thread::sleep(Duration::from_millis(100));
            }
        });
    }
}

struct UpperLayer {
    lower_layer: Arc<LowerLayer>,
    ready_events_receiver: Receiver<(u64, u32)>,
}

impl UpperLayer {
    fn new(lower_layer: Arc<LowerLayer>, receiver: Receiver<(u64, u32)>) -> Self {
        UpperLayer {
            lower_layer,
            ready_events_receiver: receiver,
        }
    }

    pub fn handle_fuse_poll_request(&self, req: RequestMeta, ino: u64, fh: u64, ph: u64, events: u32, flags: u32) -> Result<u32, Errno> {
        self.lower_layer.handle_poll_request(req, ino, fh, ph, events, flags)
    }

    fn run_notification_loop(&self) {
        let receiver_clone = self.ready_events_receiver.clone();
        thread::spawn(move || {
            loop {
                match receiver_clone.recv() {
                    Ok((ph, events)) => {
                        println!("Notifying kernel: Poll handle {} is ready with events {}", ph, events);
                    }
                    Err(e) => {
                        eprintln!("Error receiving poll notification: {}", e);
                        break;
                    }
                }
            }
        });
    }
}

fn main() {
    let (tx_initial, rx_initial) = unbounded();
    let lower_data_arc = Arc::new(Mutex::new(LowerLayerData::new(Some(tx_initial))));
    let lower_layer = Arc::new(LowerLayer::new(Arc::clone(&lower_data_arc)));
    let upper_layer = UpperLayer::new(Arc::clone(&lower_layer), rx_initial);
    lower_layer.run_generation_loop();
    upper_layer.run_notification_loop();
    let _ = upper_layer.handle_fuse_poll_request((), 0, 1, 12345, libc::POLLIN as u32, 0);
}
```

This code snippet illustrates the setup of MPMC channels, layer initialization, poll handle registration, and notification processing, providing a practical starting point for implementing the Poll feature in the fuser crate. Full implementation details, including internal data structures and method implementations, are available in the example at `experiment/fuser/examples/poll.rs`. This reference should be used to adapt these structures to the specific Filesystem and Session interactions within fuser, replacing simulated data generation with actual filesystem event detection.

## Conceptual Application to Other Features
To extend the learnings from Poll to other features—Notify (inval_entry/inval_inode and store/delete) and Passthrough—the following conceptual approach is proposed based on the core principles of decoupled message passing and layered ownership:

1. **Core Principle Adaptation**: For Notify and Passthrough, which also require back-and-forth communication, establish dedicated MPMC channels (or a single channel with tagged messages) to handle their specific needs, mirroring Poll's asynchronous communication pattern.

2. **Notify (inval_entry, inval_inode, store, delete)**: These methods currently use a ChannelSender for kernel notifications about cache invalidation or data updates. Create specific channels for each Notify operation type. The lower layer detects filesystem changes (e.g., invalid entries or new data) and pushes structured messages (e.g., inode, operation type, data range). The upper layer consumes these and translates them into kernel notifications using existing Notification structs. This mirrors Poll's flow of event detection and notification without direct coupling.

3. **Passthrough (BackingId handling)**: Passthrough manages file descriptor mappings via ioctls. Establish an MPMC channel for passthrough events where the lower layer (filesystem logic) pushes messages about file open/release with backing_id details. The upper layer (session logic) receives these to perform ioctls (e.g., FUSE_DEV_IOC_BACKING_OPEN or CLOSE), decoupling state management from kernel interaction, similar to Poll's dynamic channel handling for resilience.

4. **Shared Design Elements**: Maintain finer-grained locking across features to minimize contention, ensure lower layer ownership of critical state (e.g., inode metadata for Notify, backing_id for Passthrough), and implement dynamic channel re-establishment for robustness, as in Poll's design.

5. **Key Difference Handling**: While Poll focuses on readiness events, Notify deals with state changes, and Passthrough with resource mappings. Message payloads differ—Poll sends (poll_handle, events), Notify might send (notification_type, relevant_ids, data), and Passthrough might send (backing_id, operation)—but the communication pattern remains consistent, allowing a unified model for back-and-forth interactions via asynchronous queues.

## Implementation Steps for Poll Migration
To execute the migration of the Poll feature in the experimental fuser crate variant, follow these specific steps to integrate the described architectural plan into the existing codebase. These steps are designed to adapt the conceptual LowerLayer and UpperLayer structures to the fuser crate's Filesystem and Session interactions, replacing non-idiomatic callbacks with MPMC channel-based communication.

1. **Identify Poll-Related Components in fuser**:
   - Locate the current implementation of Poll functionality within the fuser crate under `experiment/fuser/src`. Focus on modules or structs handling `poll` operations, likely within the `Filesystem` trait implementation or related session management code.
   - Review how callbacks or synchronous returns are currently used for Poll notifications, identifying entry points where kernel poll requests are received and where readiness notifications are sent back.

2. **Integrate MPMC Channel Dependency**:
   - Add `crossbeam-channel` as a dependency in `experiment/fuser/Cargo.toml` if not already present, to enable unbounded MPMC channel communication:
     ```toml
     [dependencies]
     crossbeam-channel = "0.5"
     ```
   - Ensure the dependency is compatible with the fuser crate's existing dependencies to avoid conflicts.

3. **Define Lower Layer Equivalent**:
   - Create or adapt a struct in the fuser codebase to represent the lower layer (data/event source), likely within the filesystem implementation. This struct should manage file state relevant to Poll (e.g., readiness indicators) and store poll handles provided by the kernel.
   - Implement state management similar to `LowerLayerData`, using `Arc<Mutex<...>>` for thread-safe access to shared state like poll handles and readiness flags.
   - Add methods to register poll handles and detect readiness events, pushing notifications to an MPMC channel sender.

4. **Define Upper Layer Equivalent**:
   - Identify or create a struct for the upper layer (kernel interaction), likely within the session or daemon management code of fuser, responsible for receiving kernel requests and sending notifications back.
   - Implement a receiver for the MPMC channel to process readiness notifications, translating them into kernel-compatible responses (e.g., using fuser's mechanisms for `fuse_reply_poll` equivalents).
   - Ensure this layer delegates poll requests to the lower layer for handle registration.

5. **Establish Channel Communication**:
   - Initialize an unbounded MPMC channel pair (Sender, Receiver) during the setup of the fuser filesystem or session.
   - Assign the Sender to the lower layer struct for sending readiness events, wrapped in `Option` to support dynamic replacement.
   - Assign the Receiver to the upper layer struct for processing notifications, spawning a background thread or using an async task to continuously handle incoming messages.

6. **Implement Poll Request Handling**:
   - Modify the existing Poll request handler in fuser to delegate to the lower layer for registering poll handles, as outlined in the interaction flow.
   - Ensure the lower layer checks for immediate readiness upon registration and sends initial notifications if the file is already ready.

7. **Implement Event Detection and Notification**:
   - Adapt the lower layer to detect filesystem events relevant to Poll (e.g., data availability for reading), replacing simulated data generation with actual event detection logic tied to fuser's filesystem operations.
   - Send notifications via the MPMC channel when events occur, using the stored poll handle and appropriate event bitmask (e.g., POLLIN).

8. **Handle Channel Disconnection and Re-establishment**:
   - Implement logic to detect channel disconnection (e.g., send or receive errors) and re-establish communication by creating a new channel pair and updating the sender and receiver endpoints.
   - Ensure the upper layer can restart its notification loop if disconnected, maintaining system resilience.

9. **Test Implementation**:
   - Develop test cases to validate the Poll migration, focusing on:
     - Correct registration of poll handles.
     - Asynchronous notification delivery upon file readiness.
     - Handling of channel disconnection and re-establishment.
   - Integrate these tests into fuser's existing test suites (e.g., `tests_good.rs` for passing scenarios, `tests_bad.rs` for error handling).
   - Use integration tests to confirm correct behavior in a layered filesystem setup, similar to existing `fuse_mounted_tests.rs`.

10. **Document and Refine**:
    - Update internal documentation within the fuser codebase to reflect the new Poll architecture, detailing the use of MPMC channels and layer responsibilities.
    - Address any build warnings or unused code issues identified during integration, ensuring code cleanliness.
    - Refine the implementation based on test results and performance metrics to optimize concurrency and minimize overhead.

**Considerations**:
- Ensure compatibility with fuser's existing API and feature flags (e.g., ABI versions), adjusting struct and method signatures as needed.
- Monitor for potential contention in mutex usage, optimizing lock scopes to maintain high concurrency.
- Consider async runtime integration (e.g., tokio) if fuser supports it, adapting channel operations to use async/await for better scalability.

This plan, with the detailed architectural guidance and code example provided, equips an agent to execute the Poll feature migration effectively within the experimental fuser crate variant.
