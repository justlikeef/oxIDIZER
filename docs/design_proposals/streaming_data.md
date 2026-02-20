# Streaming Data Implementation Plan

## Goal
Enable modules to return large datasets record-by-record (streamed) without loading the entire dataset into memory.

## Proposed Changes

### 1. Update `ox_plugin` (Shared Definition)

Modify `ox_plugin/src/lib.rs`:

*   Add `FlowControl::StreamData` variant.
*   Define `OxStream` struct for the iterator interface.
*   Define `StreamChunk` for return data.

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    // ... existing ...
    StreamData = 5,
}

#[repr(C)]
pub struct StreamChunk {
    pub data: *const u8,    // Pointer to data
    pub len: usize,         // Length of data
    pub is_last: bool,      // Indicator if this is the last chunk (optional, or rely on len=0)
}

#[repr(C)]
pub struct OxStream {
    pub instance: *mut c_void, // Module-managed iterator state
    // Function to get next chunk. 
    // Returns StreamChunk. If len == 0, stream is finished.
    // The specific ownership model of 'data' must be defined (e.g., valid until next call).
    pub next: unsafe extern "C" fn(instance: *mut c_void) -> StreamChunk, 
    // Function to clean up the iterator instance
    pub free: unsafe extern "C" fn(instance: *mut c_void),
}
```

> [!NOTE]
> This `OxStream` interface is generic. It can be used for:
> *   **Database Rows**: Fetching and serializing one record at a time.
> *   **Files**: Reading a file chunk-by-chunk (useful if the file needs on-the-fly encryption or processing, or if the host cannot access the file path directly). passing raw files via `StreamFile` is still more optimized for static content (using OS `sendfile`), but `OxStream` provides a unified fallback.
> *   **Generated Content**: Infinite streams, SSE, etc.

### 2. Update `ox_webservice_api`
Re-export the new types (automatic if using `pub use ox_plugin::*`).

### 3. Update `ox_webservice` (Host)
Modify `src/pipeline.rs` -> `LoadedModule::execute` and `Pipeline::execute_pipeline`:

*   Handle `FlowControl::StreamData`.
*   When receiving this flow control, expect `return_parameters.return_data` to be a `*mut OxStream`.
*   Convert this raw C-pointer into a Rust `Stream` (using `futures::stream::unfold` or similar).
*   Wrap this stream in an `axum::body::Body::from_stream`.
*   This generic stream body will call the C `next` function on the worker thread (or blocking thread via `tokio::task::spawn_blocking` if the C-module is not async-aware) to pull pages of data.

## Verification Plan

### Automated Tests
*   Create a new test module `ox_test_stream_data` that implements `OxStream`.
    *   It should return a sequence of numbers or strings (e.g., "Row 1\n", "Row 2\n"...) with a slight delay or just iterating.
*   Create a functional test `900011-StreamData` that:
    *   Configures `ox_webservice` to use this module.
    *   Curls the endpoint.
    *   Verifies the chunked response contains all rows.

### Manual Verification
*   Verify memory usage is low (not loading a huge buffer).

## Industry Analysis (Research)

To inform this design, we analyzed how industry-standard web servers handle streaming:

### Nginx (Buffers & Chain Links)
*   **Architecture**: Uses `ngx_buf_t` (buffers) linked together by `ngx_chain_t` (chains).
*   **Abstraction**: A buffer can point to:
    *   Memory (RAM).
    *   File segment (File Descriptor + Offset + Length).
*   **Streaming**: Nginx processes these chains. If a buffer points to a file, it uses `sendfile()` (zero-copy) to send data directly from disk to socket, bypassing user-space memory.
*   **Filters**: "Filter Modules" modify these chains as they pass through (e.g., gzip compression).

### Apache (Bucket Brigades)
*   **Architecture**: Uses "Buckets" grouped into "Brigades".
*   **Abstraction**: Similar to Nginx, a bucket can be:
    *   Heap memory.
    *   Mmap’d memory.
    *   File (supports `sendfile`).
    *   Socket / Pipe.
    *   "Flush" or "EOS" (End of Stream) metadata buckets.
*   **Streaming**: Handlers pass Brigades to Output Filters.

### Recommendation for oxIDIZER
The initial `StreamChunk` proposal (pointer + len) is equivalent to a "Memory Buffer". To achieve parity with Nginx/Apache efficiency, we should allow `StreamChunk` to represent **File Regions** as well. This enables `ox_webservice` to use `sendfile` (Zero-Copy) for static assets served by plugins, while still supporting dynamic in-memory generation (e.g., templates) via the same `OxStream` interface.

## Refined Design: `OxChunk`

We should rename `StreamChunk` to `OxChunk` and make it a tagged union (or struct with flags) to support both memory and file sources.

```rust
#[repr(C)]
pub enum ChunkType {
    Memory = 0,
    FileRegion = 1,
}

