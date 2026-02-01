# Kelpie Verification Pipeline

This document describes the canonical verification pipeline for Kelpie: **ADR → TLA+ → DST → Code**.

## Overview

Every significant feature in Kelpie follows a verification-driven development process:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ADR (Architecture Decision Record)                                         │
│  - Defines the problem and chosen solution                                  │
│  - Lists safety invariants that MUST hold                                   │
│  - Documents trade-offs and alternatives                                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                              ↓                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  TLA+ Specification                                                         │
│  - Formalizes the ADR's invariants mathematically                           │
│  - Models concurrent/distributed behavior                                   │
│  - TLC model checker proves invariants hold (or finds violations)           │
│  - Includes SpecSafe (correct) and SpecBuggy (violation examples)           │
├─────────────────────────────────────────────────────────────────────────────┤
│                              ↓                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  DST (Deterministic Simulation Testing)                                     │
│  - Implements tests that verify TLA+ invariants                             │
│  - Injects faults (network partitions, crashes, storage failures)           │
│  - Deterministic: same seed = same result                                   │
│  - Covers bug patterns from TLA+ SpecBuggy configs                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                              ↓                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  Implementation                                                             │
│  - Rust code that satisfies the TLA+ spec                                   │
│  - Must pass all DST tests                                                  │
│  - Production code with proper error handling                               │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Why This Pipeline?

### The Problem

Distributed systems are hard to test because:
- Race conditions are non-deterministic
- Network failures happen unpredictably
- Crashes can occur at any point
- Traditional testing misses edge cases

### The Solution

1. **ADRs** capture WHAT invariants we need (human-readable)
2. **TLA+** proves those invariants CAN hold (mathematical proof)
3. **DST** verifies invariants DO hold under faults (executable tests)
4. **Code** implements the verified design (production)

This is the same approach used by FoundationDB, TigerBeetle, and other mission-critical distributed systems.

## New Feature Checklist

When adding a significant feature, follow this checklist:

### 1. ADR Phase

- [ ] Create ADR documenting the decision
- [ ] List safety invariants (what MUST always be true)
- [ ] List liveness properties (what SHOULD eventually happen)
- [ ] Document failure modes and recovery
- [ ] Add "Formal Specification" section referencing TLA+ spec (if applicable)

### 2. TLA+ Phase

- [ ] Create `docs/tla/Kelpie{Feature}.tla` specification
- [ ] Model all actions (state transitions)
- [ ] Define invariants from ADR
- [ ] Create `.cfg` file for TLC model checker
- [ ] Run TLC and verify invariants pass
- [ ] Create `_Buggy.cfg` that demonstrates violations
- [ ] Add entry to `docs/tla/README.md`
- [ ] Add ADR cross-reference in spec header

### 3. DST Phase

- [ ] Create `crates/kelpie-dst/tests/{feature}_dst.rs`
- [ ] Map each TLA+ invariant to a verification function
- [ ] Map each TLA+ bug pattern to a test case
- [ ] Add fault injection tests (storage, network, crash)
- [ ] Verify determinism (same seed = same result)
- [ ] Add stress test (1000+ iterations)

### 4. Implementation Phase

- [ ] Write production code
- [ ] Run DST tests until all pass
- [ ] Fix any invariant violations found
- [ ] Verify no regressions (`cargo test`)
- [ ] Run clippy and fix warnings

## TLA+ to DST Mapping

Each TLA+ construct maps to DST patterns:

| TLA+ Construct | DST Equivalent |
|----------------|----------------|
| `INVARIANT` | `verify_*()` function in `common/invariants.rs` |
| Action (state transition) | Test scenario in `*_dst.rs` |
| `CONSTANT BUGGY` | Test with fault injection |
| Model checking states | DST seed-based exploration |
| Temporal property | `liveness_dst.rs` with timeouts |

## Spec-to-ADR Cross-References

Every TLA+ spec should reference its ADR:

```tla
(***************************************************************************)
(* KelpieSingleActivation.tla                                              *)
(*                                                                         *)
(* Models the single-activation guarantee for Kelpie virtual actors.       *)
(*                                                                         *)
(* Related ADR: docs/adr/001-virtual-actor-model.md                        *)
(*              docs/adr/004-linearizability-guarantees.md                 *)
(***************************************************************************)
```

Every ADR with formal verification should have a "Formal Specification" section:

```markdown
## Formal Specification

This ADR is formalized in [KelpieSingleActivation.tla](../tla/KelpieSingleActivation.tla).

### Safety Invariants

| Invariant | Description | TLA+ Definition |
|-----------|-------------|-----------------|
| SingleActivation | At most one active instance per actor | `SingleActivation` |
| PlacementConsistency | Registry placement matches actual location | `PlacementConsistency` |

### TLC Verification

- **Safe config**: All invariants hold (714 states, depth 27)
- **Buggy config**: SingleActivation violated with racy claims
```

## Current Coverage

| ADR | TLA+ Spec | DST Tests | Status |
|-----|-----------|-----------|--------|
| ADR-001: Virtual Actor Model | KelpieSingleActivation.tla | single_activation_dst.rs | ✅ Complete |
| ADR-002: FDB Integration | KelpieFDBTransaction.tla | fdb_transaction_dst.rs | ✅ Complete |
| ADR-004: Linearizability | KelpieLinearizability.tla | single_activation_dst.rs, liveness_dst.rs, linearizability_dst.rs | ✅ Complete |
| ADR-022: WAL Design | KelpieWAL.tla | (pending) | 📋 TLA+ done, DST pending |
| ADR-023: Actor Registry | KelpieRegistry.tla | cluster_dst.rs | ✅ Complete |
| ADR-024: Migration Protocol | KelpieMigration.tla | cluster_dst.rs | ✅ Complete |
| ADR-025: Cluster Membership | KelpieClusterMembership.tla | partition_tolerance_dst.rs, cluster_dst.rs | ✅ Complete |
| ADR-030: HTTP Linearizability | KelpieHttpApi.tla | http_api_dst.rs | ✅ Complete |
| ADR-028: Multi-Agent Communication | KelpieMultiAgentInvocation.tla | multi_agent_dst.rs | ✅ Complete |

