# kelpie-cluster

**Examined:** 2026-01-24T15:22:01.357633+00:00

## Summary

Cluster coordination with heartbeat, migration protocol, but join_cluster() is stub - single-node only

## Connections

- kelpie-registry
- kelpie-runtime
- kelpie-core

## Details

**Cluster State Machine:**
- Stopped → Initializing → Running → ShuttingDown → Stopped
- Only tracks THIS node's state, not cluster-wide

**Join/Leave Protocol:**
- join_cluster(): STUB (Phase 3) - iterates seed nodes but does nothing
- leave_cluster(): PARTIAL - broadcasts but doesn't wait for acks

**Consensus: NONE**
- No Raft, Paxos, or PBFT
- Designed to use FDB for consensus (Phase 3)
- Single-node registry only (MemoryRegistry)

**Heartbeat:**
- Sending: IMPLEMENTED - periodic broadcast with metrics
- Reception: IMPLEMENTED - updates registry, sends ACK
- Timeout detection: delegated to registry

**Failure Detection:**
- Detects failed nodes from registry
- Calls plan_migrations() but DOES NOT EXECUTE (Phase 6)
- Just logs "planning" then discards

**Migration Protocol (3-phase):**
- Phase 1 Prepare: IMPLEMENTED
- Phase 2 Transfer: IMPLEMENTED  
- Phase 3 Complete: IMPLEMENTED
- Orchestration: IMPLEMENTED but NEVER CALLED

**Transport:**
- MemoryTransport: FULLY IMPLEMENTED (testing)
- TcpTransport: STUB (reader_task incomplete)

## Issues

### [HIGH] join_cluster() is stub - does nothing, single-node only

**Evidence:** cluster.rs:423-435 iterates seeds but takes no action

### [HIGH] Failure detection runs but never executes migrations

**Evidence:** cluster.rs:566 TODO(Phase 6)

### [MEDIUM] No consensus algorithm - relies on FDB not yet integrated

**Evidence:** lib.rs comments: No consensus algorithm - Designed to use FDB

### [MEDIUM] TcpTransport incomplete - reader_task truncated

**Evidence:** rpc.rs TCP implementation partial
