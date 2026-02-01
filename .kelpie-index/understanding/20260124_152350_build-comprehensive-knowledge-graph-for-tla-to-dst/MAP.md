# Codebase Map

**Task:** Build comprehensive knowledge graph for TLA+ to DST alignment - extract invariants, state machines, concurrency patterns, TOCTOU risks
**Generated:** 2026-01-24T15:23:50.884531+00:00
**Components:** 14
**Issues Found:** 27

---

## Components Overview

### kelpie-agent

**Summary:** AI agent abstractions - REMOVED from workspace (crate deleted)

**Connects to:** kelpie-server

**Details:**

**Status:** DELETED
- Cargo.toml shows "D crates/kelpie-agent/Cargo.toml"
- Agent functionality moved to kelpie-server

**Note:** No longer a separate crate, skip for TLA+ analysis.

---

### kelpie-cli

**Summary:** CLI tools - minimal main.rs entry point for command-line utilities

**Connects to:** kelpie-server, kelpie-core

**Details:**

**Status:** Minimal
- Single main.rs file
- Command-line interface for kelpie operations

**Note:** Not relevant for TLA+ system invariants - just a UI layer.

---

### kelpie-cluster

**Summary:** Cluster coordination with heartbeat, migration protocol, but join_cluster() is stub - single-node only

**Connects to:** kelpie-registry, kelpie-runtime, kelpie-core

**Details:**

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

**Issues (4):**
- [HIGH] join_cluster() is stub - does nothing, single-node only
- [HIGH] Failure detection runs but never executes migrations
- [MEDIUM] No consensus algorithm - relies on FDB not yet integrated
- [MEDIUM] TcpTransport incomplete - reader_task truncated

---

### kelpie-core

**Summary:** Core types, constants (TigerStyle naming), error types with retriability, and compile-time invariants

**Connects to:** kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-dst

**Details:**

**Core Types:**
- ActorId: {namespace: String, id: String} with validation
- ActorRef: location-transparent wrapper
- Architecture: Arm64, X86_64
- SnapshotKind: Suspend, Teleport, Checkpoint
- StorageBackend: Memory, FoundationDb

**Constants (TigerStyle - THING_UNIT_MAX):**
- ACTOR_ID_LENGTH_BYTES_MAX = 256
- ACTOR_STATE_SIZE_BYTES_MAX = 10MB
- ACTOR_INVOCATION_TIMEOUT_MS_MAX = 30,000
- TRANSACTION_SIZE_BYTES_MAX = 10MB (FDB aligned)
- HEARTBEAT_INTERVAL_MS = 1,000
- HEARTBEAT_TIMEOUT_MS = 5,000
- DST_STEPS_COUNT_MAX = 10,000,000

**Error Types (Retriable):**
- TransactionConflict
- NodeUnavailable
- ActorInvocationTimeout

**Compile-Time Invariants:**
- HEARTBEAT_TIMEOUT_MS > HEARTBEAT_INTERVAL_MS
- ACTOR_ID_LENGTH_BYTES_MAX >= 64
- ACTOR_STATE_SIZE_BYTES_MAX <= 100MB
- ACTOR_INVOCATION_TIMEOUT_MS_MAX >= 1000

**TLA+ Foundation:**
- ActorId constraints well-defined
- Constant bounds explicit with units
- Error state machine clear (retriable vs non-retriable)

**Issues (1):**
- [LOW] StorageBackend::FoundationDb requires fdb_cluster_file but validation is runtime not compile-time

---

### kelpie-dst

**Summary:** Deterministic Simulation Testing framework with SimClock, SimStorage, SimNetwork, FaultInjector (41 fault types), and deterministic RNG

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-core

**Details:**

**Components:**
- SimClock: Controlled time, no wall-clock dependencies
- DeterministicRng: ChaCha20-based, seeded, forkable
- SimStorage: In-memory KV with size limits
- SimNetwork: Message queue with latency simulation
- FaultInjector: Probabilistic injection (41 fault types)
- SimLlmClient: Deterministic LLM responses

