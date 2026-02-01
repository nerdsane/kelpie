# Codebase Map

**Task:** Understand what Kelpie currently can do and what is properly working, then map the path to formal verification
**Generated:** 2026-01-23T01:04:01.007566+00:00
**Components:** 10
**Issues Found:** 17

---

## Components Overview

### kelpie-agent

**Summary:** AI agent abstractions - STUB/placeholder only, P5 priority

**Connects to:** kelpie-runtime, kelpie-server

**Details:**

**Status: STUB (0 tests)**

Only contains a placeholder struct `Agent`. Modules for actual implementation are commented out:
- agent.rs (not implemented)
- memory.rs (not implemented)
- orchestrator.rs (not implemented)
- tool.rs (not implemented)

**Planned features:**
- Agent actor base class
- Agent memory/context management
- Tool invocation framework
- Multi-agent orchestration

**Note:** This is P5 priority. Agent functionality is actually implemented in kelpie-server (AgentActor, agent types, tools), not here. This crate appears to be for future higher-level abstractions.

**Issues (1):**
- [LOW] kelpie-agent is stub-only - agent implementation lives in kelpie-server

---

### kelpie-cluster

**Summary:** Cluster coordination with migration, RPC, and config - for distributed actor management

**Connects to:** kelpie-registry, kelpie-runtime, kelpie-storage

**Details:**

**Status: Implementation exists but needs verification**

**Modules:**
- cluster.rs - Cluster coordination logic
- config.rs - Cluster configuration
- error.rs - Cluster error types
- migration.rs - Actor migration between nodes
- rpc.rs - Inter-node RPC communication

**Note:** This component enables distributed features (single-activation across nodes, actor migration). Currently the runtime only has single-node support - cluster coordination would enable true distributed deployment.

**Issues (2):**
- [HIGH] Cluster coordination not integrated with runtime - distributed single-activation not enforced
- [MEDIUM] No tests found for kelpie-cluster in test index

---

### kelpie-core

**Summary:** Core types and abstractions - Actor, ActorId, ActorRef, Config, Error, I/O context, Teleport types

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-dst

**Details:**

**WORKING (27 tests pass):**
- ActorId with namespace validation (length, character checks)
- ActorRef for location-transparent actor references
- BufferingContextKV with read-your-writes semantics
- Error types with retriable classification
- IoContext abstraction for time/RNG providers
- TeleportPackage and VmSnapshotBlob encode/decode with checksums
- Runtime abstraction (tokio + madsim support)
- Telemetry configuration

**Traits defined (implemented elsewhere):**
- Actor trait (invoke, on_activate, on_deactivate)
- ContextKV trait (get, set, delete, exists, list_keys)
- TimeProvider, RngProvider traits
- TeleportStorage trait

---

### kelpie-dst

**Summary:** Deterministic Simulation Testing framework with 70 tests - clock, RNG, storage, network, faults, sandbox simulation

**Connects to:** kelpie-core, kelpie-storage, kelpie-runtime, kelpie-vm

**Details:**

**WORKING (70 tests pass):**
- SimClock - deterministic time control, async sleep primitives
- DeterministicRng - seeded RNG with fork support, reproducible
- SimStorage - in-memory KV with fault injection, transactions
- SimNetwork - message queuing, latency, partitions, reordering
- FaultInjector - 40+ fault types across storage/crash/network/time/resource/MCP/LLM/sandbox/snapshot/teleport
- SimSandbox - lifecycle, exec, snapshot/restore simulation
- SimLlmClient - deterministic LLM mocking with canned responses
- SimTeleportStorage - teleport package simulation
- Simulation harness - environment builder, determinism verification

**Fault types supported:**
- Storage: ReadFail, WriteFail, Corruption, Latency, DiskFull
- Crash: BeforeWrite, AfterWrite, DuringTransaction
- Network: Partition, Delay, PacketLoss, MessageReorder
- Time: ClockSkew, ClockJump
- Sandbox: BootFail, ExecFail, SnapshotFail, etc.

