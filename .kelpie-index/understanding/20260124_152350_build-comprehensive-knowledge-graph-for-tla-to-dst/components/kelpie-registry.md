# kelpie-registry

**Examined:** 2026-01-24T15:19:23.735107+00:00

## Summary

Actor placement registry with MemoryRegistry (testing) and FdbRegistry (production), lease mechanism, heartbeat tracking

## Connections

- kelpie-runtime
- kelpie-cluster
- kelpie-core

## Details

**MemoryRegistry try_claim_actor:**
- Single RwLock write lock covers check + insert
- ATOMIC within single process - no TOCTOU
- Sequential lock acquisition (placements then nodes) - low risk

**FdbRegistry try_claim_actor:**
- FIXED in register_actor (lines 700-760): read + write in same transaction
- Uses FDB conflict detection for linearizability
- Retry loop handles conflicts correctly

**Lease Mechanism:**
- Lease struct: node_id, acquired_at_ms, expires_at_ms, version
- is_expired() check, renew() with version bump
- Default duration: 30,000ms

**ZOMBIE ACTOR RISK (Critical):**
- Scenario: Node A holds lease, crashes, lease expires, Node B claims
- Node A still running → DUAL ACTIVATION
- Missing: heartbeat-lease coordination
- Missing: check if old node is alive before reclaiming

**Lease Renewal:**
- renew_lease() checks ownership and expiry
- SAFE: is_owned_by prevents renewal if different node owns

**Heartbeat Integration:**
- check_heartbeat_timeouts() tracks node health
- get_actors_to_migrate() for failover
- GAP: No coordination between heartbeat and lease expiry

## Issues

### [HIGH] Zombie actor risk: no coordination between heartbeat failure and lease expiry allows dual activation

**Evidence:** fdb.rs lease mechanism has no node-alive check

### [MEDIUM] try_claim_actor implementation may be incomplete - async reads in sync closure issue

**Evidence:** fdb.rs:603-620 shows _lease_value not awaited

### [LOW] Sequential lock acquisition in MemoryRegistry could allow stale node state

**Evidence:** registry.rs:330-360
