# kelpie-dst

**Examined:** 2026-01-24T23:28:02.333498+00:00

## Summary

COMPLETE - Deterministic simulation testing with SimClock, DeterministicRng, FaultInjector, 20+ fault types, liveness verification

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-storage
- kelpie-server

## Details

Core components:
- SimClock: Explicit time advancement (no wall clock dependency)
- DeterministicRng: ChaCha20-based, forkable, same seed = same output
- FaultInjector: Probabilistic injection with ~20 fault types
- BoundedLiveness: Temporal property verification (eventually, leads-to)
- InvariantChecker: System invariant validation
- SimVm, SimLlmClient, SimSandboxIO: Mock implementations

Test count: 113+ in lib, 200+ across all DST test files

DST_SEED environment variable ensures reproducibility.

## Issues

### [MEDIUM] SimNetwork and SimStorage referenced in lib.rs but not shown in analyzed code

**Evidence:** pub use statements exist but implementations may be incomplete

### [LOW] No clock-wait integration - can deadlock if never advanced

**Evidence:** SimClock::sleep waits on notify but requires external advance
