# Task: TLA+ Specification Implementation Alignment

**Created:** 2026-01-25 04:00:00
**State:** PLANNING

---

## Vision Alignment

**Vision files read:** CLAUDE.md, .vision/CONSTRAINTS.md (referenced in CLAUDE.md)

**Relevant constraints/guidance:**
- Simulation-first development (CONSTRAINTS.md §1) - All invariants must have DST coverage
- TigerStyle safety principles (CONSTRAINTS.md §3) - Assertions, explicit error handling
- No placeholders in production (CONSTRAINTS.md §4) - FdbRegistry todo!() must be implemented
- Verification-first development - Trust execution, not documentation
- DST determinism - All critical paths must be deterministically testable

---

## Task Description

**Problem:** The TLA+ specifications in `docs/tla/` define 12 formal models with critical safety and liveness invariants. After thorough verification, the implementation **violates 15 critical invariants**, including:

- **SingleActivation**: No OCC/CAS for distributed placement
- **LeaseUniqueness**: No fencing tokens, no grace period
- **NoSplitBrain**: No quorum enforcement, no primary election
- **MigrationAtomicity**: Source deactivation missing
- **IdleTimeoutRespected**: Dead code (should_deactivate never called)

**Goal:** Bring implementation into compliance with TLA+ specifications, with full DST test coverage for each invariant.

**Scope:**
- 6 crates: kelpie-core, kelpie-registry, kelpie-runtime, kelpie-storage, kelpie-cluster, kelpie-dst
- 12 TLA+ specifications
- 30 identified issues (15 critical, 8 high, 7 medium)

---

## Options & Decisions [REQUIRED]

### Decision 1: Implementation Order (Safety vs. Complexity)

**Context:** We have 15 critical violations. Which should we fix first?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Safety-First | Fix SingleActivation, Lease, NoSplitBrain first | Addresses most dangerous bugs; enables multi-node | Hardest problems first; slower visible progress |
| B: Quick-Wins | Fix ActorLifecycle, WAL recovery first | Fast progress; single-node immediately safer | Critical distributed bugs remain |
| C: Bottom-Up | Fix kelpie-core, then registry, then cluster | Clean dependency order; no rework | Mixes critical and non-critical |

**Decision:** Option A: Safety-First

**Reasoning:** The SingleActivation and LeaseUniqueness violations can cause **data corruption** in distributed deployments. Even if multi-node isn't production-ready, fixing these first:
1. Establishes correct patterns for all subsequent work
2. Unblocks DST tests that verify distributed invariants
3. Prevents developers from accidentally deploying unsafe code

**Trade-offs accepted:**
- Slower initial progress (FDB OCC is complex)
- May need to refactor other code once registry is fixed
- These are acceptable because correctness > velocity

---

### Decision 2: FDB Backend Strategy

**Context:** FdbRegistry is entirely `todo!()`. How do we implement it?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Full FDB | Implement complete FDB backend with OCC | Production-ready; real distributed system | Complex; requires FDB test infrastructure |
| B: FDB + Memory | Implement OCC in MemoryRegistry first, then port | Faster iteration; DST-testable first | Two implementations to maintain |
| C: Abstract OCC | Create OCC trait, implement for both | Clean abstraction; single logic | More upfront design work |

**Decision:** Option C: Abstract OCC

**Reasoning:** The TLA+ spec defines the OCC protocol clearly. We should:
1. Create `OptimisticConcurrencyControl` trait matching the spec
2. Implement for MemoryRegistry (DST-testable)
3. Implement for FdbRegistry (production)
4. Same logic, same tests, different backends

**Trade-offs accepted:**
- More upfront design work
- Trait abstraction adds indirection
- Acceptable because it ensures both backends have identical semantics

---

### Decision 3: DST Test Strategy

**Context:** Zero DST coverage for critical invariants. How do we structure tests?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Per-Invariant | One test file per TLA+ invariant | Clear mapping to specs; focused | Many small files |
| B: Per-Component | Tests organized by crate | Matches code structure | Invariants span components |
| C: Hybrid | Per-invariant for safety, per-component for liveness | Best of both | More complex organization |

