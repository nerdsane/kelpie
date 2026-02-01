# kelpie-registry

**Examined:** 2026-01-23T00:02:00.716449+00:00

## Summary

Local-only in-memory registry - no distributed consensus, FoundationDB integration planned but not implemented

## Connections

- kelpie-cluster
- kelpie-runtime

## Details

- MemoryRegistry: RwLock<HashMap> based, all state lost on restart
- Actor placement: Local tracking with generation-based versioning
- Single activation: Checked locally via HashMap lookup, NOT distributed lock/lease
- Heartbeat: Failure detection (Active/Suspect/Failed states) but local-only
- FoundationDB backend: Documented as 'planned' but not implemented
- No distributed consensus: No Raft/Paxos, cannot prevent split-brain across nodes

## Issues

### [HIGH] Single activation guarantee is local-only - no distributed enforcement

**Evidence:** registry.rs: uses RwLock<HashMap>, no distributed lock or lease

### [HIGH] FoundationDB registry backend not implemented despite being planned

**Evidence:** lib.rs: '- Multiple backends (Memory, FoundationDB planned)'

### [MEDIUM] All registry state lost on restart - no persistence

**Evidence:** registry.rs: 'All state is lost on restart'
