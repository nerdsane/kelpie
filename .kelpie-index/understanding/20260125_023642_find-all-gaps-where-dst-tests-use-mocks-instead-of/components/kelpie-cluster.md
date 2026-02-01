# kelpie-cluster

**Examined:** 2026-01-25T02:36:34.006127+00:00

## Summary

Cluster crate has NO TimeProvider and uses real network - 80 async functions untestable via DST

## Connections

- kelpie-dst
- kelpie-registry
- kelpie-runtime

## Details

7 files analyzed:
- rpc.rs: 32 async functions, uses real network (tokio::net) - CRITICAL GAP
- cluster.rs: 18 async functions, has gossip protocol - GAP
- handler.rs: 18 async functions - GAP
- migration.rs: 11 async functions - GAP
- lib.rs: 1 async function
- config.rs, error.rs: No async code

Total: 80 async functions, NONE have TimeProvider injection.
rpc.rs uses real network - cannot be tested with SimNetwork.
Gossip protocol timing cannot be tested under simulated time.

## Issues

### [HIGH] rpc.rs uses real network (tokio::net) - cannot test with SimNetwork

**Evidence:** cluster_analysis shows rpc.rs: uses_network=true, 32 async functions

### [HIGH] kelpie-cluster has 0% TimeProvider coverage - 80 async functions without time injection

**Evidence:** All 7 files show has_time_injection=false

### [HIGH] Gossip protocol cannot be tested under simulated time

**Evidence:** cluster.rs has gossip logic but no time injection
