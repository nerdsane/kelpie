# Task: Foundation to Formal Verification Pipeline

**Created:** 2026-01-22 14:30:00
**State:** PLANNING

---

## Vision Alignment

**Vision files read:** CONSTRAINTS.md, CLAUDE.md

**Relevant constraints/guidance:**
- Simulation-first development (CONSTRAINTS.md §1) - DST is primary verification method
- TigerStyle safety principles (CONSTRAINTS.md §3) - Assertions, explicit constants, no silent failures
- No placeholders in production (CONSTRAINTS.md §4) - Must verify before claiming complete
- Changes are traceable (CONSTRAINTS.md §7) - ADRs document architectural decisions

---

## Task Description

### Problem

The Kelpie codebase has infrastructure for advanced verification (DST framework, ADRs), but the current state is uncertain:
- **DST coverage is questionable** - Do we have tests for critical paths? Are the tests real DST or pseudo-DST?
- **ADRs are questionable** - Are they up to date? Do they match the implementation?
- **Code quality is uncertain** - TigerStyle compliance? Dead code? Stubs?

Without a solid foundation, we cannot build the advanced verification pipeline:
- We can't generate TLA+ specs from ADRs if ADRs are stale
- We can't build a verification pyramid if DST coverage has gaps
- We can't trust the system if the code itself is questionable

### Solution

Build from foundation to formal verification in two major stages:

**Stage 1: Foundation Cleanup (Phases 1-4)**
- Audit and fix DST coverage
- Audit and update ADRs
- Audit and fix code quality
- Achieve a known-good baseline

**Stage 2: Formal Verification Infrastructure (Phases 5-8)**
- Create TLA+ specifications from validated ADRs
- Build verification pyramid (DST → Stateright → Kani)
- Add production telemetry integration
- Create ADR → TLA+ → DST pipeline for ongoing development

### Why This Order

```
                    ┌─────────────────────────────────────────┐
                    │  Stage 2: Formal Verification          │
                    │                                        │
                    │  Phase 8: ADR→TLA+→DST Pipeline        │
                    │  Phase 7: Production Telemetry         │
                    │  Phase 6: Verification Pyramid         │
                    │  Phase 5: TLA+ Specifications          │
                    └────────────────┬────────────────────────┘
                                     │ DEPENDS ON
                    ┌────────────────▼────────────────────────┐
                    │  Stage 1: Foundation Cleanup           │
                    │                                        │
                    │  Phase 4: Foundation Fixes             │
                    │  Phase 3: Code Quality Audit           │
                    │  Phase 2: ADR Audit                    │
                    │  Phase 1: DST Coverage Audit           │
                    └─────────────────────────────────────────┘
```

---

## Options & Decisions [REQUIRED]

### Decision 1: Audit Before Fix vs Fix As We Go

**Context:** Should we audit everything first, then fix, or fix issues as we find them?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Audit-First | Complete all audits (DST, ADR, Code), then fix in Phase 4 | Full picture before changes; can prioritize | Longer before any fixes; might forget context |
| B: Fix-As-We-Go | Fix issues immediately when found during audit | Immediate progress; fresh context | Might miss bigger patterns; harder to track |
| C: Hybrid | Audit creates issues, fix critical ones immediately, batch the rest | Best of both; critical issues fixed fast | More complex tracking |

**Decision:** C (Hybrid) - Audit creates a full inventory of issues, critical/blocking issues are fixed immediately, non-critical issues are batched for Phase 4.

**Trade-offs accepted:**
- More complex issue tracking (use VFS tools)
- Some context may be lost between audit and batch fix
- Acceptable because: critical issues get immediate attention, pattern recognition from full audit

---

### Decision 2: What Constitutes "Good Enough" Foundation

**Context:** When is Stage 1 complete? What's the bar for moving to Stage 2?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Perfection | 100% DST coverage, all ADRs current, zero code issues | Highest confidence | May never finish; diminishing returns |
| B: Critical Paths | Critical paths have DST, core ADRs current, no blocking issues | Pragmatic; enables progress | Some gaps remain |
| C: Metrics-Based | Define specific thresholds (e.g., 80% DST, 90% ADR accuracy) | Objective; measurable | Numbers may not reflect quality |

