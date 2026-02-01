# docs/adr

**Examined:** 2026-01-24T19:24:58.487321+00:00

## Summary

24 ADRs covering actor model, storage, VM backends, transactions, agent API, teleport, and testing

## Connections

- docs/tla
- crates/kelpie-storage
- crates/kelpie-registry
- crates/kelpie-cluster

## Details

ADRs cover: ADR-001 (virtual actors), ADR-002 (FDB integration), ADR-004 (linearizability), ADR-005 (DST), ADR-007/008 (transactions), ADR-013/014 (agent service), ADR-015-021 (VM/teleport). Many reference TLA+ but none have direct spec mappings documented.

## Issues

### [HIGH] ADRs don't reference which TLA+ specs verify their claims

**Evidence:** No ADR mentions a TLA+ spec by name except ADR-004 which mentions lease protocol but no spec file

### [MEDIUM] Lease protocol in ADR-004 has no corresponding lease TLA+ spec mapping

**Evidence:** KelpieLease.tla exists but ADR-004 doesn't reference it
