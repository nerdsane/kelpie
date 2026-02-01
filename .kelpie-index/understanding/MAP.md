# Codebase Map

**Task:** Verify statement: Distributed virtual actor system with linearizability guarantees for AI agent orchestration
**Generated:** 2026-01-23T00:12:24.523573+00:00
**Components:** 6
**Issues Found:** 13

---

## Components Overview

### kelpie-agent

**Summary:** Placeholder stub only - no AI agent implementation exists

**Connects to:** kelpie-runtime

**Details:**

- All modules commented out: agent, memory, orchestrator, tool
- Only contains empty `pub struct Agent;` placeholder
- Documentation describes planned features for Phase 5
- No LLM integration, no tool calling, no memory management
- No actual AI agent orchestration code exists in this crate

**Issues (1):**
- [HIGH] kelpie-agent is a stub - no AI agent implementation

---

### kelpie-cluster

**Summary:** Distributed cluster coordination scaffolding - framework exists but many critical implementations are stubs

**Connects to:** kelpie-registry, kelpie-runtime, kelpie-core

**Details:**

- RPC transport layer with TCP and memory backends
- Migration protocol defined (Prepare/Transfer/Complete) but handlers not fully implemented
- Cluster join/leave messages defined but consensus absent
- Heartbeat-based failure detection configured
- Actor placement delegates to registry
- Critical gaps: No linearizability enforcement, no consensus algorithm, seed node join is stub

**Issues (4):**
- [HIGH] Cluster join implementation is stub - seed node loop does nothing
- [HIGH] No consensus algorithm - no Raft/Paxos for membership agreement
- [HIGH] Incoming RPC message handler is stub
- [HIGH] Migration execution is planned but never executes

---

### kelpie-core

**Summary:** Core types for virtual actor system - ActorId, ActorRef, ActorContext with location transparency claims

**Connects to:** kelpie-runtime, kelpie-storage

**Details:**

- ActorRef provides location transparency via `qualified_name()` abstraction
- Single-threaded execution guarantee documented in Actor trait
- TransactionalKV for atomic state+KV operations
- Linearizability claimed in module docs but implementation is in runtime
- No AI-specific types at core level - generic actor infrastructure

**Issues (1):**
- [LOW] AI agent orchestration claimed but no agent-specific abstractions in core

---

### kelpie-registry

**Summary:** Local-only in-memory registry - no distributed consensus, FoundationDB integration planned but not implemented

**Connects to:** kelpie-cluster, kelpie-runtime

**Details:**

- MemoryRegistry: RwLock<HashMap> based, all state lost on restart
- Actor placement: Local tracking with generation-based versioning
- Single activation: Checked locally via HashMap lookup, NOT distributed lock/lease
- Heartbeat: Failure detection (Active/Suspect/Failed states) but local-only
- FoundationDB backend: Documented as 'planned' but not implemented
- No distributed consensus: No Raft/Paxos, cannot prevent split-brain across nodes

**Issues (3):**
- [HIGH] Single activation guarantee is local-only - no distributed enforcement
- [HIGH] FoundationDB registry backend not implemented despite being planned
- [MEDIUM] All registry state lost on restart - no persistence

---

### kelpie-runtime

**Summary:** Local actor runtime with on-demand activation, single-threaded dispatcher, and transactional state persistence

**Connects to:** kelpie-core, kelpie-storage, kelpie-cluster

**Details:**

- Virtual actor activation: On-demand via Dispatcher.handle_invoke() - activates on first message
- Single activation guarantee: Enforced via single-threaded dispatcher loop (local only)
- Location transparency: ActorHandle routes by ActorId, physical location hidden
- Linearizability: FIFO mailbox + sequential dispatcher + transactional save_all_transactional()
- State atomicity: Snapshot/rollback on failure, atomic state+KV persistence
- Missing: Distributed coordination, cross-node routing, lease-based activation

**Issues (2):**
- [MEDIUM] Single activation guarantee is local-only, no distributed lock/lease
- [MEDIUM] max_pending_per_actor config defined but unused

---

### kelpie-storage

**Summary:** Dual-backend storage: FoundationDB provides linearizability via MVCC, MemoryKV for testing only

**Connects to:** kelpie-runtime, kelpie-core

**Details:**

- FdbKV: Production backend with FoundationDB integration
- Linearizability: FDB's MVCC provides linearizable reads/writes
- ACID: Atomic commit via FDB transactions, buffered writes, automatic conflict retry
- Read-your-writes: Transaction buffer checked before storage
- MemoryKV: Testing mock only, NOT linearizable (RwLock allows staleness)
- Per-actor isolation via FDB subspaces
- Missing: Range scans not transactional (phantom reads possible)

**Issues (2):**
- [LOW] Transaction finalization uses assert! instead of Result
- [MEDIUM] Range scans not transactional - phantom reads possible

---

## Component Connections

```
kelpie-agent -> kelpie-runtime
kelpie-cluster -> kelpie-registry, kelpie-runtime, kelpie-core
kelpie-core -> kelpie-runtime, kelpie-storage
kelpie-registry -> kelpie-cluster, kelpie-runtime
kelpie-runtime -> kelpie-core, kelpie-storage, kelpie-cluster
kelpie-storage -> kelpie-runtime, kelpie-core
```
