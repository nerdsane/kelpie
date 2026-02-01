# Codebase Map

**Task:** Identify gaps between implementation attempts, ADR coverage, and TLA+ models
**Generated:** 2026-01-24T19:25:36.576941+00:00
**Components:** 6
**Issues Found:** 17

---

## Components Overview

### crates/kelpie-cluster

**Summary:** Cluster crate with migration, RPC, handlers, but no split-brain prevention

**Connects to:** docs/tla/KelpieMigration.tla, docs/tla/KelpieClusterMembership.tla, docs/adr/004-linearizability-guarantees.md

**Details:**

Files: cluster.rs, migration.rs (3-phase), handler.rs (RPC handlers), rpc.rs, config.rs. Migration has Prepare→Transfer→Complete phases but no source deactivation step. Join protocol marked 'not implemented'. No primary election or quorum.

**Issues (4):**
- [CRITICAL] KelpieClusterMembership.tla models split-brain prevention but implementation has none
- [CRITICAL] Migration lacks explicit source deactivation - can violate single activation
- [HIGH] No persistent migration journal - crashes lose in-flight migration state
- [MEDIUM] Join protocol not implemented

---

### crates/kelpie-registry

**Summary:** Registry crate with placement, heartbeat tracking, node management, but no leases

**Connects to:** docs/tla/KelpieLease.tla, docs/tla/KelpieRegistry.tla, docs/tla/KelpieSingleActivation.tla, docs/adr/004-linearizability-guarantees.md

**Details:**

Files: registry.rs, placement.rs (generation-based), heartbeat.rs (HeartbeatTracker), node.rs, fdb.rs. Uses heartbeat-based failure detection (Active→Suspect→Failed). Single activation via compare-and-set in try_claim_actor(). No lease TTL or renewal.

**Issues (3):**
- [CRITICAL] KelpieLease.tla models lease-based ownership but implementation uses heartbeats only
- [HIGH] Placement has no distributed coordination - relies on external FDB but FDB integration incomplete
- [HIGH] Generation counter alone insufficient for single activation - two nodes could both see gen=1

---

### crates/kelpie-server

**Summary:** Server crate with agent actors, API handlers, dispatcher, but relies on incomplete lower crates

**Connects to:** crates/kelpie-registry, crates/kelpie-storage, docs/adr/013-actor-based-agent-server.md, docs/adr/014-agent-service-layer.md

**Details:**

46 files including agent_actor.rs, registry_actor.rs, API handlers. Implements AgentActor with state management, AgentService layer. Relies on kelpie-registry and kelpie-storage for distributed guarantees which are incomplete.

**Issues (2):**
- [HIGH] Server relies on registry single-activation but registry lacks lease-based guarantees
- [MEDIUM] AgentActor crash recovery depends on incomplete WAL recovery

---

### crates/kelpie-storage

**Summary:** Storage crate with WAL, KV traits, memory backend, and FDB backend stub

**Connects to:** docs/tla/KelpieWAL.tla, docs/adr/002-foundationdb-integration.md, docs/adr/008-transaction-api.md

**Details:**

Files: kv.rs (traits), wal.rs (WAL with Pending/Complete/Failed states), transaction.rs, memory.rs (in-memory), fdb.rs (FDB backend). WAL has idempotency checking but no automatic recovery. FDB file exists but needs verification of completeness.

**Issues (3):**
- [HIGH] WAL idempotency check is not atomic - race condition between find and insert
- [HIGH] WAL has no automatic crash recovery - only provides pending_entries()
- [MEDIUM] MemoryWal provides no durability - test-only

---

### docs/adr

**Summary:** 24 ADRs covering actor model, storage, VM backends, transactions, agent API, teleport, and testing

**Connects to:** docs/tla, crates/kelpie-storage, crates/kelpie-registry, crates/kelpie-cluster

**Details:**

ADRs cover: ADR-001 (virtual actors), ADR-002 (FDB integration), ADR-004 (linearizability), ADR-005 (DST), ADR-007/008 (transactions), ADR-013/014 (agent service), ADR-015-021 (VM/teleport). Many reference TLA+ but none have direct spec mappings documented.

**Issues (2):**
- [HIGH] ADRs don't reference which TLA+ specs verify their claims
- [MEDIUM] Lease protocol in ADR-004 has no corresponding lease TLA+ spec mapping

---

### docs/tla

**Summary:** 10 TLA+ specs covering WAL, Registry, SingleActivation, Lease, Migration, Teleport, FDBTransaction, ClusterMembership, ActorState, ActorLifecycle

**Connects to:** docs/adr, crates/kelpie-storage, crates/kelpie-registry, crates/kelpie-cluster

**Details:**

Each spec has safety invariants, liveness properties, and BUGGY mode for testing. Specs model: WAL (idempotency, recovery), Registry (single activation, failure detection), Lease (TTL, renewal), Migration (3-phase, crash recovery), Teleport (architecture validation), FDB (serializable isolation), Cluster (membership, split-brain), ActorState (rollback), ActorLifecycle (activation ordering).

**Issues (3):**
- [HIGH] KelpieLease.tla models lease-based ownership but implementation uses heartbeats, not leases
- [HIGH] KelpieClusterMembership.tla models split-brain prevention but implementation has none
- [HIGH] KelpieWAL.tla models recovery but implementation has no automatic recovery

---

## Component Connections

```
crates/kelpie-cluster -> docs/tla/KelpieMigration.tla, docs/tla/KelpieClusterMembership.tla, docs/adr/004-linearizability-guarantees.md
crates/kelpie-registry -> docs/tla/KelpieLease.tla, docs/tla/KelpieRegistry.tla, docs/tla/KelpieSingleActivation.tla, docs/adr/004-linearizability-guarantees.md
crates/kelpie-server -> crates/kelpie-registry, crates/kelpie-storage, docs/adr/013-actor-based-agent-server.md, docs/adr/014-agent-service-layer.md
crates/kelpie-storage -> docs/tla/KelpieWAL.tla, docs/adr/002-foundationdb-integration.md, docs/adr/008-transaction-api.md
docs/adr -> docs/tla, crates/kelpie-storage, crates/kelpie-registry, crates/kelpie-cluster
docs/tla -> docs/adr, crates/kelpie-storage, crates/kelpie-registry, crates/kelpie-cluster
```