**Fault Types (41 total):**
- Storage: WriteFail, ReadFail, Corruption, Latency, DiskFull
- Crash: BeforeWrite, AfterWrite, DuringTransaction
- Network: Partition, Delay, PacketLoss, MessageReorder
- Time: ClockSkew, ClockJump
- Resource: OutOfMemory, CPUStarvation
- MCP: ServerCrash, SlowStart, ToolTimeout, ToolFail
- LLM: Timeout, Failure, RateLimited, AgentLoopPanic
- Sandbox/VM: BootFail, Crash, PauseFail, ResumeFail, ExecFail, ExecTimeout
- Snapshot: CreateFail, Corruption, RestoreFail, TooLarge
- Teleport: UploadFail, DownloadFail, Timeout, ArchMismatch, ImageMismatch

**Determinism:**
- RNG seeding via DST_SEED env var
- SimTime auto-advances SimClock
- No std::time::SystemTime usage
- Tokio task scheduling still non-deterministic

**INVARIANT CHECKING GAP:**
- No dedicated invariant verification module
- Tests use weak assertions: is_ok()/is_err() without extracting values
- No property-based testing (Proptest/QuickCheck)
- No temporal logic (LTL/MTL)

**STATERIGHT: NOT INTEGRATED**
- No Model trait implementations
- No state space exploration
- Framework CAN support it but doesn't require it

**Issues (4):**
- [HIGH] No invariant verification helpers - tests use weak is_ok()/is_err() assertions
- [HIGH] Stateright model checking not integrated - only pseudocode exists
- [MEDIUM] Missing fault types: ConcurrentAccessConflict, DeadlockDetection, DataRace, PartialWrite
- [MEDIUM] ClockSkew/ClockJump faults declared but never injected

---

### kelpie-memory

**Summary:** Three-tier memory system: Core (~32KB), Working (~1MB), Archival (embeddings) with checkpoint/restore

**Connects to:** kelpie-server, kelpie-storage, kelpie-core

**Details:**

**Memory Tiers:**
- Core Memory: Fixed capacity, explicit blocks with ordering
- Working Memory: KV with TTL, capacity-bounded
- Archival: Vector embeddings for semantic search (partial)

**Invariants:**
- Core: current_bytes ≤ max_bytes
- Working: current_bytes ≤ max_bytes
- Block: size_bytes ≤ 16KB
- block_order.len() == blocks.len()
- Checkpoint sequence strictly increasing

**Checkpoint:**
- Serialization snapshot (not WAL)
- No atomic checkpoint with state mutation

**Issues (3):**
- [HIGH] No thread safety - CoreMemory/WorkingMemory are Clone but not Arc<Mutex>
- [MEDIUM] Checkpoint not atomic with state mutations - crash during checkpoint loses state
- [MEDIUM] Expired entries still count toward capacity until pruned

---

### kelpie-registry

**Summary:** Actor placement registry with MemoryRegistry (testing) and FdbRegistry (production), lease mechanism, heartbeat tracking

**Connects to:** kelpie-runtime, kelpie-cluster, kelpie-core

**Details:**

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

**Issues (3):**
- [HIGH] Zombie actor risk: no coordination between heartbeat failure and lease expiry allows dual activation
- [MEDIUM] try_claim_actor implementation may be incomplete - async reads in sync closure issue
- [LOW] Sequential lock acquisition in MemoryRegistry could allow stale node state

---

### kelpie-runtime

**Summary:** Actor runtime with single-threaded dispatcher, ActivationState machine, backpressure, and transactional state persistence

**Connects to:** kelpie-registry, kelpie-storage, kelpie-core

**Details:**

**State Machine (ActivationState):**
- Deactivated → Activating → Active → Deactivating → Deactivated
- Critical transitions require state load/save

**Single Activation (Local Mode):**
- HashMap membership check + activation is atomic due to single-threaded dispatcher
- NO TOCTOU race - commands processed sequentially via command loop

**Single Activation (Distributed Mode):**
- TOCTOU race DETECTED but not PREVENTED at dispatcher.rs:512-530
- get_placement() → try_claim_actor() window allows dual activation
- Race is detected via PlacementDecision::Existing check, client gets error

**Dispatcher Guarantees:**
- Single-threaded command processing (line 480)
- Per-actor single-threadedness
- FIFO message ordering
- Backpressure at handle level (max_pending_per_actor)

