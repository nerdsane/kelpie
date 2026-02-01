# kelpie-cluster

**Examined:** 2026-01-24T03:02:07.686968+00:00

## Summary

RPC transport layer is REAL (TcpTransport with actual TCP I/O), but cluster coordination is largely STUB. join_cluster() does nothing, migration execution not wired up.

## Connections

- kelpie-registry
- kelpie-runtime

## Details

**REAL implementations:**
- TcpTransport: Real TCP socket I/O, length-prefixed JSON wire protocol, reader/writer tasks
- MemoryTransport: In-memory channels for testing
- MigrationCoordinator: Full 3-phase migration orchestration (prepare→transfer→complete)
- RpcHandler: All message types handled, actor invocation forwarding works
- Heartbeat task: Actually broadcasts heartbeats
- Failure detection task: Plans migrations (but doesn't execute)

**STUB implementations:**
- join_cluster() - Line 252-267: Logs seed nodes but does NOTHING. TODO(Phase 3)
- JoinRequest RPC handling - Returns None, not implemented
- ClusterStateRequest RPC - Returns None, not implemented
- TcpTransport accept_task - Uses fake node ID, no handshake protocol

**Critical gaps:**
- No consensus algorithm (Raft/Paxos) - multi-node membership not implemented
- Failure-triggered migrations planned but never executed (TODO Phase 6)
- MemoryTransport::connect() is broken - creates channels but drops receivers

## Issues

### [HIGH] join_cluster() is non-functional stub - multi-node deployment broken

**Evidence:** cluster.rs:252-267 TODO(Phase 3)

### [HIGH] Failure-triggered migrations never executed - actors on failed nodes not recovered

**Evidence:** cluster.rs:367-373 TODO(Phase 6)

### [HIGH] No consensus algorithm - cluster membership has no agreement protocol

**Evidence:** No Raft/Paxos code found in any file

### [MEDIUM] TcpTransport uses fake node ID on accept - no handshake protocol

**Evidence:** rpc.rs:607-611 temp_node_id fabricated

### [MEDIUM] MemoryTransport::connect() broken - receivers immediately dropped

**Evidence:** rpc.rs:226-235 _rx_to_other unused

### [MEDIUM] JoinRequest and ClusterStateRequest RPC handlers not implemented

**Evidence:** handler.rs:351-356 returns None