#[repr(C)]
pub struct OxChunk {
    pub chunk_type: ChunkType,
    // Union-like behavior would be better, but for simplicity:
    pub data: *const u8,       // For Memory: pointer to data. For File: NULL (or ignored).
    pub len: usize,            // For Memory: length. For File: length to send.
    pub fd: i32,               // For File: File Descriptor. Ignored for Memory.
    pub file_offset: u64,      // For File: Offset. Ignored for Memory.
    pub is_last: bool,
}
```

This allows a plugin to return a mix: "Here is a memory header" -> "Here is 1GB from disk (sendfile)" -> "Here is a memory footer".

## Implementation Strategy: Use Cases

To validate this design, we will implement `OxStream` in two key modules:

### 1. `ox_webservice_stream` (Static Files)
This module currently returns `FlowControl::StreamFile` with a simple path string. It should be updated to use the unified `OxStream` interface.

*   **Implementation**:
    *   **State**: The `OxStream` instance state will hold the file path (or open file descriptor).
    *   **Iteration**: The `next` function will return a **single** `OxChunk` of type `FileRegion`.
        *   `chunk_type`: `FileRegion`
        *   `fd`: Open file descriptor (or path resolved by host if we keep path support). *Self-Correction: To support true zero-copy in the plugin, the plugin should likely open the FD or return the path if the Host API supports it. For simplicity, returning a FileRegion chunk with a Path/FD allows the Host to do the `sendfile`.*
        *   `is_last`: `true`.
    *   **Optimization**: This effectively aliases the old `StreamFile` behavior but standardizes it under the `OxStream` API.

### 2. `ox_data_object` (Database Records)
This module is more complex as it loads dynamic drivers (e.g., Postgres, SQLite) to fetch data.

*   **Challenge**: The current `PersistenceDriver::fetch` returns `Vec<GenericDataObject>`, which loads all results into memory.
*   **Required Changes**:
    1.  **Update `PersistenceDriver` Trait**:
        *   Add `fetch_cursor(&self, filter: ..., location: &str) -> Box<dyn DriverCursor>`.
        *   Define `DriverCursor` trait with `next(&mut self) -> Option<HashMap<...>>` (or potentially raw bytes/JSON to save serialization steps).
    2.  **Implement `OxStream`**:
        *   **State**: The `OxStream` instance will assume ownership of the `Box<dyn DriverCursor>`.
        *   **Iteration**: The `next` function will:
            *   Call `cursor.next()`.
            *   Serialize the result (e.g., to JSON lines).
            *   Return an `OxChunk` of type `Memory` containing the serialized bytes.
            *   If `cursor.next()` returns `None`, return a chunk with `len: 0` (End of Stream).
    3.  **Dynamic Loading**: Since drivers are loaded dynamically, `ox_data_object` acts as the bridge. The `OxStream` functions are defined in `ox_data_object`, but the *state* they operate on contains the driver-specific cursor created by the dynamically loaded driver.

### Summary
*   **ox_webservice_stream** -> `OxStream` -> `OxChunk::FileRegion` (1 chunk, Zero-Copy).
*   **ox_data_object** -> `OxStream` -> `OxChunk::Memory` (Many chunks, Serialized Rows).