**DST Quality:**
- True determinism via DST_SEED reproducibility
- Explicit time control (no wall clock dependency)
- Comprehensive fault coverage

**Issues (2):**
- [LOW] SimTeleportStorage ignores DeterministicRng parameter (_rng unused)
- [LOW] Max steps/time limits defined but not enforced in simulation

---

### kelpie-registry

**Summary:** Actor registry with node management, placement strategies, heartbeat tracking - 43 tests pass

**Connects to:** kelpie-runtime, kelpie-cluster, kelpie-storage

**Details:**

**WORKING (43 tests pass):**
- NodeId with validation (alphanumeric, max 128 bytes)
- NodeInfo with heartbeat tracking, capacity management
- NodeStatus state machine (Joining→Active→Suspect/Leaving→Failed/Left)
- ActorPlacement with generation versioning, migration support
- PlacementStrategy enum (LeastLoaded, Random, Affinity, RoundRobin)
- PlacementContext builder pattern
- MemoryRegistry with RwLock-based state
- HeartbeatTracker with timeout detection
- Registry trait with CAS semantics for actor claim

**Features:**
- Node registration, unregistration, listing
- Actor registration, migration, conflict detection
- Heartbeat-based failure detection
- Load-based node selection

**Limitations:**
- In-memory only (no persistence)
- Single-node (no distributed coordination)
- RoundRobin falls back to LeastLoaded

**Issues (3):**
- [MEDIUM] PlacementStrategy defined but actual algorithms not implemented
- [MEDIUM] No actual network heartbeat sending - only tracking
- [LOW] Failed nodes stay tracked forever - memory leak risk

---

### kelpie-runtime

**Summary:** Actor runtime with dispatcher, activation lifecycle, mailbox, state persistence

**Connects to:** kelpie-core, kelpie-storage

**Details:**

**WORKING (23 tests pass):**
- RuntimeBuilder with fluent config (factory, KV store, tokio runtime)
- Dispatcher message routing and actor management
- Actor activation/deactivation lifecycle with state guards
- Mailbox with bounded FIFO queue and capacity limits
- ActorHandle for async invocation with timeouts
- State persistence via KV store (JSON serialization)
- Transactional KV - atomic state + KV persistence
- Multiple independent actors with separate state

**Key features:**
- Single-threaded execution per actor (mailbox queuing)
- Timeout enforcement on invocations
- Graceful error handling with rollback on failure
- Idle timeout detection

**Issues (2):**
- [MEDIUM] No distributed lock for single-activation guarantee - only works single-node
- [LOW] No actor cleanup policy - actors stay in HashMap indefinitely

---

### kelpie-server

**Summary:** Letta-compatible agent server with REST API, LLM integration, tools, actor-based architecture - extensive DST coverage

**Connects to:** kelpie-runtime, kelpie-storage, kelpie-dst, kelpie-vm

**Details:**

**WORKING (based on test index - 70+ DST tests):**
- REST API: agents, messages, blocks, tools, archival, teleport
- Agent types: MemGPT, LettaV1, React with capability-based tool filtering
- LLM integration via trait abstraction (Claude/OpenAI compatible)
- Memory tools: core_memory_append, archival_memory_insert/search
- Heartbeat/pause mechanism for agent control
- Code execution tool support (Python, JavaScript, TypeScript, R)
- Session storage with FDB backend
- Teleport service for agent migration
- SSE streaming for responses

**DST Test Coverage:**
- heartbeat_dst.rs: 18 tests (pause, clock skew, determinism)
- heartbeat_integration_dst.rs: 8 tests (fault injection)
- agent_types_dst.rs: 15 tests (capabilities, tool filtering)
- agent_loop_types_dst.rs: 11 tests (loop behavior under faults)
- heartbeat_real_dst.rs: 13 tests (real registry integration)

**Architecture:**
- Actor-based via AgentActor wrapping agent state
- AgentService layer between REST and Dispatcher
- FDB for hot-path, UMI for search (dual storage)
- SimLlmClient for deterministic testing