**Concurrency:**
- HashMap<String, ActiveActor> NOT shared (dispatcher only)
- pending_counts: Arc<Mutex<HashMap>> for backpressure
- Arc<AtomicUsize> per actor for pending tracking

**Failure Recovery:**
- Actor panic: state rolled back, no auto-reactivation
- Dispatcher crash: no auto-restart
- State persistence failure: transaction rolled back, actor stays active

**Issues (3):**
- [MEDIUM] Distributed mode TOCTOU race detected but not prevented - client retry required
- [MEDIUM] Stale registry entries on node crash - no TTL-based cleanup
- [LOW] No auto-restart of dispatcher task on crash

---

### kelpie-sandbox

**Summary:** Sandbox execution environment with Mock, Firecracker backends; GenericSandbox<IO> pattern for DST

**Connects to:** kelpie-vm, kelpie-dst, kelpie-core

**Details:**

**Sandbox Types:**
- MockSandbox: In-memory simulation (testing)
- FirecrackerSandbox: MicroVM via KVM
- GenericSandbox<IO>: Pluggable I/O for DST

**State Machine:**
Stopped → Creating → Running ⇄ Paused → Stopped
Any → Destroying

**Invariants:**
- State pre-conditions: can_start(), can_pause(), can_exec() guards
- Exec only in Running state
- Snapshot only Running/Paused
- Exit status: signal=Some(n) ⇒ code=128+n
- Architecture compatibility on restore

**Issues (3):**
- [HIGH] State TOCTOU in Firecracker: state read then released then written, allowing interleaving
- [MEDIUM] Async I/O without atomicity - VM could be partially configured if task cancels
- [LOW] Process cleanup race - process might be dead when kill() called

---

### kelpie-server

**Summary:** Main server binary with actor-based AppState, AgentService, WAL-backed transactions, and comprehensive DST test coverage

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-registry, kelpie-dst

**Details:**

**State Machines:**
- AppState lifecycle: Uninitialized → Initialized → ShuttingDown → Shutdown
- AgentService request lifecycle: Pending → Processing → Completed/Failed/CrashedDuringTransaction
- WAL recovery enables Crashed → Completed transition

**Invariants Found:**
1. AppState initialization must be all-or-nothing (no partial state)
2. No duplicate agents from concurrent requests with same name
3. All created agents must be retrievable immediately after creation (or via WAL recovery)
4. In-flight requests during shutdown must complete OR fail - never silently drop
5. New requests after shutdown must be rejected with shutdown error
6. Agent data must not be corrupted after retrieval (name, system, tool_ids match)
7. Search must never return results from other agents (BUG-002 pattern)

**Concurrency:**
- Arc<AppState<R>> shared across concurrent tasks
- Arc<FaultInjector> for deterministic fault injection
- Arc<UnifiedToolRegistry> for tool execution
- RwLock<Vec<String>> for execution logging

**TOCTOU Risks:**
- Concurrent creation race: between name existence check and write
- BUG-001 pattern: between create return and get call (mitigated by WAL)
- Shutdown race: between shutdown initiation and rejection taking effect

**Issues (2):**
- [MEDIUM] Shutdown race between initiation and rejection needs atomic state transition
- [LOW] BUG-001/BUG-002 patterns documented but should be verified with DST

---

### kelpie-storage

**Summary:** Per-actor KV storage with WAL, transaction support, Memory and FDB backends

**Connects to:** kelpie-server, kelpie-runtime, kelpie-core

**Details:**

**WAL (Write-Ahead Log):**
- Operations: CreateAgent, UpdateAgent, SendMessage, DeleteAgent, UpdateBlock, Custom
- Entry: id, operation, actor_id, payload, status (Pending/Complete/Failed)
- Append durability: uses atomic transaction
- Idempotency: append_with_idempotency() deduplicates by key

**WAL Recovery Gap:**
- pending_entries() returns pending ops
- NO REPLAY MECHANISM in code - recovery coordinator missing
- Requires external coordinator to replay on startup

**Memory Transaction (Testing):**
- Write buffer until commit
- NOT ATOMIC: writes applied sequentially
- Crash vulnerability: partial writes persist

