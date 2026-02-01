# kelpie-registry

**Examined:** 2026-01-23T01:03:14.735928+00:00

## Summary

Actor registry with node management, placement strategies, heartbeat tracking - 43 tests pass

## Connections

- kelpie-runtime
- kelpie-cluster
- kelpie-storage

## Details

**WORKING (43 tests pass):**
- NodeId with validation (alphanumeric, max 128 bytes)
- NodeInfo with heartbeat tracking, capacity management
- NodeStatus state machine (Joining→Active→Suspect/Leaving→Failed/Left)
- ActorPlacement with generation versioning, migration support
- PlacementStrategy enum (LeastLoaded, Random, Affinity, RoundRobin)
- PlacementContext builder pattern
- MemoryRegistry with RwLock-based state
- HeartbeatTracker with timeout detection
- Registry trait with CAS semantics for actor claim

**Features:**
- Node registration, unregistration, listing
- Actor registration, migration, conflict detection
- Heartbeat-based failure detection
- Load-based node selection

**Limitations:**
- In-memory only (no persistence)
- Single-node (no distributed coordination)
- RoundRobin falls back to LeastLoaded

## Issues

### [MEDIUM] PlacementStrategy defined but actual algorithms not implemented

**Evidence:** placement.rs - strategy enum unused in placement logic

### [MEDIUM] No actual network heartbeat sending - only tracking

**Evidence:** heartbeat.rs - send_heartbeats flag unused

### [LOW] Failed nodes stay tracked forever - memory leak risk

**Evidence:** heartbeat.rs - no cleanup for failed nodes