**Issues (2):**
- [MEDIUM] Requires external LLM API key for production - not testable without mock
- [LOW] FDB storage tests require external cluster

---

### kelpie-storage

**Summary:** Actor KV storage with MemoryKV (in-memory) and FdbKV (FoundationDB) backends, transaction support

**Connects to:** kelpie-core, kelpie-runtime, kelpie-server

**Details:**

**WORKING (9 tests pass, 8 ignored for FDB):**
- MemoryKV - in-memory KV store for testing/dev
- Transaction API - commit, abort, read-your-writes isolation
- Key encoding with ordering preservation
- Subspace isolation per actor
- ActorKV trait abstraction

**FDB Backend (requires running cluster):**
- FdbKV connects to FoundationDB cluster
- Tuple-encoded keys for efficient range scans
- Transaction atomicity with FDB guarantees
- 8 tests ignored (require FDB cluster)

**Key features:**
- Actor-scoped key prefixing for isolation
- Transaction buffer with atomic commit
- CRUD operations (get, set, delete, exists, list_keys)

**Issues (2):**
- [MEDIUM] FDB tests require external cluster - not run in CI
- [LOW] No WAL/journaling for crash recovery in MemoryKV

---

### kelpie-vm

**Summary:** VM abstraction layer with MockVm, Apple Vz, and Firecracker backends for sandbox execution

**Connects to:** kelpie-core, kelpie-dst, kelpie-sandbox

**Details:**

**WORKING (36 tests pass):**
- VmInstance trait - lifecycle (start, stop, pause, resume), exec, snapshot/restore
- MockVm - test implementation with simulated commands
- VmSnapshot with CRC32 checksums and architecture compatibility
- VirtioFS mount configuration
- VmConfig builder pattern with resource limits

**Backends:**
- MockVm (working) - for testing without hypervisor
- Apple Vz (macOS, feature-gated) - Virtualization.framework via C FFI bridge
- Firecracker (Linux, feature-gated) - wraps kelpie-sandbox::FirecrackerSandbox

**Resource limits defined:**
- vCPU: 32 max
- RAM: 128-16384 MiB
- Snapshot: 1 GiB max
- Mounts: 16 max

**Issues (2):**
- [LOW] MockVm command execution is hardcoded (only ~6 commands)
- [LOW] Snapshot cleanup ignores errors silently in Firecracker backend

---

### kelpie-wasm

**Summary:** WASM actor runtime - STUB/placeholder only, P3 priority

**Connects to:** kelpie-runtime

**Details:**

**Status: STUB (0 tests)**

Only contains a placeholder struct `WasmRuntime`. Modules for actual implementation are commented out:
- module.rs (not implemented)
- runtime.rs (not implemented)  
- wapc.rs (not implemented)

**Planned features (per ADR-003):**
- wasmtime integration
- waPC protocol for polyglot actors
- WASM module loading/caching
- Cross-language actor invocation

**Note:** This is P3 priority per ADR-003. The scaffolding exists but no implementation.

**Issues (1):**
- [LOW] WASM runtime is stub-only - no actual implementation

---

## Component Connections

```
kelpie-agent -> kelpie-runtime, kelpie-server
kelpie-cluster -> kelpie-registry, kelpie-runtime, kelpie-storage
kelpie-core -> kelpie-runtime, kelpie-storage, kelpie-dst
kelpie-dst -> kelpie-core, kelpie-storage, kelpie-runtime, kelpie-vm
kelpie-registry -> kelpie-runtime, kelpie-cluster, kelpie-storage
kelpie-runtime -> kelpie-core, kelpie-storage
kelpie-server -> kelpie-runtime, kelpie-storage, kelpie-dst, kelpie-vm
kelpie-storage -> kelpie-core, kelpie-runtime, kelpie-server
kelpie-vm -> kelpie-core, kelpie-dst, kelpie-sandbox
kelpie-wasm -> kelpie-runtime
```