**Decision:** B (Critical Paths) - Focus on critical paths and core components. Stage 2 can proceed when:
- All critical paths (actor lifecycle, storage, migration) have verified DST coverage
- Core ADRs (001-005) are current and accurate
- No blocking code quality issues (stubs in production paths, fake DST)

**Trade-offs accepted:**
- Non-critical paths may have gaps
- Some ADRs may be deprioritized
- Acceptable because: Stage 2 builds on critical paths, not edge cases

---

### Decision 3: TLA+ Scope

**Context:** Which components get TLA+ specifications in Phase 5?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Everything | TLA+ for all crates | Complete formal coverage | Massive effort; diminishing returns |
| B: Distributed Core | Actor lifecycle, storage, cluster coordination | High-value targets; where bugs hide | Doesn't cover all code |
| C: Just Actor Lifecycle | Start with single-actor invariants | Fastest to deliver; proof of concept | Limited coverage |

**Decision:** B (Distributed Core) - TLA+ specs for:
1. Actor Lifecycle (single-actor activation, deactivation, invocation)
2. Actor Storage (KV operations, consistency)
3. Actor Migration/Teleport (snapshot, restore, state transfer)
4. Cluster Coordination (registry, placement) - if time permits

**Trade-offs accepted:**
- WASM runtime, CLI tools don't get TLA+ specs
- Agent abstraction layer doesn't get formal specs
- Acceptable because: formal verification most valuable for distributed correctness

---

## Quick Decision Log [REQUIRED]

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-22 | Hybrid audit approach | Balance thoroughness with progress | Tracking complexity |
| 2026-01-22 | Critical paths bar for Stage 2 | Pragmatic path to formal verification | Non-critical gaps |
| 2026-01-22 | TLA+ for distributed core only | Highest value targets | Limited scope |
| 2026-01-23 | Add Phase 1.5 for invariants | Test quality audit revealed no invariant verification; invariants serve dual purpose (test quality + TLA+ input) | Additional phase before fixes |
| 2026-01-23 | Invariants in Rust, not just docs | Code-level invariants can be used by test helpers; documentation alone isn't enforceable | Implementation effort |
| 2026-01-23 | 6 core invariants first | Cover critical paths identified in examination; can add more later | Limited initial scope |

---

## Current State Assessment

### Infrastructure We Have

| Component | Location | Status |
|-----------|----------|--------|
| **Python MCP Server** | `tools/mcp-kelpie-python/` | ✅ 31 tools, 89 tests |
| **tree-sitter Indexer** | `mcp_kelpie/indexer/` | ✅ Structural indexes |
| **RLM Environment** | `mcp_kelpie/rlm/` | ✅ REPL + sub-LLM |
| **AgentFS (Turso)** | `agentfs-sdk` | ✅ Persistent state |
| **DST Framework** | `crates/kelpie-dst/` | ⚠️ Exists, coverage unknown |
| **ADRs** | `docs/adr/` | ⚠️ Exist, accuracy unknown |

### MCP Tools Available for This Work

**For Auditing:**
- `repl_load` + `repl_sub_llm` - Load code, have sub-LLM analyze
- `repl_map_reduce` - Parallel analysis across partitions
- `index_*` - Query structural indexes

**For Tracking:**
- `vfs_fact_add/check` - Track verified facts with evidence
- `vfs_invariant_verify/status` - Track invariant verification
- `vfs_exploration_log` - Audit trail of exploration

**For Verification:**
- Claude's `Bash` tool - Run `cargo test`, `cargo clippy`, etc.
- Claude's `Read/Grep/Glob` - Examine code directly

---

## Implementation Plan

### Stage 1: Foundation Cleanup

#### Phase 1: DST Coverage Audit

**Goal:** Understand current DST coverage - what's tested, what's missing, what's fake.

**Approach:** Use RLM to load and analyze all DST tests.

```bash
# Load all DST-related files
repl_load(pattern="crates/kelpie-dst/**/*.rs", var_name="dst_framework")
repl_load(pattern="**/*_dst.rs", var_name="dst_tests")
repl_load(pattern="**/*_chaos.rs", var_name="chaos_tests")
```

- [ ] **1.1: Inventory DST Framework**
  - Load `kelpie-dst` crate
  - Document available components: SimClock, SimStorage, SimNetwork, FaultInjector, etc.
  - Identify supported fault types
  - Record in `vfs_fact_add`

