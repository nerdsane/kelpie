# Plan: Fix memory_tools_dst.rs Mock Implementations

**Issue**: #112 - Fix memory_tools_dst.rs mock implementations
**Created**: 2026-01-29
**Status**: Complete

## Investigation Summary

### Issue Claims vs Reality

The issue claims:
> Uses mock implementations instead of real code with interface swap. Missing simulated time control. Fault injection not demonstrated despite being imported.

**Investigation findings:**

1. **File location**: `crates/kelpie-server/tests/memory_tools_dst.rs` (not kelpie-dst)

2. **Mock implementation confirmed**: The file defines a `SimAgentMemory` struct with inline handler closures that simulate memory operations - this is NOT using the real code path.

3. **Real implementation exists**: `crates/kelpie-server/tests/memory_tools_real_dst.rs` already implements the correct pattern:
   - Uses `AppState::with_fault_injector()` for real fault injection
   - Uses `register_memory_tools()` to register REAL tools
   - Tools delegate to real `AppState` methods with swappable `AgentStorage` trait

4. **Fault injection IS demonstrated**: The mock file does use fault injection (FaultInjector, FaultConfig), but it's injected into mock handlers, not real code paths.

### The Problem

The old `memory_tools_dst.rs` file violates the FDB "same code path" principle:
- **Mock file**: Tests mock `SimAgentMemory` handlers (NOT production code)
- **Real file**: Tests actual `tools/memory.rs` → `AppState` → `SimStorage` (same code as production)

The mock tests provide false confidence because bugs in the real implementation won't be caught.

## Decision: Delete the Mock File

### Options Considered

| Option | Pros | Cons |
|--------|------|------|
| **A: Delete mock file** | Clean, no duplication, follows FDB principle | Lose ~900 lines (but redundant) |
| **B: Migrate mock to real** | Preserves test cases | Already done in `memory_tools_real_dst.rs` |
| **C: Keep both** | More coverage | Violates DST principles, false confidence |

**Decision**: Option A - Delete the mock file

**Rationale**:
1. `memory_tools_real_dst.rs` already has comprehensive coverage (687 lines)
2. The mock file tests fake code, providing no real value
3. FDB principle: "The same code must run in production and simulation"
4. Keeping duplicates creates maintenance burden and confusion

## Implementation Plan

### Phase 1: Verify Real File Coverage ✅
- [x] Confirm `memory_tools_real_dst.rs` covers all test scenarios
- [x] Both files test: append, replace, archival insert, archival search, conversation search
- [x] Real file adds: TOCTOU race detection, recovery tests, concurrent access

### Phase 2: Delete Mock File ✅
- [x] Remove `crates/kelpie-server/tests/memory_tools_dst.rs`
- [x] Run tests to confirm nothing breaks

### Phase 3: Verify ✅
- [x] Run `cargo test -p kelpie-server --features dst`
- [x] Run `cargo clippy`
- [x] Run `cargo fmt --check`

### Phase 4: Create PR
- [ ] Commit with message: "fix(dst): Remove mock memory_tools_dst.rs in favor of real implementation"
- [ ] Push and create PR referencing "Closes #112"

## Quick Decision Log

| Time | Decision | Rationale |
|------|----------|-----------|
| Initial | Delete mock file | Real implementation already exists with better coverage |

## What to Try

### Works Now
- `memory_tools_real_dst.rs` tests real code with fault injection
- Uses `AppState::with_fault_injector()` for interface swap
- Tests concurrent access and TOCTOU races

### Doesn't Work Yet
- N/A - all work complete

### Known Limitations
- None - straightforward deletion

## Verification Results

- All 22 memory tools tests pass:
  - `memory_tools_real_dst.rs`: 10 tests
  - `memory_tools_simulation.rs`: 12 tests
- Clippy: Clean
- Formatting: Clean
