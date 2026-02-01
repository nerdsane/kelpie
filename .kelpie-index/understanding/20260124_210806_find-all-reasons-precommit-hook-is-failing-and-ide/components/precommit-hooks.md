# precommit-hooks

**Examined:** 2026-01-24T21:07:32.867060+00:00

## Summary

Pre-commit hook enforces 3 checks in sequence: fmt, clippy, test

## Connections

- formatting-issues
- clippy-warnings
- test-failures

## Details

The hook at `hooks/pre-commit` runs these checks:

1. **cargo fmt --check** - FAILS (7 formatting issues)
2. **cargo clippy --workspace --all-targets** - FAILS (compilation errors)
3. **cargo test --all** - Skipped if previous checks fail, would FAIL (11 test compilation errors)

**Hook behavior:**
- Uses `set -e` (exits on first error)
- Each check uses `run_check()` function that captures output
- If any check fails, FAILED=1 and hook exits with code 1
- Also checks for `.kelpie-index/constraints/extracted.json` for additional hard constraints (file doesn't exist, shows warning)

**Current state:**
The hook will fail at step 1 (cargo fmt --check), never reaching clippy or tests.

## Issues

### [HIGH] Pre-commit hook will fail on first check (cargo fmt)

**Evidence:** Hook runs fmt first, which fails with 7 formatting violations
