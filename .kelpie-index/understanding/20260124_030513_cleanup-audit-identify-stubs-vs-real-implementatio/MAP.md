# Codebase Map

**Task:** Cleanup audit: Identify stubs vs real implementations in kelpie-cluster, kelpie-agent; verify single-activation gaps; check ADR accuracy. Goal: authoritative issue list for Option B cleanup.
**Generated:** 2026-01-24T03:05:13.553855+00:00
**Components:** 6
**Issues Found:** 22

---

## Components Overview

### docs/adr

**Summary:** ADRs make distributed guarantees (single-activation, linearizability) that are ASPIRATIONAL, not implemented. Critical gap: ADR-004 promises CP behavior via FDB but FDB integration is not started. ADR-001/002/004 all claim single-activation but no working distributed mechanism exists.

**Connects to:** kelpie-runtime, kelpie-registry, kelpie-cluster, kelpie-storage

**Details:**

**ADR-001 (Virtual Actor Model):**
- Claims "single activation guarantee" enforced by registry
- Claims "failed actors transparently reactivated on healthy nodes"
- Status shows ✅ Complete for single-activation in dispatcher.rs
- REALITY: Dispatcher only has local HashMap check, no distributed coordination

**ADR-002 (FoundationDB Integration):**
- Claims FDB provides "linearizable transactions essential for single activation"
- Status shows ✅ Complete for FDB backend
- REALITY: FDB backend exists but has TOCTOU race, tests are ignored

**ADR-004 (Linearizability Guarantees):**
- Claims "lease-based ownership with automatic recovery"
- Claims "atomic lease acquisition via FDB transactions"
- REALITY: Implementation status shows "Not Started" for lease-based ownership
- Critical gap: Entire CP guarantee depends on unfinished FDB work

**ADR-005 (DST Framework):**
- Claims 49+ DST tests validate 7 distributed invariants
- Stateright integration is "scaffolded only"
- REALITY: Tests exist but may not validate claimed invariants

**Summary: ADRs document aspirational design, not current implementation.**

**Issues (4):**
- [HIGH] ADR-001/004 claim single-activation as Complete but no distributed mechanism exists
- [HIGH] ADR-004 promises CP behavior via FDB but FDB lease integration Not Started
- [HIGH] ADRs document aspirational design as if implemented
- [MEDIUM] ADR-005 Stateright integration is scaffolded only, not functional

---

### kelpie-agent

**Summary:** DELETED - kelpie-agent crate no longer exists in workspace. Agent implementation moved to kelpie-server.

**Connects to:** kelpie-server

**Details:**

The kelpie-agent crate was removed from Cargo.toml workspace members. The git status shows deleted files:
- crates/kelpie-agent/Cargo.toml (deleted)
- crates/kelpie-agent/src/lib.rs (deleted)

Agent functionality now lives in kelpie-server crate (actor/agent_actor.rs, api/agents.rs, service/mod.rs).

This is a cleanup from prior ISSUES.md which listed it as a stub. The stub has been purged.

**Issues (1):**
- [LOW] kelpie-agent references in ISSUES.md are stale - crate deleted

---

### kelpie-cluster

**Summary:** RPC transport layer is REAL (TcpTransport with actual TCP I/O), but cluster coordination is largely STUB. join_cluster() does nothing, migration execution not wired up.

**Connects to:** kelpie-registry, kelpie-runtime

**Details:**

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

**Issues (6):**
- [HIGH] join_cluster() is non-functional stub - multi-node deployment broken
- [HIGH] Failure-triggered migrations never executed - actors on failed nodes not recovered
- [HIGH] No consensus algorithm - cluster membership has no agreement protocol
- [MEDIUM] TcpTransport uses fake node ID on accept - no handshake protocol
- [MEDIUM] MemoryTransport::connect() broken - receivers immediately dropped
- [MEDIUM] JoinRequest and ClusterStateRequest RPC handlers not implemented

---

### kelpie-registry

**Summary:** Two implementations: MemoryRegistry (single-node, TOCTOU races) and FdbRegistry (distributed with leases, mostly complete but has TOCTOU in try_claim_actor). FDB implementation exists but tests are ignored (require external cluster).

**Connects to:** kelpie-runtime, kelpie-cluster, kelpie-storage

**Details:**

**MemoryRegistry (in-memory, single-node):**
- Uses RwLock<HashMap> - only works within single process
- TOCTOU race in try_claim_actor(): two separate lock acquisitions
- State lost on restart - no persistence
- Claims to be "linearizable" but only locally

