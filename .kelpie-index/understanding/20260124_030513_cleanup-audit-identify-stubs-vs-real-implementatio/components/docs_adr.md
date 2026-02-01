# docs/adr

**Examined:** 2026-01-24T03:04:34.028267+00:00

## Summary

ADRs make distributed guarantees (single-activation, linearizability) that are ASPIRATIONAL, not implemented. Critical gap: ADR-004 promises CP behavior via FDB but FDB integration is not started. ADR-001/002/004 all claim single-activation but no working distributed mechanism exists.

## Connections

- kelpie-runtime
- kelpie-registry
- kelpie-cluster
- kelpie-storage

## Details

**ADR-001 (Virtual Actor Model):**
- Claims "single activation guarantee" enforced by registry
- Claims "failed actors transparently reactivated on healthy nodes"
- Status shows ✅ Complete for single-activation in dispatcher.rs
- REALITY: Dispatcher only has local HashMap check, no distributed coordination

**ADR-002 (FoundationDB Integration):**
- Claims FDB provides "linearizable transactions essential for single activation"
- Status shows ✅ Complete for FDB backend
- REALITY: FDB backend exists but has TOCTOU race, tests are ignored

**ADR-004 (Linearizability Guarantees):**
- Claims "lease-based ownership with automatic recovery"
- Claims "atomic lease acquisition via FDB transactions"
- REALITY: Implementation status shows "Not Started" for lease-based ownership
- Critical gap: Entire CP guarantee depends on unfinished FDB work

**ADR-005 (DST Framework):**
- Claims 49+ DST tests validate 7 distributed invariants
- Stateright integration is "scaffolded only"
- REALITY: Tests exist but may not validate claimed invariants

**Summary: ADRs document aspirational design, not current implementation.**

## Issues

### [HIGH] ADR-001/004 claim single-activation as Complete but no distributed mechanism exists

**Evidence:** ADR-001 status table shows Complete for dispatcher.rs only

### [HIGH] ADR-004 promises CP behavior via FDB but FDB lease integration Not Started

**Evidence:** ADR-004 implementation status: lease-based ownership Not Started

### [HIGH] ADRs document aspirational design as if implemented

**Evidence:** Multiple ✅ Complete markers for unimplemented features

### [MEDIUM] ADR-005 Stateright integration is scaffolded only, not functional

**Evidence:** ADR-005: Model implementation is aspirational pseudocode
