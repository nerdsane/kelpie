# Codebase Map

**Task:** Map Kelpie's aspirational features (ADRs/specs) vs actual implementation status, identify verification metrics
**Generated:** 2026-01-24T23:29:53.310965+00:00
**Components:** 11
**Issues Found:** 21

---

## Components Overview

### docs

**Summary:** High-level planning docs including VISION.md (status tracker), PLAN.md (duplicate), VERIFICATION.md (pipeline spec), and MAP.md

**Connects to:** docs/adr, kelpie-dst

**Details:**

VISION.md contains the definitive status tracker with phases 0-7:
- Phase 0 Bootstrap: COMPLETE
- Phase 1 Actor Runtime: COMPLETE  
- Phase 2 Memory Hierarchy: COMPLETE (gap: no vector search)
- Phase 3 Sandbox: PARTIAL (ProcessSandbox yes, Firecracker no)
- Phase 4 Tools: PARTIAL (Tool trait yes, MCP client stub)
- Phase 5 Cluster: PARTIAL (protocols yes, TCP transport no)
- Phase 6 Adapters: PARTIAL (REST API yes, Letta backend no)
- Phase 7 Production: NOT STARTED

VERIFICATION.md defines ADR→TLA+→DST→Code pipeline with current coverage table.

**Issues (2):**
- [HIGH] VISION.md claims performance targets (1M agents, <1ms invocation) that are unverified
- [MEDIUM] Duplicate content between VISION.md and docs/PLAN.md - maintenance burden

---

### docs/adr

**Summary:** 27 ADRs defining architectural commitments across actor model, storage, VM, cluster, agent, and tool integration

**Connects to:** docs, kelpie-core, kelpie-runtime, kelpie-dst, kelpie-vm, kelpie-cluster

**Details:**

ADR status breakdown:
- Accepted/Complete: 001, 002, 004, 005, 006, 009, 010, 012, 014, 016, 018, 020, 021, 022, 023, 024, 025, 026, 027
- Proposed: 003 (WASM), 011 (Agent Types)
- In Progress: 013 (Actor-Based Agent Server)
- Superseded: 017, 019 (by ADR-020)

Key verification artifacts referenced:
- 12 TLA+ specifications in docs/tla/
- DST test suite alignment documented
- Safety invariants: SingleActivation, Linearizability, Durability, PlacementConsistency
- Liveness properties: EventualRecovery, EventualCompletion

**Issues (2):**
- [MEDIUM] Many ADRs lack explicit acceptance criteria - they document what to build but not how to verify it's complete
- [MEDIUM] Status tracking inconsistent - some ADRs marked 'Accepted' despite incomplete implementation

---

### kelpie-agent

**Summary:** STUB - Only placeholder struct, no implementation

**Connects to:** kelpie-core, kelpie-server

**Details:**

Single file (lib.rs) with 458 bytes containing:
- Documentation comments describing intended agent abstractions
- All modules commented out: // pub mod agent; // pub mod memory; // pub mod orchestrator; // pub mod tool;
- Single placeholder: pub struct Agent;

Comment says "Phase 5 implementation" - not started.
NOTE: Agent functionality appears to live in kelpie-server instead!

**Issues (1):**
- [MEDIUM] kelpie-agent crate is empty stub - actual agent code is in kelpie-server

---

### kelpie-cluster

**Summary:** PARTIAL - Heartbeat and migration protocols implemented, but TCP transport incomplete and membership stubbed

**Connects to:** kelpie-core, kelpie-runtime, kelpie-registry

**Details:**

Working:
- Heartbeat protocol (broadcast, sequence tracking)
- 3-phase migration (Prepare/Transfer/Complete)
- MemoryTransport for testing
- Failure detection (detection only)
- Graceful shutdown/drain

Incomplete/Stub:
- TCP transport: Code truncated mid-implementation
- Cluster membership: join_cluster() is placeholder "TODO Phase 3"
- Auto-migration on failure: "TODO Phase 6"
- No partition tolerance strategy

Current verdict: Good for single-node testing, NOT ready for production multi-node.

**Issues (3):**
- [HIGH] TCP transport implementation truncated/incomplete
- [HIGH] Cluster membership is a stub - cannot form multi-node clusters
- [MEDIUM] Failure detection runs but doesn't execute migrations

---

### kelpie-core

**Summary:** Core types and traits are COMPLETE - ActorId, Actor trait, Error enum, Runtime abstraction, I/O providers, KV interfaces all implemented

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-dst

**Details:**

Fully implemented:
- ActorId with validation
- Actor trait with lifecycle hooks (on_activate, on_deactivate, invoke)
- ActorContext for runtime access
- Error enum (40+ variants)
- Runtime abstraction (tokio + madsim)
- TimeProvider + RngProvider traits with implementations
- ContextKV trait with BufferingContextKV, ArcContextKV
- TeleportPackage and VmSnapshotBlob for snapshots
- KelpieConfig validation
- OpenTelemetry integration

Incomplete:
- TeleportStorage trait has no implementations
- Cross-actor invocation routing marked "future phase"

**Issues (1):**
- [LOW] TeleportStorage trait defined but no backends implemented

---

### kelpie-dst

**Summary:** COMPLETE - Deterministic simulation testing with SimClock, DeterministicRng, FaultInjector, 20+ fault types, liveness verification

**Connects to:** kelpie-core, kelpie-runtime, kelpie-storage, kelpie-server

**Details:**

