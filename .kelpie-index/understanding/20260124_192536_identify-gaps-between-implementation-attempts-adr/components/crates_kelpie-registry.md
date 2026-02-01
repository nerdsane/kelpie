# crates/kelpie-registry

**Examined:** 2026-01-24T19:25:24.404822+00:00

## Summary

Registry crate with placement, heartbeat tracking, node management, but no leases

## Connections

- docs/tla/KelpieLease.tla
- docs/tla/KelpieRegistry.tla
- docs/tla/KelpieSingleActivation.tla
- docs/adr/004-linearizability-guarantees.md

## Details

Files: registry.rs, placement.rs (generation-based), heartbeat.rs (HeartbeatTracker), node.rs, fdb.rs. Uses heartbeat-based failure detection (Active→Suspect→Failed). Single activation via compare-and-set in try_claim_actor(). No lease TTL or renewal.

## Issues

### [CRITICAL] KelpieLease.tla models lease-based ownership but implementation uses heartbeats only

**Evidence:** No LeaseManager, no TTL expiration, no lease renewal in registry impl

### [HIGH] Placement has no distributed coordination - relies on external FDB but FDB integration incomplete

**Evidence:** fdb.rs exists but ADR-004 says 'FDB backend integration not started'

### [HIGH] Generation counter alone insufficient for single activation - two nodes could both see gen=1

**Evidence:** placement.rs: no atomic read-check-write with FDB