**Decision:** Option C: Hybrid

**Reasoning:**
- Safety invariants (SingleActivation, LeaseUniqueness) need dedicated files because they're cross-cutting
- Liveness tests naturally fit in existing `liveness_dst.rs`
- This matches how the TLA+ specs are organized

**Trade-offs accepted:**
- More complex test organization
- Some duplication of setup code
- Acceptable for clarity of invariant verification

---

### Decision 4: Fencing Token Design

**Context:** LeaseUniqueness requires fencing tokens. What's the design?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Per-Lease Token | Each lease has its own monotonic token | Simple; matches spec directly | Token management per actor |
| B: Global Epoch | Cluster-wide epoch incremented on leadership change | Simpler token management | Coarser granularity |
| C: Hybrid | Global epoch + per-actor sequence | Best protection; fine-grained | More state to track |

**Decision:** Option A: Per-Lease Token

**Reasoning:** The TLA+ KelpieLease spec defines `FencingTokenMonotonic` per-actor. A per-lease token:
1. Directly maps to the spec
2. Allows independent actor operations
3. Is simpler to implement correctly

**Trade-offs accepted:**
- More state per actor (one u64)
- Must persist fencing token with lease
- Acceptable because it matches the verified spec

---

## Quick Decision Log [REQUIRED]

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 04:00 | Safety-first implementation order | Correctness > velocity | Slower initial progress |
| 04:00 | Abstract OCC trait | Same logic for Memory and FDB | More upfront design |
| 04:00 | Hybrid DST test organization | Safety tests dedicated, liveness in existing files | Complex organization |
| 04:00 | Per-lease fencing tokens | Matches TLA+ spec exactly | More state per actor |

---

## Implementation Plan

### Phase 1: Core Infrastructure (OCC + Fencing)
**Goal:** Establish correct primitives that all other fixes depend on.

- [ ] 1.1 Create `OptimisticConcurrencyControl` trait in kelpie-core
  - `read_version(key) -> (value, version)`
  - `write_if_version(key, value, expected_version) -> Result<(), ConflictError>`
- [ ] 1.2 Add `FencingToken` type to kelpie-core
  - `struct FencingToken(u64)`
  - `fn next(&self) -> FencingToken` (monotonic increment)
- [ ] 1.3 Add `MAX_CLOCK_SKEW_MS` constant to kelpie-core
- [ ] 1.4 Add `LEASE_GRACE_PERIOD_MS` constant to kelpie-core
- [ ] 1.5 Write unit tests for new types

### Phase 2: SingleActivation Fix (kelpie-registry)
**Goal:** Fix the most critical safety invariant.

- [ ] 2.1 Add `version: u64` field to `ActorPlacement`
- [ ] 2.2 Implement OCC trait for `MemoryRegistry`
  - Read returns current version
  - Write compares version atomically (using RwLock + version check)
- [ ] 2.3 Update `try_claim_actor()` to use OCC
  ```rust
  let (existing, version) = self.read_version(&actor_id)?;
  if existing.is_some() { return Err(AlreadyClaimed); }
  self.write_if_version(&actor_id, placement, version)?;
  ```
- [ ] 2.4 Update `unregister_actor()` to bump version (invalidate in-flight claims)
- [ ] 2.5 Implement OCC trait for `FdbRegistry`
  - Use FDB's native versionstamp or read version
- [ ] 2.6 Write DST test: `test_single_activation_concurrent_claims`
- [ ] 2.7 Write DST test: `test_single_activation_release_invalidates_inflight`

### Phase 3: LeaseUniqueness Fix (kelpie-registry)
**Goal:** Fix lease safety with fencing tokens and grace period.

- [ ] 3.1 Add `fencing_token: FencingToken` to `Lease` struct
- [ ] 3.2 Add `grace_period_ms: u64` to `LeaseConfig`
- [ ] 3.3 Update `acquire()` to:
  - Check `now_ms < existing.expiry_ms + grace_period_ms` before claiming
  - Increment fencing token atomically with acquisition
