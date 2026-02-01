# crates/kelpie-server

**Examined:** 2026-01-24T19:25:24.656357+00:00

## Summary

Server crate with agent actors, API handlers, dispatcher, but relies on incomplete lower crates

## Connections

- crates/kelpie-registry
- crates/kelpie-storage
- docs/adr/013-actor-based-agent-server.md
- docs/adr/014-agent-service-layer.md

## Details

46 files including agent_actor.rs, registry_actor.rs, API handlers. Implements AgentActor with state management, AgentService layer. Relies on kelpie-registry and kelpie-storage for distributed guarantees which are incomplete.

## Issues

### [HIGH] Server relies on registry single-activation but registry lacks lease-based guarantees

**Evidence:** Server assumes single activation but registry uses heartbeats not leases

### [MEDIUM] AgentActor crash recovery depends on incomplete WAL recovery

**Evidence:** ADR-013 mentions checkpoint every iteration but WAL has no auto-recovery