### ADR-004 Linearizability - Detailed Status

**TLA+ Invariant Coverage (Actor Layer):**

| Invariant | DST Status | Location |
|-----------|------------|----------|
| `SequentialPerActor` | ✅ Covered | MPSC channel ordering (implicit) |
| `OwnershipConsistency` | ✅ Covered | `single_activation_dst.rs:852-878` |
| `EventualCompletion` | ✅ Covered | `liveness_dst.rs:706-769` |
| `EventualClaim` | ✅ Covered | `liveness_dst.rs:775-846` |
| `ReadYourWrites` | ✅ Covered | `linearizability_dst.rs` |
| `MonotonicReads` | ✅ Covered | `linearizability_dst.rs` |
| `DispatchConsistency` | ✅ Covered | `linearizability_dst.rs` |

### ADR-030 HTTP Linearizability - Detailed Status

**TLA+ Invariant Coverage (HTTP Layer):**

| Invariant | DST Status | Location |
|-----------|------------|----------|
| `IdempotencyGuarantee` | ✅ Covered | `http_api_dst.rs:test_idempotency_exactly_once` |
| `ExactlyOnceExecution` | ✅ Covered | `http_api_dst.rs:test_concurrent_idempotent_requests` |
| `ReadAfterWriteConsistency` | ✅ Covered | `http_api_dst.rs:test_create_get_consistency` |
| `DurableOnSuccess` | ✅ Covered | `http_api_dst.rs:test_durability_after_success` |
| `AtomicOperation` | ✅ Covered | Via idempotency cache atomicity |

**Implementation Layer Status:**

| Layer | Linearizable? | TLA+ Spec | DST Tests |
|-------|---------------|-----------|-----------|
| Actor Runtime | ✅ Yes | KelpieLinearizability.tla | single_activation_dst.rs, linearizability_dst.rs |
| Storage (FDB) | ✅ Yes | KelpieFDBTransaction.tla | fdb_transaction_dst.rs |
| Registry (FDB) | ✅ Yes | KelpieSingleActivation.tla | single_activation_dst.rs |
| HTTP API | ✅ Yes | KelpieHttpApi.tla | http_api_dst.rs |
| Cluster | ⚠️ Partial | KelpieClusterMembership.tla | cluster_dst.rs |

**Note:** The HTTP API layer now provides exactly-once semantics via idempotency tokens (ADR-030). Clients can safely retry requests with the same `Idempotency-Key` header.

### ADR-028 Multi-Agent Communication - Detailed Status

**TLA+ Invariant Coverage:**

| Invariant | DST Status | Location |
|-----------|------------|----------|
| `NoDeadlock` | ✅ Covered | `multi_agent_dst.rs:test_agent_call_cycle_detection` |
| `SingleActivationDuringCall` | ✅ Covered | `multi_agent_dst.rs:test_single_activation_during_cross_call` |
| `DepthBounded` | ✅ Covered | `multi_agent_dst.rs:test_agent_call_depth_limit` |
| `BoundedPendingCalls` | ✅ Covered | `multi_agent_dst.rs:test_bounded_pending_calls` |
| `CallsEventuallyComplete` (liveness) | ✅ Covered | `multi_agent_dst.rs:test_agent_call_timeout` |

**Fault Tolerance Tests:**

| Scenario | DST Status | Location |
|----------|------------|----------|
| Network partition | ✅ Covered | `multi_agent_dst.rs:test_agent_call_under_network_partition` |
| Storage faults | ✅ Covered | `multi_agent_dst.rs:test_agent_call_with_storage_faults` |
| Determinism | ✅ Covered | `multi_agent_dst.rs:test_determinism_multi_agent` |
| Stress with faults | ✅ Covered | `multi_agent_dst.rs:test_multi_agent_stress_with_faults` |

**Note:** Multi-agent tests are in `kelpie-server` (not `kelpie-dst`) because they require full agent infrastructure including LLM client, tool registry, and dispatcher.

## Running Verification

### TLA+ Model Checking

```bash
# Verify all specs pass
cd docs/tla
for spec in Kelpie*.tla; do
    java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config "${spec%.tla}.cfg" "$spec"
done
```

### DST Testing

```bash
# Run all DST tests
cargo test -p kelpie-dst

# Reproduce specific failure
DST_SEED=12345 cargo test -p kelpie-dst

# Stress test
cargo test -p kelpie-dst stress --release -- --ignored

# Verify determinism
DST_SEED=42 cargo test -p kelpie-dst test_name > run1.txt
DST_SEED=42 cargo test -p kelpie-dst test_name > run2.txt
diff run1.txt run2.txt  # Should be empty
```

## References

- [FoundationDB Testing Paper](https://www.foundationdb.org/files/fdb-paper.pdf)
- [TigerStyle Engineering](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)
- [TLA+ Home](https://lamport.azurewebsites.net/tla/tla.html)
- [TLA+ Toolbox](https://lamport.azurewebsites.net/tla/tools.html)
- [Hillel Wayne's TLA+ Guide](https://learntla.com/)