**FdbRegistry (FoundationDB, distributed):**
- REAL implementation exists with lease-based single-activation
- Lease TTL: 30 seconds, renewal every 10 seconds
- Uses FDB transactions for atomicity
- ISSUE: Lease check is OUTSIDE transaction (lines 683-722)
  - Reads lease, checks if expired, THEN starts transaction to claim
  - Race: Node A reads expired, Node B renews, Node A claims anyway
- ISSUE: Async read before write - FDB 0.10 limitation workaround
- Tests marked #[ignore] - require running FDB cluster

**Both registries lack:**
- Distributed consensus for multi-node coordination
- Thundering herd mitigation on lease expiry
- Threshold-based failure handling in renewal task

**Issues (5):**
- [HIGH] MemoryRegistry has TOCTOU race in try_claim_actor() - two separate locks
- [HIGH] FdbRegistry lease check is outside transaction - TOCTOU between check and claim
- [MEDIUM] FDB tests ignored - no CI coverage for distributed code
- [MEDIUM] LeaseRenewalTask silent failures - renewal errors only logged, node keeps serving
- [MEDIUM] MemoryRegistry claims 'linearizable' but is single-node only

---

### kelpie-runtime

**Summary:** Single-activation guarantee is NOT ENFORCED. Local mode has TOCTOU race in HashMap check. Distributed mode has race window between get_placement() and try_claim_actor(). No lease/heartbeat mechanism.

**Connects to:** kelpie-registry, kelpie-cluster, kelpie-storage

**Details:**

**Analysis of dispatcher.rs (main enforcement point):**

**Local Mode (registry=None):**
- TOCTOU race: `if !self.actors.contains_key(&key)` check followed by non-atomic `activate_actor()` call
- Multiple concurrent requests can pass the check and create duplicate instances
- No mutex protection around activation path

**Distributed Mode (registry=Some):**
- Better: Uses `registry.try_claim_actor()` for atomic placement decision
- Gap: Non-atomic window between `get_placement()` check (line 404) and `try_claim_actor()` (line 450)
- Gap: No lease/heartbeat - actor ownership is permanent until explicit unregister
- Node crash = orphaned actors forever in registry

**activation.rs confirms:**
- `ActiveActor::activate()` has NO locking mechanism
- Multiple concurrent calls succeed independently
- `on_activate()` hook can run multiple times for same actor

**The claim "only one ActiveActor per ActorId can exist" is ASPIRATIONAL, not enforced.**

**Issues (5):**
- [HIGH] Local mode TOCTOU race allows duplicate actor activations
- [HIGH] Distributed mode has race window between get_placement() and try_claim_actor()
- [HIGH] No lease/heartbeat - node crash orphans actors forever
- [MEDIUM] unwrap() on mutex lock can panic if poisoned
- [MEDIUM] ActiveActor::activate() lacks any locking mechanism

---

### kelpie-server

**Summary:** PARTIAL implementation - architectural foundation is solid (AgentActor, RegistryActor, LlmClient trait, tool execution) but some functions appear truncated in analysis. Real LLM integration exists with streaming support.

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-core

**Details:**

**Real implementations:**
- AgentActor with state management
- RegistryActor for agent lifecycle
- LlmClient trait abstraction over real/simulated LLM
- Tool execution with UnifiedToolRegistry
- Streaming support with StreamChunk enum
- WAL wrapper for crash recovery

**File counts:**
- API handlers: 17 files
- Actor implementations: 5 files
- Service layer: 2 files
- Total: 45 files

**Architecture:**
- TigerStyle patterns applied (assertions, explicit state)
- Proper actor model using kelpie-runtime
- State is serializable for durability

**Note:** Analysis showed some truncated code which may be analysis artifact rather than actual stubs. The 70+ DST tests in kelpie-server suggest substantial real implementation.

**Issues (1):**
- [LOW] Some code analysis showed truncation - may need manual verification

---

## Component Connections

```
docs/adr -> kelpie-runtime, kelpie-registry, kelpie-cluster, kelpie-storage
kelpie-agent -> kelpie-server
kelpie-cluster -> kelpie-registry, kelpie-runtime
kelpie-registry -> kelpie-runtime, kelpie-cluster, kelpie-storage
kelpie-runtime -> kelpie-registry, kelpie-cluster, kelpie-storage
kelpie-server -> kelpie-runtime, kelpie-storage, kelpie-core
```
