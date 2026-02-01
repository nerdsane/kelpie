# kelpie-dst

**Examined:** 2026-01-25T02:35:32.476670+00:00

## Summary

DST test infrastructure with 24 test files - many use mocks instead of production code

## Connections

- kelpie-runtime
- kelpie-registry
- kelpie-storage
- kelpie-cluster

## Details

Analysis of 24 DST test files:
- FULL_SIM (3): single_activation_dst, actor_lifecycle_dst, fdb_transaction_dst
- PARTIAL_SIM (3): agent_integration_dst, fdb_faults_dst, partition_tolerance_dst  
- MOCK_ONLY (2): liveness_dst, lease_dst
- UNKNOWN (16): Various tests without clear simulation strategy

Key finding: Even "FULL_SIM" tests often use HashMap-based mocks for state instead of actual production code with SimStorage.

## Issues

### [HIGH] 12 tests don't import any production crates - test algorithms only

**Evidence:** liveness_dst.rs, agent_integration_dst.rs, teleport_service_dst.rs, memory_dst.rs, vm_teleport_dst.rs, proper_dst_demo.rs, integration_chaos_dst.rs, bug_hunting_dst.rs

### [HIGH] 4 tests use Arc<RwLock<HashMap>> mocks instead of SimStorage for state

**Evidence:** single_activation_dst.rs uses ActivationProtocol with HashMap, partition_tolerance_dst.rs similar

### [HIGH] 2 tests use MemoryLeaseManager instead of production LeaseManager with SimStorage

**Evidence:** liveness_dst.rs, lease_dst.rs

### [MEDIUM] 7 tests don't use SimNetwork for RPC simulation

**Evidence:** Most tests lack network fault injection at I/O layer
