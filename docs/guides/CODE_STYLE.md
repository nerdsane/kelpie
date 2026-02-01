# Code Style Guide

## Module Organization

```rust
//! Module-level documentation with TigerStyle note
//!
//! TigerStyle: Brief description of the module's invariants.

// Imports grouped by: std, external crates, internal crates, local modules
use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use kelpie_core::{ActorId, Error, Result};

use crate::internal_module;
```

## Struct Layout

```rust
/// Brief description
///
/// Longer description if needed.
#[derive(Debug, Clone)]
pub struct ActorContext<S> {
    // Public fields at top with documentation
    /// The actor's unique identifier
    pub id: ActorId,
    /// The actor's state
    pub state: S,

    // Private fields below
    kv: Box<dyn ActorKV>,
    runtime: ActorRuntime,
}
```

## Function Signatures

```rust
/// Brief description of what the function does.
///
/// # Arguments
/// * `key` - The key to look up
///
/// # Returns
/// The value if found, None otherwise
///
/// # Errors
/// Returns `Error::StorageReadFailed` if the storage operation fails
pub async fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
    // Preconditions
    assert!(!key.is_empty(), "key cannot be empty");
    assert!(key.len() <= KEY_LENGTH_BYTES_MAX);

    // Implementation...
}
```

## Testing Guidelines

### Test Naming

```rust
#[test]
fn test_actor_id_valid() { }           // Positive case
#[test]
fn test_actor_id_too_long() { }        // Edge case
#[test]
fn test_actor_id_invalid_chars() { }   // Error case
```

### Property-Based Testing

Use proptest for invariant testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_actor_id_roundtrip(namespace in "[a-z]{1,10}", id in "[a-z0-9]{1,10}") {
        let actor_id = ActorId::new(&namespace, &id).unwrap();
        let serialized = serde_json::to_string(&actor_id).unwrap();
        let deserialized: ActorId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(actor_id, deserialized);
    }
}
```

### DST Test Coverage

Every critical path must have DST coverage:
- [ ] Actor activation/deactivation
- [ ] State persistence and recovery
- [ ] Cross-actor invocation
- [ ] Failure detection and recovery
- [ ] Migration correctness

## Error Handling

### Error Types (kelpie-core)

```rust
#[derive(Error, Debug)]
pub enum Error {
    #[error("actor not found: {id}")]
    ActorNotFound { id: String },

    #[error("storage read failed for key '{key}': {reason}")]
    StorageReadFailed { key: String, reason: String },

    // ...
}
```

### Result Type

```rust
// All fallible operations return kelpie_core::Result
pub type Result<T> = std::result::Result<T, Error>;
```

### Retriable Errors

```rust
impl Error {
    /// Whether this error is retriable
    pub fn is_retriable(&self) -> bool {
        matches!(self,
            Error::StorageReadFailed { .. } |
            Error::NetworkTimeout { .. } |
            Error::TransactionConflict
        )
    }
}
```

## Performance Guidelines

### Allocation

- Prefer stack allocation for small, fixed-size data
- Use `Bytes` for byte buffers (zero-copy slicing)
- Pool allocations for hot paths

### Async

- Use `tokio` runtime with `current_thread` flavor for DST
- Avoid blocking operations in async contexts
- Use channels for cross-task communication

### Benchmarking

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench -p kelpie-runtime -- single_actor
```

## Documentation

### ADRs (Architecture Decision Records)

All significant architectural decisions are documented in `docs/adr/`:

- `001-virtual-actor-model.md` - Why virtual actors
- `002-foundationdb-integration.md` - Storage layer design
- `003-wasm-actor-runtime.md` - WASM support
- `004-linearizability-guarantees.md` - Consistency model
- `005-dst-framework.md` - Testing approach

### Code Documentation

- All public items must have doc comments
- Include examples for complex APIs
- Document invariants and safety requirements