Core components:
- SimClock: Explicit time advancement (no wall clock dependency)
- DeterministicRng: ChaCha20-based, forkable, same seed = same output
- FaultInjector: Probabilistic injection with ~20 fault types
- BoundedLiveness: Temporal property verification (eventually, leads-to)
- InvariantChecker: System invariant validation
- SimVm, SimLlmClient, SimSandboxIO: Mock implementations

Test count: 113+ in lib, 200+ across all DST test files

DST_SEED environment variable ensures reproducibility.

**Issues (2):**
- [MEDIUM] SimNetwork and SimStorage referenced in lib.rs but not shown in analyzed code
- [LOW] No clock-wait integration - can deadlock if never advanced

---

### kelpie-runtime

**Summary:** COMPLETE - Dispatcher with single-activation enforcement, actor lifecycle with state persistence, transactional KV operations

**Connects to:** kelpie-core, kelpie-storage, kelpie-registry

**Details:**

Fully implemented:
- Single-activation: Local mode via HashMap, distributed via Registry claims
- Actor lifecycle: on_activate/on_deactivate hooks, state load/save
- State persistence: Transactional atomic state+KV commits with rollback on error
- Mailbox: Bounded queue with capacity limits
- Registry integration: try_claim_actor, get_placement, unregister_actor
- Backpressure: max_pending_per_actor with atomic counters
- Metrics: record_invocation, record_agent_activated

27 unit tests passing.

**Issues (2):**
- [LOW] No timeout on registry operations - could hang indefinitely
- [LOW] Pending counter HashMap never cleaned up - minor memory growth

---

### kelpie-server

**Summary:** PARTIAL - REST API models complete, LLM integration working, but MCP is stub and archival search unimplemented

**Connects to:** kelpie-core, kelpie-runtime, kelpie-storage, kelpie-dst

**Details:**

Working:
- REST API models (Letta-compatible with serde aliases)
- LLM integration: OpenAI + Anthropic with streaming
- Tool definitions with JSON schema
- Agent/Block/Message storage (in-memory HashMap)
- Job scheduling (cron/interval/once)
- SSE streaming parser
- AgentActor + AgentService setup code exists

Phase 5/6 Migration in Progress:
- Agent_service and dispatcher instantiated
- But legacy HashMap storage still used for HTTP handlers
- TODO comments indicate migration incomplete

Not Implemented:
- MCP client (data models only, no execution)
- Archival memory search (no vector embeddings)
- Message windowing for context management
- Tool execution pipeline (registration exists, executor unclear)

**Issues (3):**
- [HIGH] MCP integration is stub only - data models exist but no client implementation
- [HIGH] Archival memory search not operational - no vector embeddings or retrieval
- [MEDIUM] Phase 5/6 actor migration incomplete - still using legacy HashMap storage

---

### kelpie-storage

**Summary:** COMPLETE - Memory and FDB backends with linearizable transactions, WAL implementation

**Connects to:** kelpie-core, kelpie-runtime

**Details:**

Fully implemented:
- MemoryKV: In-memory HashMap, good for testing/DST
- FdbKV: Production FoundationDB backend with tuple layer encoding
- Transactions: Both backends support ACID with read-your-writes
- FDB auto-retry on conflicts (up to 5 attempts)
- WAL: MemoryWal and KvWal with idempotency support
- ScopedKV: Actor-scoped wrapper for isolation

Note: FDB tests marked #[ignore] - require running FDB cluster

**Issues (2):**
- [LOW] Transaction module appears unused - dead code
- [LOW] Size validation uses debug asserts only - not enforced in release builds

---

### kelpie-vm

**Summary:** COMPLETE - Mock, Apple VZ, and Firecracker backends all implemented with snapshot/restore

**Connects to:** kelpie-core, kelpie-sandbox

**Details:**

Backends:
- MockVm: Testing/CI - simulated lifecycle and commands
- Apple VZ: Production macOS - FFI to Virtualization.framework, vsock exec
- Firecracker: Production Linux - wraps kelpie-sandbox

All backends support:
- Full lifecycle (Created→Starting→Running→Paused→Stopped)
- Snapshot/restore with CRC32 validation
- Architecture compatibility checking
- Config validation with explicit error types

36 unit tests passing.

**Issues (2):**
- [LOW] No disk existence validation at config time - errors only at VM start
- [LOW] Snapshot file cleanup is best-effort - orphaned files possible

---

### kelpie-wasm

**Summary:** STUB - Only placeholder struct, no implementation

**Connects to:** kelpie-core

**Details:**

Single file (lib.rs) with 413 bytes containing:
- Documentation comments describing intended wasmtime/waPC integration
- All modules commented out: // pub mod module; // pub mod runtime; // pub mod wapc;
- Single placeholder: pub struct WasmRuntime;

Comment says "Phase 4 implementation" - not started.

**Issues (1):**
- [HIGH] WASM actor runtime is a stub with no implementation

---

## Component Connections

```
docs -> docs/adr, kelpie-dst
docs/adr -> docs, kelpie-core, kelpie-runtime, kelpie-dst, kelpie-vm, kelpie-cluster
kelpie-agent -> kelpie-core, kelpie-server
kelpie-cluster -> kelpie-core, kelpie-runtime, kelpie-registry
kelpie-core -> kelpie-runtime, kelpie-storage, kelpie-dst
kelpie-dst -> kelpie-core, kelpie-runtime, kelpie-storage, kelpie-server
kelpie-runtime -> kelpie-core, kelpie-storage, kelpie-registry
kelpie-server -> kelpie-core, kelpie-runtime, kelpie-storage, kelpie-dst
kelpie-storage -> kelpie-core, kelpie-runtime
kelpie-vm -> kelpie-core, kelpie-sandbox
kelpie-wasm -> kelpie-core
```