**FDB Transaction (Production):**
- STRONG atomicity: all ops in single FDB transaction
- ACID guarantees from FDB
- Retry logic with exponential backoff
- Crash-safe: FDB guarantees all-or-nothing

**Invariants:**
1. WAL Entry Uniqueness - idempotency key deduplication
2. Entry Status Monotonicity - Pending → Complete/Failed only
3. Transaction Finalization - exactly one commit/abort
4. Actor Isolation - namespace separation in key space
5. Write Buffer Boundedness - 10000 ops max (memory)

**Issues (3):**
- [HIGH] WAL has no replay mechanism - recovery coordinator missing
- [MEDIUM] Memory transaction not atomic - sequential writes, crash vulnerability
- [LOW] FDB batch size limit implicit - should be explicit

---

### kelpie-tools

**Summary:** Tool execution framework with UnifiedToolRegistry, builtin tools (filesystem, git, shell), MCP integration, and SimTool for DST

**Connects to:** kelpie-server, kelpie-sandbox, kelpie-dst

**Details:**

**Components:**
- UnifiedToolRegistry: Central registry for builtin + MCP tools
- Builtin tools: filesystem, git, shell
- MCP integration: External tool servers
- SimTool: Deterministic simulation tools

**Key Traits:**
- Tool: async execute() -> ToolResult
- ToolRegistry: register, unregister, execute, list

**State:**
- Tools are stateless (registry manages tool references)
- MCP server connections managed externally

---

### kelpie-vm

**Summary:** VM abstraction layer with Mock, Firecracker, Apple VZ backends; snapshot/restore with checksum verification

**Connects to:** kelpie-sandbox, kelpie-dst, kelpie-core

**Details:**

**VM Backends:**
- Mock: Testing with configurable failures
- Firecracker: Linux microVMs (feature-gated)
- Apple VZ: macOS virtualization (feature-gated)

**State Machine:**
Created → Starting → Running ⇄ Paused → Stopped → (restart) Created
Any → Crashed (terminal)

**Snapshot Guarantees:**
- CRC32 checksum verification
- Architecture validation (full snapshots require matching arch)
- 1 GiB max size
- Restore only from Created/Stopped states

**Invariants:**
- vcpu_count >= 1
- VM must be Running for exec()
- Checksum match for valid snapshot

**Error Recovery:**
- Retriable: BootTimeout, ExecTimeout, Crashed
- Requires recreate: Crashed, SnapshotCorrupted

**Issues (1):**
- [LOW] Snapshot checksum is CRC32 - weak for integrity, consider SHA256

---

### kelpie-wasm

**Summary:** WASM actor runtime - minimal implementation, single lib.rs file

**Connects to:** kelpie-runtime, kelpie-core

**Details:**

**Status:** Minimal/Stub
- Single lib.rs file
- WASM module loading and execution
- Actor trait implementation for WASM modules

**Note:** Less critical for TLA+ alignment as WASM execution is opaque from system perspective.

---

## Component Connections

```
kelpie-agent -> kelpie-server
kelpie-cli -> kelpie-server, kelpie-core
kelpie-cluster -> kelpie-registry, kelpie-runtime, kelpie-core
kelpie-core -> kelpie-runtime, kelpie-registry, kelpie-storage, kelpie-dst
kelpie-dst -> kelpie-runtime, kelpie-storage, kelpie-core
kelpie-memory -> kelpie-server, kelpie-storage, kelpie-core
kelpie-registry -> kelpie-runtime, kelpie-cluster, kelpie-core
kelpie-runtime -> kelpie-registry, kelpie-storage, kelpie-core
kelpie-sandbox -> kelpie-vm, kelpie-dst, kelpie-core
kelpie-server -> kelpie-runtime, kelpie-storage, kelpie-registry, kelpie-dst
kelpie-storage -> kelpie-server, kelpie-runtime, kelpie-core
kelpie-tools -> kelpie-server, kelpie-sandbox, kelpie-dst
kelpie-vm -> kelpie-sandbox, kelpie-dst, kelpie-core
kelpie-wasm -> kelpie-runtime, kelpie-core
```
