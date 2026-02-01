# Plan: Issue #140 DST Quality Remediation

## Status: ✅ COMPLETE (2026-01-30)

## Summary

Fixed the two verified DST violations in the codebase. The investigation revealed that most claims in Issue #140 were incorrect - only 2 files had actual violations, not 8.

## Files Modified

### 1. `crates/kelpie-dst/tests/snapshot_types_dst.rs`

**Issue:** Custom `get_seed()` function used `println!` instead of `tracing::info!` and bypassed proper `SimConfig` integration.

**Fix:**
- Removed custom `get_seed()` function
- Updated all 14 tests to use `SimConfig::from_env_or_random()` with proper `config.seed` access
- Added `SimConfig` to imports

**Before:**
```rust
fn get_seed() -> u64 {
    std::env::var("DST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            let seed = rand::random();  // Uses rand::random()
            println!("DST_SEED={}", seed);  // Uses println, not tracing
            seed
        })
}
```

**After:**
```rust
let config = SimConfig::from_env_or_random();  // Proper logging via tracing
let rng = DeterministicRng::new(config.seed);
```

### 2. `crates/kelpie-dst/tests/simstorage_transaction_dst.rs`

**Issue:** Test helpers used non-deterministic sources:
- `chrono::Utc::now()` - 5 occurrences
- `uuid::Uuid::new_v4()` - 3 occurrences

**Fix:**
- Added thread-local DST context (`DST_CLOCK` and `DST_RNG`)
- Created `init_dst_context()` for test initialization
- Created `dst_now()` for deterministic timestamps from `SimClock`
- Created `dst_uuid()` for deterministic UUIDs from `DeterministicRng`
- Updated all 10 tests to initialize DST context at start
- Updated `test_agent()`, `test_block()`, `test_message()` helpers to use deterministic sources

## Verification

```bash
# All tests pass
cargo test -p kelpie-dst --test snapshot_types_dst
# 13 passed; 0 failed; 1 ignored

cargo test -p kelpie-dst --test simstorage_transaction_dst
# 10 passed; 0 failed; 0 ignored

# Reproducibility verified with fixed seed
DST_SEED=12345 cargo test -p kelpie-dst --test snapshot_types_dst
DST_SEED=12345 cargo test -p kelpie-dst --test simstorage_transaction_dst

# Full DST suite passes
cargo test -p kelpie-dst
# All tests pass

# Code quality checks pass
cargo clippy -p kelpie-dst -- -D warnings  # Clean
cargo fmt -p kelpie-dst --check  # No changes needed
```

## Issue #140 False Claims

The issue significantly overclaimed the problems. Here's the reality:

| Issue Claim | Actual Status |
|-------------|---------------|
| cluster_dst.rs violations | ❌ FALSE - Uses correct `from_env_or_random()` |
| sandbox_dst.rs violations | ❌ FALSE - Uses correct pattern |
| partition_tolerance_dst.rs violations | ❌ FALSE - Uses correct pattern |
| tools_dst.rs violations | ❌ FALSE - Uses correct pattern |
| vm_teleport_dst.rs violations | ❌ FALSE - Uses fixed seeds correctly |
| vm_backend_firecracker_chaos.rs violations | ❌ OVERSTATED - Minimal platform-gated test |
| **snapshot_types_dst.rs violations** | ✅ CONFIRMED - Fixed |
| **simstorage_transaction_dst.rs violations** | ✅ CONFIRMED - Fixed |

The issue author confused `SimConfig::from_env_or_random()` with being non-deterministic. This pattern IS correct FoundationDB-style DST:
1. Allows random exploration when developing tests
2. Always logs the seed via `tracing::info!` for reproduction
3. Supports `DST_SEED=12345 cargo test` for deterministic replay
