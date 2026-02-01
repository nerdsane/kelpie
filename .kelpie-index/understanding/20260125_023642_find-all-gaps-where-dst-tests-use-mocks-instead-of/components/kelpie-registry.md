# kelpie-registry

**Examined:** 2026-01-25T02:36:03.600052+00:00

## Summary

Registry has good TimeProvider injection in core files, but fdb.rs (25 async fns) lacks it

## Connections

- kelpie-dst
- kelpie-storage
- foundationdb

## Details

8 files analyzed:
- registry.rs: Has TimeProvider ✓, 53 async functions - GOOD
- lease.rs: Has TimeProvider ✓, 20 async functions - GOOD
- node.rs: Has TimeProvider ✓ - GOOD
- placement.rs: Has TimeProvider ✓ - GOOD
- fdb.rs: NO TimeProvider, 25 async functions, uses FDB directly - CRITICAL GAP
- lib.rs: Uses FDB, 1 async function - minor
- heartbeat.rs: No async code
- error.rs: Type definitions only

Key issue: fdb.rs is the FDB backend with 25 async functions but no TimeProvider.
This means FDB operations can't be tested under simulated time.

## Issues

### [HIGH] fdb.rs has 25 async functions with FDB calls but no TimeProvider injection

**Evidence:** registry_analysis shows fdb.rs: uses_fdb=true, has_time_injection=false, async_functions=25

### [MEDIUM] FDB backend cannot be tested under simulated time - real FDB must be running

**Evidence:** fdb.rs directly uses foundationdb crate without abstraction