- [ ] 3.4 Update `renew()` to verify fencing token matches
- [ ] 3.5 Add clock skew buffer to expiry checks: `expiry_ms - MAX_CLOCK_SKEW_MS`
- [ ] 3.6 Integrate `LeaseManager` into `Registry` trait
- [ ] 3.7 Write DST test: `test_lease_uniqueness_concurrent_acquire`
- [ ] 3.8 Write DST test: `test_lease_grace_period_prevents_overlap`
- [ ] 3.9 Write DST test: `test_fencing_token_monotonic`

### Phase 4: NoSplitBrain Fix (kelpie-cluster)
**Goal:** Implement quorum enforcement and primary election.

- [ ] 4.1 Add `term: u64` field to cluster state
- [ ] 4.2 Implement `join_cluster()` properly (currently no-op)
  - Contact seed nodes
  - Exchange membership views
  - Reach consensus on view
- [ ] 4.3 Call `check_quorum()` before cluster operations
  - `try_claim_actor()` must verify quorum
  - `migrate_actor()` must verify quorum
- [ ] 4.4 Implement primary election (Raft-lite or lease-based)
  - Primary required for placement decisions
  - Step down on quorum loss
- [ ] 4.5 Add term comparison to conflict resolution
- [ ] 4.6 Write DST test: `test_no_split_brain_under_partition`
- [ ] 4.7 Write DST test: `test_quorum_loss_blocks_writes`
- [ ] 4.8 Write DST test: `test_term_monotonic_override`

### Phase 5: MigrationAtomicity Fix (kelpie-cluster)
**Goal:** Ensure no dual activation during migration.

- [ ] 5.1 Add `deactivate_on_source()` RPC to migration protocol
- [ ] 5.2 Update migration flow:
  ```
  1. Prepare (target checks capacity)
  2. Transfer (state sent to target)
  3. Deactivate source (NEW - RPC to source)
  4. Complete (target activates)
  ```
- [ ] 5.3 Add migration rollback on failure
  - If step 3 fails: abort, source continues
  - If step 4 fails: deactivate target, reactivate source
- [ ] 5.4 Add state checksum verification before activation
- [ ] 5.5 Write DST test: `test_migration_no_dual_activation`
- [ ] 5.6 Write DST test: `test_migration_rollback_on_failure`
- [ ] 5.7 Write DST test: `test_migration_state_integrity`

### Phase 6: ActorLifecycle Fix (kelpie-runtime)
**Goal:** Enforce idle timeout and fix lifecycle guards.

- [ ] 6.1 Replace `assert!(state == Active)` with runtime error
  ```rust
  if self.state != ActivationState::Active {
      return Err(Error::ActorNotActive { actor_id: self.id.clone() });
  }
  ```
- [ ] 6.2 Add periodic idle check task to dispatcher
  ```rust
  // In dispatcher run loop:
  if now_ms - last_idle_check_ms > IDLE_CHECK_INTERVAL_MS {
      for actor in actors.values() {
          if actor.should_deactivate() {
              self.handle_deactivate(&actor.id).await;
          }
      }
  }
  ```
- [ ] 6.3 Add deactivation guard to `handle_invoke()`
  ```rust
  if active.activation_state() == ActivationState::Deactivating {
      return Err(Error::ActorDeactivating { actor_id });
  }
  ```
- [ ] 6.4 Wait for pending invocations in `handle_deactivate()`
- [ ] 6.5 Write DST test: `test_idle_timeout_triggers_deactivation`
- [ ] 6.6 Write DST test: `test_no_invoke_while_deactivating`

### Phase 7: WAL Recovery Fix (kelpie-storage)
**Goal:** Ensure WAL recovery runs on startup.

- [ ] 7.1 Add `recover()` method to WAL trait
  ```rust
  async fn recover(&self) -> Result<Vec<WalEntry>> {
      let pending = self.pending_entries().await?;
      for entry in &pending {
          // Mark as recovering
      }
      Ok(pending)
  }
  ```
