# kelpie-cluster

**Examined:** 2026-01-22T23:59:37.113567+00:00

## Summary

Distributed cluster coordination scaffolding - framework exists but many critical implementations are stubs

## Connections

- kelpie-registry
- kelpie-runtime
- kelpie-core

## Details

- RPC transport layer with TCP and memory backends
- Migration protocol defined (Prepare/Transfer/Complete) but handlers not fully implemented
- Cluster join/leave messages defined but consensus absent
- Heartbeat-based failure detection configured
- Actor placement delegates to registry
- Critical gaps: No linearizability enforcement, no consensus algorithm, seed node join is stub

## Issues

### [HIGH] Cluster join implementation is stub - seed node loop does nothing

**Evidence:** cluster.rs: for seed_addr in &self.config.seed_nodes { debug!(...); } does nothing

### [HIGH] No consensus algorithm - no Raft/Paxos for membership agreement

**Evidence:** No quorum-based consensus visible in any cluster file

### [HIGH] Incoming RPC message handler is stub

**Evidence:** rpc.rs: 'Received non-response message (handler not implemented for incoming)'

### [HIGH] Migration execution is planned but never executes

**Evidence:** cluster.rs: Plans migrations but loop only logs, no execution
