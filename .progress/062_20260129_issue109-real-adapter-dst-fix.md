# Issue #109: Fix real_adapter_dst.rs Stub Tests

**Status**: COMPLETE
**Date**: 2026-01-29
**Issue**: #109

## Investigation Summary

### Issue Claims vs Reality

| Claim | Actual Status |
|-------|---------------|
| Tests are stubs | ✓ TRUE - 5 tests don't invoke RealLlmAdapter |
| Uses real tokio runtime | ✗ FALSE - Already migrated to madsim conditional |
| Tests don't invoke RealLlmAdapter | ✓ TRUE - for `real_adapter_dst.rs` |
| #[tokio::test] with real runtime | ✗ FALSE - Uses `#[cfg_attr(feature = "madsim", madsim::test)]` |

### Key Finding

There are THREE test files for RealLlmAdapter:

1. **`real_adapter_dst.rs`** - STUBS (5 tests) - Don't invoke RealLlmAdapter
2. **`real_adapter_http_dst.rs`** - REAL (3 tests) - Actually test RealLlmAdapter with HTTP mocking
3. **`real_adapter_simhttp_dst.rs`** - REAL (3 tests) - Actually test with network fault injection

The issue is correct that `real_adapter_dst.rs` contains stubs, but the REAL DST tests exist in the other two files.

## Analysis of Each Stub Test

| Test | What It Does | Is It Redundant? |
|------|--------------|------------------|
| `test_dst_real_adapter_chunk_count` | Asserts streaming chunks > batch chunks (constant comparison) | YES - `real_adapter_http_dst.rs::test_dst_real_adapter_uses_real_streaming` verifies actual chunk counts |
| `test_dst_real_adapter_fault_resilience` | Just confirms fault config is accepted | YES - Fault acceptance tested in many places |
| `test_dst_stream_delta_to_chunk_conversion` | Tests StreamDelta enum variants, NOT conversion | PARTIAL - Could be unit test, not DST |
| `test_dst_concurrent_streaming_with_faults` | Tests concurrent tasks with SimClock sleep | NO - But doesn't test adapter, just runtime |
| `test_dst_streaming_error_propagation` | Tests Error::Internal wrapping | YES - `real_adapter_http_dst.rs::test_dst_real_adapter_error_handling` tests real error path |

## Options Considered

### Option A: Convert Stubs to Real Tests (REJECTED)
- Pros: Fulfills original issue intent
- Cons: Would duplicate `real_adapter_http_dst.rs` and `real_adapter_simhttp_dst.rs`

### Option B: Delete Redundant File (SELECTED)
- Pros: Removes misleading stubs, reduces test maintenance
- Cons: Loses 1 non-redundant test (concurrent streaming)
- Mitigation: Move concurrent streaming test to `real_adapter_simhttp_dst.rs`

### Option C: Mark File as Deprecated with TODO (REJECTED)
- Pros: Documents issue without deleting
- Cons: Violates "No Placeholders" constraint from CONSTRAINTS.md

## Decision

**Option B: Delete `real_adapter_dst.rs`** with these modifications:

1. Migrate `test_dst_concurrent_streaming_with_faults` to `real_adapter_simhttp_dst.rs` and make it actually test RealLlmAdapter
2. Delete `real_adapter_dst.rs`
3. Document the coverage provided by the remaining two files

## Implementation Steps

1. [x] Migrate concurrent streaming test to `real_adapter_simhttp_dst.rs`
2. [x] Make the migrated test actually invoke RealLlmAdapter
3. [x] Delete `real_adapter_dst.rs`
4. [x] Run all tests with `--features dst` ✅
5. [x] Run clippy, fmt ✅
6. [x] Commit and push
7. [x] Create PR

## Implementation Progress (2026-01-29)

### Summary of Changes:

1. **Deleted `real_adapter_dst.rs`** - 5 stub tests that didn't invoke RealLlmAdapter
2. **Migrated concurrent streaming test** to `real_adapter_simhttp_dst.rs` as `test_dst_concurrent_adapter_streaming_with_faults`
3. **New test properly invokes RealLlmAdapter** - spawns 3 concurrent streaming tasks, each creating their own adapter and calling `stream_complete()`

### Final Test Coverage:

| File | Tests | Coverage |
|------|-------|----------|
| `real_adapter_http_dst.rs` | 3 | HTTP mocking, streaming chunks, error handling |
| `real_adapter_simhttp_dst.rs` | 4 | Network delays, packet loss, combined faults, concurrent streaming |

All 7 tests invoke `RealLlmAdapter.stream_complete()` with proper fault injection.

## Verification Commands

```bash
# Run all RealLlmAdapter DST tests
cargo test -p kelpie-server --features dst --test real_adapter_http_dst
cargo test -p kelpie-server --features dst --test real_adapter_simhttp_dst

# Verify with deterministic seed
DST_SEED=42 cargo test -p kelpie-server --features dst --test real_adapter_simhttp_dst
DST_SEED=42 cargo test -p kelpie-server --features dst --test real_adapter_http_dst

# Full verification
cargo test -p kelpie-server --features dst && cargo clippy -p kelpie-server -- -D warnings && cargo fmt --check
```

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Investigate before fixing | Issue claims might be incorrect | Spent time verifying |
| After investigation | Delete stubs, keep real tests | Stubs violate DST principles, real tests exist | Lose 1 non-redundant test |
| After analysis | Migrate concurrent test | Preserves useful coverage | Slight code churn |