- [ ] 7.2 Call WAL recovery in server startup (kelpie-server)
- [ ] 7.3 Add idempotency key index (`HashMap<String, WalEntryId>`)
- [ ] 7.4 Schedule WAL cleanup task (periodic `cleanup()` calls)
- [ ] 7.5 Write DST test: `test_wal_recovery_replays_pending`
- [ ] 7.6 Write DST test: `test_wal_idempotency_prevents_duplicate`

### Phase 8: DST Comprehensive Coverage
**Goal:** Ensure all TLA+ invariants have DST tests.

- [ ] 8.1 Create `single_activation_dst.rs` (dedicated invariant tests)
- [ ] 8.2 Create `lease_uniqueness_dst.rs` (dedicated invariant tests)
- [ ] 8.3 Add network partition fault to SimNetwork
- [ ] 8.4 Add clock skew fault to SimClock
- [ ] 8.5 Write `test_serializable_isolation_concurrent_txn`
- [ ] 8.6 Write `test_cluster_membership_view_convergence`
- [ ] 8.7 Verify all DST tests are deterministic (same seed = same result)

### Phase 9: Cleanup and Documentation
**Goal:** Remove dead code, update docs.

- [ ] 9.1 Remove `should_deactivate()` dead code warning (now used)
- [ ] 9.2 Update ADRs for OCC and fencing token decisions
- [ ] 9.3 Add TLA+ ↔ Implementation mapping documentation
- [ ] 9.4 Update CLAUDE.md with new invariant test requirements
- [ ] 9.5 Final verification: `cargo test && cargo clippy && cargo fmt`

---

## Checkpoints

- [x] Codebase understood
- [ ] Plan approved
- [x] **Options & Decisions filled in**
- [x] **Quick Decision Log maintained**
- [ ] Implemented
- [ ] Tests passing (`cargo test`)
- [ ] Clippy clean (`cargo clippy`)
- [ ] Code formatted (`cargo fmt`)
- [ ] /no-cap passed
- [ ] Vision aligned
- [ ] **DST coverage added** (all 12 TLA+ specs)
- [ ] **What to Try section updated**
- [ ] Committed

---

## Test Requirements

**Unit tests:**
- `kelpie-core`: OCC trait, FencingToken, constants
- `kelpie-registry`: Version-based claim, lease with fencing
- `kelpie-runtime`: Lifecycle state transitions
- `kelpie-storage`: WAL recovery, idempotency index

**DST tests (CRITICAL - all invariants):**

| TLA+ Spec | DST Test File | Faults to Inject |
|-----------|---------------|------------------|
| KelpieSingleActivation | `single_activation_dst.rs` | Concurrent claims, OCC conflicts |
| KelpieLease | `lease_uniqueness_dst.rs` | Clock skew, concurrent acquire |
| KelpieClusterMembership | `cluster_membership_dst.rs` | Network partition, node failure |
| KelpieMigration | `migration_dst.rs` | Crash during transfer, rollback |
| KelpieActorLifecycle | `actor_lifecycle_dst.rs` | Idle timeout, concurrent deactivate |
| KelpieWAL | `wal_recovery_dst.rs` | Crash before/after write |
| KelpieFDBTransaction | `fdb_transaction_dst.rs` | Concurrent conflicting txns |

**Commands:**
```bash
# Run all tests
cargo test

# Run DST tests specifically
cargo test -p kelpie-dst

# Run new invariant tests
cargo test -p kelpie-dst single_activation
cargo test -p kelpie-dst lease_uniqueness
cargo test -p kelpie-dst cluster_membership

# Verify determinism
DST_SEED=12345 cargo test -p kelpie-dst single_activation -- --nocapture > run1.txt
DST_SEED=12345 cargo test -p kelpie-dst single_activation -- --nocapture > run2.txt
diff run1.txt run2.txt  # Must be identical

# Run clippy
cargo clippy --all-targets --all-features

# Format code
cargo fmt
```

---

## Dependencies Between Phases

