# Issues Found During Examination

**Task:** Map Kelpie's aspirational features (ADRs/specs) vs actual implementation status, identify verification metrics
**Generated:** 2026-01-24T23:29:53.311292+00:00
**Total Issues:** 21

---

## HIGH (6)

### [docs] VISION.md claims performance targets (1M agents, <1ms invocation) that are unverified

**Evidence:** VISION.md line 259-266: All metrics marked 'Unverified'

*Found: 2026-01-24T23:24:11.238923+00:00*

---

### [kelpie-cluster] TCP transport implementation truncated/incomplete

**Evidence:** rpc.rs code truncated mid-implementation, reader task missing

*Found: 2026-01-24T23:28:02.484943+00:00*

---

### [kelpie-cluster] Cluster membership is a stub - cannot form multi-node clusters

**Evidence:** join_cluster() has TODO comment: 'Phase 3: Use FDB for membership'

*Found: 2026-01-24T23:28:02.484945+00:00*

---

### [kelpie-wasm] WASM actor runtime is a stub with no implementation

**Evidence:** lib.rs has only placeholder struct WasmRuntime and commented-out modules

*Found: 2026-01-24T23:28:24.027809+00:00*

---

### [kelpie-server] MCP integration is stub only - data models exist but no client implementation

**Evidence:** mcp_servers HashMap stored but never used, no discovery/execution code

*Found: 2026-01-24T23:29:46.924938+00:00*

---

### [kelpie-server] Archival memory search not operational - no vector embeddings or retrieval

**Evidence:** ArchivalEntry model exists but no search implementation

*Found: 2026-01-24T23:29:46.924940+00:00*

---

## MEDIUM (7)

### [docs/adr] Many ADRs lack explicit acceptance criteria - they document what to build but not how to verify it's complete

**Evidence:** ADRs 003, 015, 016, 017, 018, 019, 026 have no explicit test requirements

*Found: 2026-01-24T23:24:11.114435+00:00*

---

### [docs/adr] Status tracking inconsistent - some ADRs marked 'Accepted' despite incomplete implementation

**Evidence:** ADR-026 MCP integration marked 'Accepted' but all components marked 'Designed' not implemented

*Found: 2026-01-24T23:24:11.114437+00:00*

---

### [docs] Duplicate content between VISION.md and docs/PLAN.md - maintenance burden

**Evidence:** Both files contain identical phase status tracking

*Found: 2026-01-24T23:24:11.238925+00:00*

---

### [kelpie-dst] SimNetwork and SimStorage referenced in lib.rs but not shown in analyzed code

**Evidence:** pub use statements exist but implementations may be incomplete

*Found: 2026-01-24T23:28:02.333506+00:00*

---

### [kelpie-cluster] Failure detection runs but doesn't execute migrations

**Evidence:** TODO Phase 6 comment - detection only, no recovery

*Found: 2026-01-24T23:28:02.484946+00:00*

---

### [kelpie-agent] kelpie-agent crate is empty stub - actual agent code is in kelpie-server

**Evidence:** lib.rs has only placeholder struct Agent; server has 46 files with agent implementations

*Found: 2026-01-24T23:28:24.235035+00:00*

---

### [kelpie-server] Phase 5/6 actor migration incomplete - still using legacy HashMap storage

**Evidence:** TODO comments: 'Remove after HTTP handlers migrated to agent_service'

*Found: 2026-01-24T23:29:46.924941+00:00*

---

## LOW (8)

### [kelpie-core] TeleportStorage trait defined but no backends implemented

**Evidence:** teleport.rs defines trait but no S3/filesystem/sim implementations

*Found: 2026-01-24T23:24:45.098340+00:00*

---

### [kelpie-runtime] No timeout on registry operations - could hang indefinitely

**Evidence:** dispatcher.rs:620 - registry.try_claim_actor().await not wrapped in timeout

*Found: 2026-01-24T23:26:05.446588+00:00*

---

### [kelpie-runtime] Pending counter HashMap never cleaned up - minor memory growth

**Evidence:** dispatcher.rs:284-293 - entry never removed after actor deactivates

*Found: 2026-01-24T23:26:05.446590+00:00*

---

### [kelpie-storage] Transaction module appears unused - dead code

**Evidence:** transaction.rs defines Transaction/TransactionOp but backends use own implementations

*Found: 2026-01-24T23:26:05.572693+00:00*

---

### [kelpie-storage] Size validation uses debug asserts only - not enforced in release builds

**Evidence:** encode_key(), set() use assert! not Result::Err

*Found: 2026-01-24T23:26:05.572694+00:00*

---

### [kelpie-dst] No clock-wait integration - can deadlock if never advanced

**Evidence:** SimClock::sleep waits on notify but requires external advance

*Found: 2026-01-24T23:28:02.333508+00:00*

---

### [kelpie-vm] No disk existence validation at config time - errors only at VM start

**Evidence:** RootDiskNotAccessible error defined but only Firecracker validates

*Found: 2026-01-24T23:28:02.632700+00:00*

---

### [kelpie-vm] Snapshot file cleanup is best-effort - orphaned files possible

**Evidence:** let _ = ... pattern in cleanup methods

*Found: 2026-01-24T23:28:02.632702+00:00*

---
