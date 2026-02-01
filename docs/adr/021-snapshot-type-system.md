# ADR-021: Snapshot Type System for Teleportation

## Status
Accepted

## Context

Kelpie needs to support different teleportation scenarios with varying requirements:

1. **Same-host pause/resume**: Developer pauses work, resumes later on same machine
2. **Same-architecture teleport**: Mid-execution migration to another host with same CPU (ARM64→ARM64 or x86_64→x86_64)
3. **Cross-architecture transfer**: Migration between ARM64 and x86_64 at safe points

Each scenario has different constraints:
- Same-host can use memory-only snapshots (fast, small)
- Same-architecture can capture full VM state (memory + CPU registers + disk)
- Cross-architecture cannot capture CPU state (registers are architecture-specific)

## Decision

Implement three distinct snapshot types:

### SnapshotKind::Suspend
- **Purpose**: Fast same-host pause/resume
- **Contents**: VM memory state only
- **Size**: ~50MB typical
- **Speed**: <1s
- **Constraint**: Same host only (memory addresses must match)

### SnapshotKind::Teleport
- **Purpose**: Full VM migration within same architecture
- **Contents**: VM memory + CPU registers + disk state
- **Size**: ~500MB-2GB typical
- **Speed**: ~5s
- **Constraint**: Same CPU architecture required

### SnapshotKind::Checkpoint
- **Purpose**: Application-level state for cross-architecture transfer
- **Contents**: Agent state + workspace (no VM state)
- **Size**: ~10-100MB typical
- **Speed**: <1s
- **Constraint**: Must be at "safe point" (not mid-syscall)

## Architecture Validation

Added `Architecture` enum with compile-time detection:
```rust
pub enum Architecture {
    Arm64,   // Apple Silicon, AWS Graviton
    X86_64,  // Intel/AMD
}

impl Architecture {
    pub fn current() -> Self {
        #[cfg(target_arch = "aarch64")]
        { Architecture::Arm64 }
        #[cfg(target_arch = "x86_64")]
        { Architecture::X86_64 }
    }
}
```

Validation enforced on restore:
- Suspend/Teleport: `source_arch == target_arch` required
- Checkpoint: Any architecture allowed

## Base Image Version Validation

Snapshots include `base_image_version` field. On restore, version must match to ensure:
- Same kernel version
- Same system libraries
- Same agent runtime

## Consequences

### Positive
- Clear separation of concerns for different use cases
- Type safety prevents invalid operations (e.g., cross-arch Teleport)
- DST can test each type independently with appropriate faults
- Users can choose optimal type for their scenario

### Negative
- Three code paths to maintain
- Users must understand which type to use (mitigated by auto-selection based on target)
- Checkpoint requires "safe point" coordination (future work)

### Neutral
- Snapshot format version bumped to v2 (breaking change from v1)
- API change: `Snapshot::new()` now requires `SnapshotKind` parameter

## DST Coverage

13 DST tests verify behavior under faults:
- Suspend with crash faults
- Teleport with storage/corruption faults
- Checkpoint with state faults
- Architecture validation (ARM64↔X86_64)
- Base image version validation
- Determinism verification
- Chaos testing (all types combined)

## Files Changed

- `crates/kelpie-sandbox/src/snapshot.rs` - New types and validation
- `crates/kelpie-sandbox/src/lib.rs` - Exports
- `crates/kelpie-dst/tests/snapshot_types_dst.rs` - DST tests
- Updated all sandbox implementations to use typed constructors

## References

- Plan: `.progress/009_20260114_teleportable_sandboxes_libkrun.md`
- ADR-020: Consolidated VM Crate
- CONSTRAINTS.md: DST-first development requirement
