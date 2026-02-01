# clippy-warnings

**Examined:** 2026-01-24T21:07:22.318991+00:00

## Summary

clippy fails due to same compilation errors as test failures

## Connections

- test-failures
- kelpie-server

## Details

Clippy cannot run because the workspace fails to compile. Same E0599 errors as test suite:

- No function or associated item named `new_without_wal` 
- No method named `recover`

Clippy will only work once the test compilation errors are fixed.

## Issues

### [HIGH] clippy blocked by compilation errors

**Evidence:** Same E0599 errors prevent clippy from running: new_without_wal and recover methods missing