```
Phase 1 (Core Infrastructure)
    ↓
Phase 2 (SingleActivation) ←─── Phase 3 (LeaseUniqueness)
    ↓                              ↓
Phase 4 (NoSplitBrain) ───────────┘
    ↓
Phase 5 (MigrationAtomicity)
    ↓
Phase 6 (ActorLifecycle) ─── Phase 7 (WAL Recovery)
    ↓                              ↓
Phase 8 (DST Coverage) ────────────┘
    ↓
Phase 9 (Cleanup)
```

**Critical Path:** 1 → 2 → 4 → 5 (distributed safety)
**Parallel Track:** 6, 7 (can be done independently after Phase 1)

---

## Effort Estimates (Relative)

| Phase | Complexity | Risk | Estimated Effort |
|-------|------------|------|------------------|
| Phase 1 | Medium | Low | 1x |
| Phase 2 | High | High | 3x |
| Phase 3 | High | High | 2x |
| Phase 4 | Very High | Very High | 5x |
| Phase 5 | Medium | Medium | 2x |
| Phase 6 | Low | Low | 1x |
| Phase 7 | Low | Low | 1x |
| Phase 8 | Medium | Low | 2x |
| Phase 9 | Low | Low | 0.5x |

**Total:** ~17.5x base unit

---

## Blockers

| Blocker | Status | Resolution |
|---------|--------|------------|
| FDB test infrastructure | Pending | Need FDB running for FdbRegistry tests |
| Network partition simulation | Pending | SimNetwork needs partition fault |
| Clock skew simulation | Pending | SimClock needs skew injection |

---

## Instance Log (Multi-Instance Coordination)

| Instance | Claimed Phases | Status | Last Update |
|----------|----------------|--------|-------------|
| | | | |

---

## Findings

**Key Discoveries from TLA+ Verification:**

1. **FdbRegistry is completely unimplemented** - All methods are `todo!()`
2. **LeaseManager and Registry are not integrated** - Two parallel paths
3. **should_deactivate() is dead code** - Never called anywhere
4. **check_quorum() exists but is never called** - Defense not deployed
5. **join_cluster() is a no-op** - TODO comment admits Phase 3 needed
6. **Source deactivation missing from migration** - Critical dual-activation bug

**TLA+ Spec Mapping:**
- Specs are well-designed and complete
- TTrace files are TLC model checker outputs (can reproduce with TLC)
- All 12 specs have clear invariant definitions

---

## What to Try [REQUIRED - UPDATE AFTER EACH PHASE]

### Works Now ✅
| What | How to Try | Expected Result |
|------|------------|-----------------|
| Single-node deployment | `cargo run -p kelpie-server` | Server starts, accepts requests |
| Actor invocation | `curl -X POST /v1/agents/{id}/messages` | Messages processed |
| Memory storage | Create/read/update agents | State persists in memory |

### Doesn't Work Yet ❌
| What | Why | When Expected |
|------|-----|---------------|
| Multi-node SingleActivation | No OCC/CAS | Phase 2 |
| Lease fencing tokens | Not implemented | Phase 3 |
| Quorum enforcement | check_quorum() not called | Phase 4 |
| Safe migration | Source not deactivated | Phase 5 |
| Idle timeout | Dead code | Phase 6 |
| WAL recovery | Never invoked | Phase 7 |

### Known Limitations ⚠️
- **Single-node only safe** - Multi-node deployment can cause data corruption
- **MemoryRegistry only** - FdbRegistry is unimplemented
- **No network partition handling** - Split-brain possible
- **Clock skew not handled** - Lease overlap possible with skewed clocks

---

## Completion Notes

**Verification Status:**
- Tests: [pending]
- Clippy: [pending]
- Formatter: [pending]
- /no-cap: [pending]
- Vision alignment: [pending]

**DST Coverage (target):**
- Fault types: OCC conflict, clock skew, network partition, crash, concurrent access
- Seeds: Randomized + regression seeds for known issues
- Determinism: Must verify same seed = same result

**Key Decisions Made:**
- Safety-first implementation order
- Abstract OCC trait for Memory and FDB
- Per-lease fencing tokens matching TLA+ spec

**What to Try (Final):**
| What | How to Try | Expected Result |
|------|------------|-----------------|
| [Pending completion] | | |

**Commit:** [pending]
**PR:** [pending]
