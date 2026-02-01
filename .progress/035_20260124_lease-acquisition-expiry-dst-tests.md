# Task: DST - Add Lease Acquisition and Expiry Testing (#22)

**Created:** 2026-01-24 16:00:00
**State:** IMPLEMENTING

---

## Vision Alignment

**Vision files read:** CONSTRAINTS.md

**Relevant constraints/guidance:**
- Simulation-first development (CONSTRAINTS.md §1) - Tests must use DST harness with fault injection
- TigerStyle safety principles (CONSTRAINTS.md §3) - Explicit constants, 2+ assertions per function
- No placeholders in production (CONSTRAINTS.md §4)

---

## Task Description

Implement lease management infrastructure and DST tests per GitHub issue #22. The TLA+ spec (`docs/tla/KelpieLease.tla`) defines lease invariants that need DST test coverage:

- **LeaseUniqueness**: At most one node believes it holds a valid lease per actor
- **BeliefConsistency**: If a node believes it holds a lease, it actually does
- **RenewalRequiresOwnership**: Only lease holder can renew
- **ExpiredLeaseClaimable**: Expired leases don't block acquisition

---

## Options & Decisions

### Decision 1: Where to Put Lease Management Code

**Context:** Need lease infrastructure for DST tests. Options are kelpie-registry, kelpie-cluster, or new crate.

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: kelpie-registry | Add lease module alongside placement | Leases are related to placement, natural fit | Adds to existing crate |
| B: kelpie-cluster | Add to cluster crate | Close to cluster coordination | Would create circular dep with registry |
| C: New kelpie-lease crate | Dedicated crate | Clean separation | Overhead for small module |

**Decision:** Option A - Add to kelpie-registry. Leases are fundamentally about who "owns" an actor placement, which is registry's domain.

**Trade-offs accepted:**
- Registry crate grows larger
- This is acceptable because leases are tightly coupled with placement semantics

---

### Decision 2: LeaseManager Interface Design

**Context:** Should LeaseManager be a trait or concrete struct?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Trait + Impl | LeaseManager trait with MemoryLeaseManager | Can swap implementations, testable | More code |
| B: Concrete | Just MemoryLeaseManager | Simpler | Less flexible |

**Decision:** Option A - Use trait. Allows future FoundationDB-backed implementation.

**Trade-offs accepted:**
- More boilerplate now
- Worth it for future extensibility

---

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 16:00 | Use Arc<Clock> for time | Matches existing pattern in registry | None |
| 16:00 | Lease duration as config | Flexible for testing | Slightly more code |
| 16:00 | Error variants in existing error.rs | Keep errors consolidated | Registry error grows |

---

## Implementation Plan

### Phase 1: Create Lease Module in kelpie-registry
- [x] Create `crates/kelpie-registry/src/lease.rs`
- [x] Define Lease struct with holder, expiry fields
- [x] Define LeaseManager trait (acquire, renew, release, is_valid)
- [x] Implement MemoryLeaseManager
- [x] Add lease error variants to error.rs
- [x] Export from lib.rs

### Phase 2: Create DST Tests
- [x] Create `crates/kelpie-dst/tests/lease_dst.rs`
- [x] Test 1: Lease acquisition race (single winner)
- [x] Test 2: Lease expiry allows reacquisition
- [x] Test 3: Lease renewal extends validity
- [x] Test 4: Non-holder cannot renew
- [x] Test 5: Lease release
- [x] Stress test with many iterations (ignored)
- [x] Determinism test (same seed = same result)

### Phase 3: Verification
- [x] Run cargo test
- [x] Run cargo clippy
- [x] Run cargo fmt
- [x] Verify all tests pass

---

## Checkpoints

- [x] Codebase understood
- [x] Plan created
- [x] **Options & Decisions filled in**
- [x] **Quick Decision Log maintained**
- [x] Implemented
- [x] Tests passing (`cargo test`)
- [x] Clippy clean (`cargo clippy`)
- [x] Code formatted (`cargo fmt`)
- [ ] /no-cap passed
- [x] Vision aligned
- [x] **DST coverage added**
- [x] **What to Try section updated**
- [ ] Committed

---

## Test Requirements

**DST tests:**
- [x] Normal conditions test (acquire, renew, release)
- [x] Race condition test (concurrent acquisition)
- [x] Time-based test (expiry and reacquisition)
- [x] Determinism verification (same seed = same result)

**Commands:**
```bash
# Run all tests
cargo test

# Run lease DST tests specifically
cargo test -p kelpie-dst --test lease_dst

# Reproduce specific DST failure
DST_SEED=12345 cargo test -p kelpie-dst --test lease_dst
```

---

## What to Try

### Works Now ✅
| What | How to Try | Expected Result |
|------|------------|-----------------|
| Lease acquire | `cargo test -p kelpie-dst --test lease_dst test_dst_lease_acquisition_race` | Single winner from concurrent attempts |
| Lease expiry | `cargo test -p kelpie-dst --test lease_dst test_dst_lease_expiry_allows_reacquisition` | New node can acquire after expiry |
| Lease renewal | `cargo test -p kelpie-dst --test lease_dst test_dst_lease_renewal_extends_validity` | Renewal extends lease lifetime |
| Non-holder renew | `cargo test -p kelpie-dst --test lease_dst test_dst_non_holder_cannot_renew` | Returns NotLeaseHolder error |
| All lease tests | `cargo test -p kelpie-dst --test lease_dst` | All tests pass |

### Doesn't Work Yet ❌
| What | Why | When Expected |
|------|-----|---------------|
| Network partition test | Requires SimNetwork integration with LeaseManager | Future enhancement |

### Known Limitations ⚠️
- MemoryLeaseManager is in-memory only (no persistence)
- Network faults not yet integrated with lease operations
- No FDB-backed implementation yet

---

## Completion Notes

**Verification Status:**
- Tests: PASS
- Clippy: CLEAN
- Formatter: PASS
- Vision alignment: Confirmed (DST with fault injection)

**DST Coverage:**
- Tests: 8 tests (7 regular + 1 stress/ignored)
- Fault types tested: StorageWriteFail (10%)
- Determinism verified: Yes

**Key Decisions Made:**
- Lease module in kelpie-registry
- Trait-based LeaseManager for extensibility

**Commit:** TBD
**PR:** TBD
