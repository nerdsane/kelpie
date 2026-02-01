# kelpie-cluster

**Examined:** 2026-01-23T01:02:14.272398+00:00

## Summary

Cluster coordination with migration, RPC, and config - for distributed actor management

## Connections

- kelpie-registry
- kelpie-runtime
- kelpie-storage

## Details

**Status: Implementation exists but needs verification**

**Modules:**
- cluster.rs - Cluster coordination logic
- config.rs - Cluster configuration
- error.rs - Cluster error types
- migration.rs - Actor migration between nodes
- rpc.rs - Inter-node RPC communication

**Note:** This component enables distributed features (single-activation across nodes, actor migration). Currently the runtime only has single-node support - cluster coordination would enable true distributed deployment.

## Issues

### [HIGH] Cluster coordination not integrated with runtime - distributed single-activation not enforced

**Evidence:** runtime activation.rs lacks distributed lock

### [MEDIUM] No tests found for kelpie-cluster in test index

**Evidence:** index_tests shows no kelpie-cluster tests