- [ ] **1.2: Inventory DST Tests**
  - Find all `*_dst.rs` files
  - For each: What component? What scenarios? What faults?
  - Identify "fake DST" (tests that claim determinism but aren't)
  - Record findings

- [ ] **1.3: Identify Coverage Gaps**
  - Map DST tests to critical paths:
    - [ ] Actor activation/deactivation
    - [ ] Actor invocation
    - [ ] State persistence (KV operations)
    - [ ] State recovery (crash/restart)
    - [ ] Migration/teleport
    - [ ] Cluster coordination
  - Document gaps

- [x] **1.4: Assess Test Quality** ✅ COMPLETE (2026-01-23)
  - Do tests use `DST_SEED` for reproducibility? **Yes, framework supports it**
  - Do tests inject faults? **Yes, 40+ fault types**
  - Do tests verify invariants (not just "doesn't crash")? **NO - CRITICAL GAP**
  - Rate each test: Real DST / Partial DST / Fake DST **See findings below**

**Findings from Phase 1.4 (Critical Review 2026-01-23):**

| Finding | Severity | Details |
|---------|----------|---------|
| Tests accept success+failure | HIGH | `match { Ok => ..., Err => continue }` pattern means tests "pass" regardless of behavior |
| ~68% smoke tests | MEDIUM | Most tests only verify `response.status() == 200`, not data correctness |
| No invariant verification | HIGH | Tests don't verify "if create succeeded, get must work" type invariants |
| No fault-type distinction | MEDIUM | Can't tell if failure was expected (fault injected) vs bug |

**Root Cause:** Tests verify "doesn't crash" not "behaves correctly under faults"

**Deliverable:** `.kelpie-index/audits/dst_coverage_audit.md`

---

#### Phase 1.5: Define System Invariants [NEW]

**Goal:** Define invariants that tests MUST verify. These become the foundation for TLA+ specs in Phase 5.

**Why This Phase:**
- Phase 1.4 found tests don't verify invariants
- Examination (MAP.md/ISSUES.md) found 17 component issues
- Connection: Tests didn't catch issues because no invariants defined
- Invariants serve dual purpose: (1) fix test quality, (2) input to TLA+ specs

**Approach:** Create invariant definitions and test helpers.

- [ ] **1.5.1: Create Invariant Definitions**
  - Create `crates/kelpie-server/src/invariants.rs`
  - Define invariants as constants with documentation
  - Map each invariant to examination issues it would catch

- [ ] **1.5.2: Define Core Invariants**

  | Invariant | Description | Would Catch (from ISSUES.md) |
  |-----------|-------------|------------------------------|
  | `SINGLE_ACTIVATION` | At most one active instance per ActorId | [HIGH] Distributed single-activation not enforced |
  | `CREATE_GET_CONSISTENCY` | If create returns Ok(agent), get(agent.id) must succeed | BUG-001 pattern (fixed) |
  | `DELETE_GET_CONSISTENCY` | If delete returns Ok, get must return NotFound | Delete atomicity issues |
  | `MESSAGE_DURABILITY` | Committed messages survive crashes | Message loss under faults |
  | `TRANSACTION_ATOMICITY` | Partial writes never visible | [MEDIUM] Range scans not transactional |
  | `BLOCK_ATOMICITY` | Memory block updates are atomic | Partial block updates |

- [ ] **1.5.3: Create Test Helpers**
  - Create `crates/kelpie-server/tests/common/invariants.rs`
  - `InvariantTracker` - tracks state for verification
  - `assert_create_get_consistent()` - verify after create
  - `assert_delete_get_consistent()` - verify after delete
  - `verify_all_invariants()` - comprehensive check

- [ ] **1.5.4: Create Fault-Aware Macros**
  - `assert_fails_under_fault!` - expected failure when fault active
  - `assert_no_corruption!` - operation may fail but no state corruption
  - Distinguish expected transient failures from bugs

- [ ] **1.5.5: Document Invariant-to-TLA+ Mapping**
  - Create `.kelpie-index/specs/invariant-tla-mapping.md`
  - Map each Rust invariant to future TLA+ invariant
  - This bridges Phase 1.5 → Phase 5

**Deliverable:**
- `crates/kelpie-server/src/invariants.rs`
- `crates/kelpie-server/tests/common/invariants.rs`
- `.kelpie-index/specs/invariant-tla-mapping.md`

**Exit Criteria:**
- [ ] All 6 core invariants defined with documentation
- [ ] Test helpers compile and have unit tests
- [ ] Mapping document connects to Phase 5 TLA+ specs

---

#### Phase 2: ADR Audit

**Goal:** Verify ADRs are current, accurate, and match implementation.

**Approach:** Read each ADR, compare to current code, identify drift.

- [ ] **2.1: Inventory ADRs**
  - List all ADRs in `docs/adr/`
  - For each: title, date, status (proposed/accepted/superseded)

- [ ] **2.2: ADR 001 - Virtual Actor Model**
  - Read ADR
  - Verify claims against `kelpie-runtime/`
  - Document drift or confirmation
  - Mark as: Current / Needs Update / Superseded

- [ ] **2.3: ADR 002 - FoundationDB Integration**
  - Read ADR
  - Verify against `kelpie-storage/`
  - Note: Is FDB still the plan? Or has it changed?

- [ ] **2.4: ADR 003 - WASM Actor Runtime**
  - Read ADR
  - Verify against `kelpie-wasm/`

- [ ] **2.5: ADR 004 - Linearizability Guarantees**
  - Read ADR
  - This is critical for TLA+ specs
  - Verify consistency model claims

- [ ] **2.6: ADR 005 - DST Framework**
  - Read ADR
  - Compare to Phase 1 findings
  - Does the ADR match reality?

- [ ] **2.7: Identify Missing ADRs**
  - What architectural decisions are NOT documented?
  - Cluster coordination?
  - Agent abstraction?
  - Teleport/migration?

**Deliverable:** `.kelpie-index/audits/adr_audit.md`

---

#### Phase 3: Code Quality Audit

**Goal:** Assess TigerStyle compliance, dead code, stubs, and overall quality.

**Approach:** Combination of automated checks and RLM analysis.

- [ ] **3.1: TigerStyle Compliance**
  - Check for explicit constants with units (e.g., `TIMEOUT_MS_MAX`)
  - Check for assertions in functions (target: 2+ per non-trivial function)
  - Check for big-endian naming
  - Check for `u64` vs `usize` for sizes

- [ ] **3.2: Dead Code Detection**
  ```bash
  cargo clippy --workspace -- -W dead_code 2>&1 | grep "dead_code"
  ```
  - Document unused functions, types, modules

- [ ] **3.3: Stub Detection**
  ```bash
  grep -r "TODO\|FIXME\|unimplemented!\|todo!\|stub" crates/ --include="*.rs"
  grep -r "not yet implemented\|placeholder" crates/ --include="*.rs"
  ```
  - Identify stubs in production paths vs tests

- [ ] **3.4: Unwrap Audit**
  ```bash
  grep -r "\.unwrap()\|\.expect(" crates/ --include="*.rs" | grep -v test | grep -v "_dst.rs"
  ```
  - Production code should use `?` or explicit error handling

- [ ] **3.5: Test Coverage Assessment**
  - Use `cargo tarpaulin` if available
  - Or manual review of critical paths

**Deliverable:** `.kelpie-index/audits/code_quality_audit.md`

---

#### Phase 4: Foundation Fixes

**Goal:** Fix issues identified in Phases 1-3 and refactor tests to use invariants from Phase 1.5.

**Prerequisites:** Phases 1-3 complete, Phase 1.5 invariants defined.

- [ ] **4.1: Critical DST Gaps**
  - Add DST tests for uncovered critical paths
  - Priority: actor lifecycle, storage, migration

- [ ] **4.2: Fake DST Remediation**
  - Convert fake DST to real DST (use Simulation harness)
  - Or rename to `*_chaos.rs` if truly non-deterministic

- [ ] **4.3: ADR Updates**
  - Update stale ADRs to match implementation

- [ ] **4.5: Refactor Tests to Use Invariants** [NEW - from Phase 1.4 findings]
  - Priority 1: Refactor 5 key test files as proof of concept
    - `agent_service_dst.rs`
    - `delete_atomicity_test.rs`
    - `agent_streaming_dst.rs`
    - `memory_tools_real_dst.rs`
    - `full_lifecycle_dst.rs`
  - Priority 2: Refactor remaining ~35 test files
  - Transform from smoke tests to invariant verification:
    ```rust
    // OLD: Smoke test
    match service.create_agent(request).await {
        Ok(_) => {},
        Err(_) => continue,  // Hides bugs!
    }

    // NEW: Invariant verification
    match service.create_agent(request).await {
        Ok(agent) => {
            tracker.record_create(&agent);
            assert_create_get_consistent(&service, &agent).await?;
        },
        Err(e) => println!("Expected failure under faults: {}", e),
    }
    let violations = tracker.verify_all(&service).await;
    assert!(violations.is_empty());
    ```

- [ ] **4.6: Add Fault-Type Distinction** [NEW]
  - Use `assert_fails_under_fault!` for expected failures
  - Use `assert_no_corruption!` for graceful degradation
  - Tests should distinguish "expected transient failure" from "bug"

- [ ] **4.7: Create Missing ADRs**
  - Create ADRs for undocumented decisions identified in Phase 2

- [ ] **4.8: Code Quality Fixes**
  - Remove dead code
  - Fix stubs in production paths
  - Replace unwraps with proper error handling

- [ ] **4.5: Verification**
  ```bash
  cargo test --workspace
  cargo clippy --workspace -- -D warnings
  cargo fmt --check
  ```

**Deliverable:** Clean test run, updated ADRs, documented fixes

**Stage 1 Exit Criteria:**
- [ ] All critical paths have verified DST coverage
- [ ] Core ADRs (001-005) are current
- [ ] No fake DST in test suite
- [ ] No stubs in production code paths
- [ ] All tests pass
- [ ] **[NEW] Core invariants defined in `invariants.rs`**
- [ ] **[NEW] At least 5 test files refactored to use invariant verification**
- [ ] **[NEW] Invariant-to-TLA+ mapping documented**

---

### Stage 2: Formal Verification Infrastructure

**Prerequisites:** Stage 1 complete.

#### Phase 5: TLA+ Specifications

**Goal:** Create formal TLA+ specifications for distributed core components.

**Reference:** VDE paper section on TLA+ integration.

**Input from Phase 1.5:** The invariants defined in `invariants.rs` are formalized here as TLA+ specs.

| Phase 1.5 Rust Invariant | Phase 5 TLA+ Invariant | Spec File |
|--------------------------|------------------------|-----------|
| `SINGLE_ACTIVATION` | SingleActivation | ActorLifecycle.tla |
| `CREATE_GET_CONSISTENCY` | Linearizability | ActorStorage.tla |
| `DELETE_GET_CONSISTENCY` | Linearizability | ActorStorage.tla |
| `MESSAGE_DURABILITY` | NoLostMessages | ActorLifecycle.tla |
| `TRANSACTION_ATOMICITY` | Isolation | ActorStorage.tla |
| `BLOCK_ATOMICITY` | Durability | ActorStorage.tla |

This mapping ensures:
1. DST tests verify the same invariants as TLA+ specs
2. Bugs caught by TLA+ model checker have corresponding DST regression tests
3. Single source of truth for system correctness properties

- [ ] **5.1: Actor Lifecycle Spec** (`specs/tla/ActorLifecycle.tla`)
  - States: Inactive, Activating, Active, Deactivating
  - Operations: Activate, Invoke, Deactivate
  - Invariants:
    - SingleActivation: At most one active instance per ActorId
    - NoLostMessages: Invocations during deactivation are queued
    - SafeDeactivation: State persisted before deactivation completes

- [ ] **5.2: Actor Storage Spec** (`specs/tla/ActorStorage.tla`)
  - Operations: Get, Put, Delete, Transaction
  - Invariants:
    - Linearizability: Operations appear atomic
    - Durability: Committed writes survive crashes
    - Isolation: Concurrent transactions don't interfere

- [ ] **5.3: Actor Migration Spec** (`specs/tla/ActorMigration.tla`)
  - Operations: Snapshot, Transfer, Restore
  - Invariants:
    - AtomicVisibility: Actor either at source or destination, never both
    - NoStateLoss: All state transferred correctly
    - NoMessageLoss: In-flight messages handled

- [ ] **5.4: Spec Validation**
  ```bash
  # Run TLC model checker
  tlc specs/tla/ActorLifecycle.tla
  tlc specs/tla/ActorStorage.tla
  tlc specs/tla/ActorMigration.tla
  ```

- [ ] **5.5: Spec-to-Test Mapping**
  - Document which TLA+ invariants map to which DST tests
  - Store in `.kelpie-index/specs/invariant-test-mapping.yaml`

**Deliverable:** TLA+ specs, passing TLC, documented mapping

---

#### Phase 6: Verification Pyramid

**Goal:** Build layered verification with increasing confidence.

**Levels:**
```
                    ┌─────────────────┐
                    │  Kani (proofs)  │ ~60s, bounded proofs
                    └────────┬────────┘
                    ┌────────▼────────┐
                    │   Stateright    │ ~30-60s, exhaustive states
                    └────────┬────────┘
                    ┌────────▼────────┐
                    │      DST        │ ~5s, simulation with faults
                    └────────┬────────┘
                    ┌────────▼────────┐
                    │   Unit Tests    │ ~1s, basic correctness
                    └─────────────────┘
```

- [ ] **6.1: Document Pyramid in CLAUDE.md**
  - When to use each level
  - Commands for each level
  - Expected timing

- [ ] **6.2: Stateright Integration**
  - Add `stateright` to `Cargo.toml`
  - Create model for actor lifecycle
  - Map to TLA+ invariants

- [ ] **6.3: Kani Integration** (optional, if installed)
  - Add Kani harnesses for critical functions
  - Focus on bounded proofs for core invariants

- [ ] **6.4: Verification Skill**
  - Create `.claude/skills/verification-pyramid.md`
  - When to run each level
  - How to interpret results

**Deliverable:** Working verification pyramid, documented usage

---

#### Phase 7: Production Telemetry Integration

**Goal:** Ground verification in real-world behavior.

**Note:** This phase is relevant when Kelpie is deployed. Can be deferred if not yet in production.

- [ ] **7.1: Define Telemetry Interface**
  - What metrics matter? (latency, throughput, errors)
  - What traces? (distributed traces, span context)
  - Provider-agnostic design

- [ ] **7.2: Implement Telemetry Client**
  - Support Prometheus, Datadog, or custom
  - Cache results with TTL (use VFS cache tools)

- [ ] **7.3: Integrate with Verification**
  - "Did the invariant hold in production?"
  - Telemetry as fifth level of pyramid

**Deliverable:** Telemetry interface, optional implementations

---

#### Phase 8: ADR → TLA+ → DST Pipeline

**Goal:** Create sustainable pipeline for ongoing development.

**Workflow:**
```
New Feature → Write ADR → Generate TLA+ → Generate DST → Implement → Verify
```

- [ ] **8.1: Document Pipeline Process**
  - Template for ADRs that support TLA+ generation
  - Guidelines for TLA+ spec writing
  - Guidelines for DST test creation

- [ ] **8.2: LLM-Assisted TLA+ Generation** (optional)
  - Use sub-LLM to draft TLA+ from ADR
  - Human review required
  - Store prompts and templates

- [ ] **8.3: Pipeline Validation Tools**
  - Check: Does ADR exist for component?
  - Check: Does TLA+ spec exist for ADR?
  - Check: Do DST tests cover TLA+ invariants?

- [ ] **8.4: Pipeline Skill**
  - Create `.claude/skills/spec-pipeline.md`
  - Workflow for new features
  - Validation commands

**Deliverable:** Documented pipeline, validation tools, skill file

---

## Checkpoints

- [ ] Codebase understood
- [ ] Plan approved
- [x] **Options & Decisions filled in**
- [x] **Quick Decision Log maintained**

### Stage 1 Checkpoints
- [x] Phase 1: DST Coverage Audit complete (1.4 done 2026-01-23)
- [ ] Phase 1.5: Define System Invariants [NEW]
- [ ] Phase 2: ADR Audit complete
- [ ] Phase 3: Code Quality Audit complete
- [ ] Phase 4: Foundation Fixes complete
- [ ] **Stage 1 Exit Criteria met**

### Stage 2 Checkpoints
- [ ] Phase 5: TLA+ Specifications complete
- [ ] Phase 6: Verification Pyramid complete
- [ ] Phase 7: Production Telemetry complete (or deferred)
- [ ] Phase 8: Pipeline complete

### Final
- [ ] Tests passing (`cargo test`)
- [ ] Clippy clean (`cargo clippy`)
- [ ] Code formatted (`cargo fmt`)
- [ ] /no-cap passed
- [ ] Vision aligned
- [ ] **What to Try section updated**
- [ ] Committed

---

## Test Requirements

**Phase 1-3 (Audits):**
- No new tests, just analysis

**Phase 4 (Fixes):**
- New DST tests for identified gaps
- All tests must pass after fixes

**Phase 5 (TLA+):**
- TLC model checker passes on all specs
- No deadlocks or invariant violations

**Phase 6 (Pyramid):**
- Stateright models compile and run
- Optional: Kani harnesses prove

**Commands:**
```bash
# Run all tests
cargo test --workspace

# Run DST tests specifically
cargo test -p kelpie-dst

# Reproduce specific DST failure
DST_SEED=12345 cargo test -p kelpie-dst

# Run Stateright tests (when implemented)
cargo test stateright_ -- --ignored

# Run TLC (when specs exist)
tlc specs/tla/ActorLifecycle.tla

# Run clippy
cargo clippy --all-targets --all-features

# Format code
cargo fmt
```

---

## Context Refreshes

| Time | Files Re-read | Notes |
|------|---------------|-------|
| | | |

---

## Blockers

| Blocker | Status | Resolution |
|---------|--------|------------|
| | | |

---

## Instance Log (Multi-Instance Coordination)

| Instance | Claimed Phases | Status | Last Update |
|----------|----------------|--------|-------------|
| | | | |

---

## Findings

### Thorough Examination Complete (2026-01-23)

**Total Tests: 208+ passing across workspace**
- kelpie-core: 27 tests ✅
- kelpie-runtime: 23 tests ✅
- kelpie-dst: 70 tests ✅
- kelpie-storage: 9 tests ✅ (8 ignored - require FDB cluster)
- kelpie-vm: 36 tests ✅
- kelpie-registry: 43 tests ✅
- kelpie-server: 70+ DST tests ✅
- kelpie-wasm: 0 tests (stub only)
- kelpie-agent: 0 tests (stub only)

### What Kelpie CAN Currently Do

| Capability | Status | Evidence |
|------------|--------|----------|
| **Actor Lifecycle** | ✅ Working | 23 runtime tests, activation/deactivation/invocation |
| **Actor State Persistence** | ✅ Working | KV store integration, JSON serialization |
| **Transactional KV** | ✅ Working | Atomic commit, read-your-writes, rollback |
| **DST Framework** | ✅ Working | 70 tests, 40+ fault types, deterministic simulation |
| **Fault Injection** | ✅ Working | Storage, crash, network, time, sandbox faults |
| **VM Abstraction** | ✅ Working | MockVm, Apple Vz, Firecracker backends |
| **Snapshot/Restore** | ✅ Working | CRC32 checksums, architecture validation |
| **Registry** | ✅ Working | Node management, placement, heartbeat tracking |
| **Agent Server** | ✅ Working | REST API, LLM integration, tools, DST coverage |
| **Agent Types** | ✅ Working | MemGPT, LettaV1, React with tool filtering |
| **Teleport Types** | ✅ Working | Package encode/decode, architecture checks |

### What Kelpie CANNOT Yet Do

| Capability | Status | Blocker |
|------------|--------|---------|
| **Distributed Single-Activation** | ❌ Missing | Cluster not integrated with runtime |
| **Multi-Node Deployment** | ❌ Missing | No distributed coordination |
| **WASM Actors** | ❌ Stub | kelpie-wasm is placeholder only |
| **FDB in CI** | ❌ External | Requires running FDB cluster |
| **Formal Verification** | ❌ Not Started | No TLA+ specs yet |

### Issues Found (17 total)

**High (1):**
- Cluster coordination not integrated with runtime - distributed single-activation not enforced

**Medium (6):**
- No distributed lock for single-activation (runtime)
- FDB tests require external cluster (storage)
- No tests for kelpie-cluster
- PlacementStrategy algorithms not implemented
- No actual network heartbeat sending
- LLM API key required for production

**Low (10):**
- Various minor gaps documented in ISSUES.md

### ADRs Status

**24 ADRs documented** covering:
- Core architecture (001-005): Virtual actors, FDB, WASM, linearizability, DST
- Compatibility (006): Letta-code compatibility
- Implementation (007-020): Storage, snapshots, tools, agent types, VM backends
- Most are **Accepted** status and appear current with implementation

### DST Quality Assessment

**Real DST (deterministic):**
- SimClock - explicit time control ✅
- DeterministicRng - seeded, reproducible ✅
- SimStorage - fault injection, transactions ✅
- SimNetwork - partitions, latency ✅
- FaultInjector - 40+ fault types ✅
- Simulation harness - determinism verification ✅

**Partial DST:**
- SimTeleportStorage - ignores RNG parameter
- SimVm - synthetic exec output, not fully deterministic

### Path to Formal Verification

**Foundation Status:**
- ✅ DST framework exists and is real DST
- ✅ 208+ tests passing
- ✅ ADRs are documented and mostly current
- ⚠️ Single-node only (no distributed coordination)
- ⚠️ 17 issues to address (1 high, 6 medium)

**Ready for Stage 2?**
Not yet. The high-priority issue (distributed single-activation) must be resolved first. However, single-node formal verification could proceed:
1. Actor lifecycle TLA+ spec - feasible now
2. Storage transaction TLA+ spec - feasible now
3. Cluster coordination TLA+ spec - need implementation first

**Recommended Next Steps:**
1. Fix DST determinism gaps (SimTeleportStorage RNG)
2. Add kelpie-cluster tests
3. Create actor lifecycle TLA+ spec (single-node first)
4. Integrate cluster with runtime for distributed guarantee

---

## What to Try [REQUIRED - UPDATE AFTER EACH PHASE]

### Works Now ✅
| What | How to Try | Expected Result |
|------|------------|-----------------|
| MCP Server | `cd kelpie-mcp && uv run --prerelease=allow mcp-kelpie` | Server starts, 37 tools available |
| Structural Indexes | `index_refresh` then `index_status` | Indexes built and queryable |
| RLM Analysis | `repl_load` + `repl_sub_llm` | Can analyze code with sub-LLM |
| Core Tests | `cargo test -p kelpie-core` | 27 tests pass |
| Runtime Tests | `cargo test -p kelpie-runtime` | 23 tests pass |
| DST Tests | `cargo test -p kelpie-dst --lib` | 70 tests pass |
| Storage Tests | `cargo test -p kelpie-storage` | 9 pass, 8 ignored (FDB) |
| VM Tests | `cargo test -p kelpie-vm` | 36 tests pass |
| Registry Tests | `cargo test -p kelpie-registry` | 43 tests pass |
| Full Workspace | `cargo test --workspace` | 208+ tests pass |

### Doesn't Work Yet ❌
| What | Why | When Expected |
|------|-----|---------------|
| Distributed single-activation | Cluster not integrated with runtime | Phase 4 |
| FDB tests in CI | Require external FDB cluster | External dependency |
| TLA+ specs | Need to write them | Phase 5 |
| Verification pyramid | Need TLA+ first | After Phase 5 |
| WASM actors | kelpie-wasm is stub only | P3 priority |

### Known Limitations ⚠️
- Single-node only (distributed coordination not integrated)
- FDB tests require external cluster (8 tests ignored)
- LLM API key required for production server
- kelpie-wasm and kelpie-agent are stubs only
- Stage 2 depends on Stage 1 completion
- TLA+ tools need to be installed (`brew install tla-plus`)
- Kani is optional (requires separate installation)

---

## Completion Notes

[To be filled when complete]

---

## Appendix: Extracted from Plan 026

This plan incorporates and supersedes the following from archived plan 026:
- Phase 11: Formal Methods Integration (now Phase 5)
- Phase 12: Verification Pyramid (now Phase 6)
- Phase 14: Production Telemetry (now Phase 7)
- Phase 15: ADR → TLA+ → Rust Pipeline (now Phase 8)

Key insight from 026: "Instructions tell the AI *what to do*; verification tells it *whether it worked*. Persistence lets it *remember what it learned*."

The foundation phases (1-4) are new, addressing the prerequisite that the codebase must be in known-good state before formal verification infrastructure can be built.
