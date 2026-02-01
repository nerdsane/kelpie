# crates/kelpie-cluster

**Examined:** 2026-01-24T19:25:24.536347+00:00

## Summary

Cluster crate with migration, RPC, handlers, but no split-brain prevention

## Connections

- docs/tla/KelpieMigration.tla
- docs/tla/KelpieClusterMembership.tla
- docs/adr/004-linearizability-guarantees.md

## Details

Files: cluster.rs, migration.rs (3-phase), handler.rs (RPC handlers), rpc.rs, config.rs. Migration has Prepare→Transfer→Complete phases but no source deactivation step. Join protocol marked 'not implemented'. No primary election or quorum.

## Issues

### [CRITICAL] KelpieClusterMembership.tla models split-brain prevention but implementation has none

**Evidence:** No quorum, no election, no fencing in cluster impl

### [CRITICAL] Migration lacks explicit source deactivation - can violate single activation

**Evidence:** migration.rs: complete_migration() activates target without confirming source stopped

### [HIGH] No persistent migration journal - crashes lose in-flight migration state

**Evidence:** migration.rs: no WAL or checkpoint for migration state

### [MEDIUM] Join protocol not implemented

**Evidence:** handler.rs: 'ignoring join request (not implemented)'
