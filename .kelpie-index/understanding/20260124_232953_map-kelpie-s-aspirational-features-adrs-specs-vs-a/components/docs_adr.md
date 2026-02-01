# docs/adr

**Examined:** 2026-01-24T23:24:11.114428+00:00

## Summary

27 ADRs defining architectural commitments across actor model, storage, VM, cluster, agent, and tool integration

## Connections

- docs
- kelpie-core
- kelpie-runtime
- kelpie-dst
- kelpie-vm
- kelpie-cluster

## Details

ADR status breakdown:
- Accepted/Complete: 001, 002, 004, 005, 006, 009, 010, 012, 014, 016, 018, 020, 021, 022, 023, 024, 025, 026, 027
- Proposed: 003 (WASM), 011 (Agent Types)
- In Progress: 013 (Actor-Based Agent Server)
- Superseded: 017, 019 (by ADR-020)

Key verification artifacts referenced:
- 12 TLA+ specifications in docs/tla/
- DST test suite alignment documented
- Safety invariants: SingleActivation, Linearizability, Durability, PlacementConsistency
- Liveness properties: EventualRecovery, EventualCompletion

## Issues

### [MEDIUM] Many ADRs lack explicit acceptance criteria - they document what to build but not how to verify it's complete

**Evidence:** ADRs 003, 015, 016, 017, 018, 019, 026 have no explicit test requirements

### [MEDIUM] Status tracking inconsistent - some ADRs marked 'Accepted' despite incomplete implementation

**Evidence:** ADR-026 MCP integration marked 'Accepted' but all components marked 'Designed' not implemented
