# formatting-issues

**Examined:** 2026-01-24T21:07:15.787938+00:00

## Summary

cargo fmt --check fails with 7 formatting violations in 2 files

## Connections

- kelpie-server

## Details

**Files with formatting issues:**

1. **crates/kelpie-server/tests/common/invariants.rs** (6 issues):
   - Line 242: Long write!() macro call needs multi-line formatting
   - Line 390: Function signature too long (verify_capacity_bounds)
   - Line 573: Long if-let statement (verify_lease_validity)
   - Line 596: Long if-let statement (verify_lease_exclusivity)

2. **crates/kelpie-server/tests/tla_bug_patterns_dst.rs** (3 issues):
   - Line 20: Long use statement with 5 imported functions
   - Line 94: Long panic!() call
   - Line 362: println! call needs multi-line formatting

**Fix:**
Run `cargo fmt` to auto-fix all formatting issues.

## Issues

### [MEDIUM] cargo fmt --check fails with 7 formatting violations

**Evidence:** 2 files need reformatting: common/invariants.rs and tla_bug_patterns_dst.rs
