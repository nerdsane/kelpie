# True DST Simulation Architecture

**Status:** PHASE 1 COMPLETE
**Created:** 2026-01-25
**Issue:** To be created after plan approval
**Branch:** feature/phase1-storage-time-provider
**Worktree:** ../kelpie-phase1-storage-time-provider

## Problem Statement

DST tests exist but don't test production code. They test algorithm mocks with simulated I/O,
which means bugs in the actual implementation (kelpie-runtime, kelpie-registry, kelpie-storage,
kelpie-cluster) are NOT caught by DST.

### Current State

| Component | Async Functions | TimeProvider | SimStorage | SimNetwork | DST Testable |
|-----------|-----------------|--------------|------------|------------|--------------|
| kelpie-runtime | 52 | 64% (33/52) | via trait | N/A | **Partial** |
| kelpie-registry | 99 | 74% (74/99) | via trait | N/A | **Partial** |
| kelpie-storage | 105 | 0% (0/105) | IS the sim | N/A | **No** |
| kelpie-cluster | 80 | 0% (0/80) | N/A | 0% | **No** |
| **Total** | **336** | **32%** | - | **0%** | - |

### Gap Analysis

1. **226 async functions** have no TimeProvider injection
2. **32 async functions** use real network (tokio::net)
3. **52 async functions** call FDB directly without abstraction
4. **18 DST test files** use mocks instead of production code

## Effort & Impact Analysis

### Effort Estimation

| Task | Files | Functions | Effort | Complexity |
|------|-------|-----------|--------|------------|
| Add TimeProvider to kelpie-storage | 6 | 105 | **3-4 days** | Medium |
| Add TimeProvider to kelpie-cluster | 7 | 80 | **3-4 days** | Medium |
| Add TimeProvider to remaining runtime/registry | 4 | 41 | **1-2 days** | Low |
| Abstract network layer (SimNetwork) | 1 | 32 | **2-3 days** | High |
| Abstract FDB backend | 2 | 52 | **3-4 days** | High |
| Rewrite DST tests to use production code | 18 | N/A | **5-7 days** | High |
| **Total** | **38** | **310** | **17-24 days** | |

### Impact Analysis

| Change | Risk Reduction | Value |
|--------|----------------|-------|
| TimeProvider in storage | Catch timing bugs in WAL, transactions | **High** |
| TimeProvider in cluster | Catch gossip/heartbeat timing bugs | **High** |
| SimNetwork for cluster RPC | Catch partition handling bugs | **Critical** |
| FDB abstraction | Test without real FDB in CI | **Medium** |
| Rewrite DST tests | Actually test production code | **Critical** |

### ROI Prioritization

```
                    HIGH IMPACT
                         │
    ┌────────────────────┼────────────────────┐
    │                    │                    │
    │  [P1] SimNetwork   │  [P3] FDB Abstract │
    │  for cluster RPC   │  (complex but      │
    │  (critical bugs)   │   enables CI)      │
    │                    │                    │
LOW ├────────────────────┼────────────────────┤ HIGH
EFFORT                   │                    EFFORT
    │                    │                    │
    │  [P2] TimeProvider │  [P4] Rewrite all  │
    │  injection         │  DST tests         │
    │  (incremental)     │  (big but needed)  │
    │                    │                    │
    └────────────────────┼────────────────────┘
                         │
                    LOW IMPACT
```

## Recommended Approach: Incremental Transformation

### Option A: Big Bang (NOT Recommended)
- Rewrite everything at once
- 4-6 weeks of work
- High risk of breaking changes
- Can't ship intermediate value

### Option B: Incremental (RECOMMENDED)
- Add I/O injection incrementally
- Ship value at each phase
- Lower risk, continuous validation
- 6-8 weeks total but value delivered throughout

## Phased Implementation Plan

### Phase 1: Foundation (Week 1-2)
**Goal:** Enable ONE production crate to be fully DST-testable

**Tasks:**
1. [x] Add TimeProvider to kelpie-storage WAL (**COMPLETE** - 2026-01-24)
   - Added `with_time_provider()` constructors to MemoryWal and KvWal
   - Removed `now_ms` parameters from WriteAheadLog trait
   - Implementations get time from injected TimeProvider
   - 11 tests updated/added, all passing
   - Commit: f1d00e1d

2. [x] Add TimeProvider to remaining kelpie-storage components (**N/A** - analysis revealed other components don't use time)
   - Analyzed: memory.rs, kv.rs, fdb.rs, transaction.rs
   - None use SystemTime::now() or time-dependent logic
   - WAL was the only time-dependent component in kelpie-storage

3. [x] Create ONE reference DST test using real storage code (**COMPLETE** - 2026-01-24)
   - Created `crates/kelpie-dst/tests/wal_production_dst.rs`
   - Uses production `MemoryWal` with `SimClock` from kelpie-dst
   - 8 tests demonstrating the pattern:
     - test_wal_append_with_sim_time
     - test_wal_time_ordering_with_sim_time
     - test_wal_pending_entries_with_sim_time
     - test_wal_lifecycle_with_sim_time
     - test_wal_determinism
     - test_wal_concurrent_operations
     - test_wal_cleanup_with_sim_time
     - test_wal_idempotency_with_sim_time
   - All 8 tests pass, proving the pattern works

**Deliverable:** Production storage code testable under simulated time ✅
**Effort:** 4-5 days
**Risk:** Low

### Phase 2: Cluster Time (Week 3)
**Goal:** Enable cluster timing tests

**Tasks:**
1. [ ] Add TimeProvider to kelpie-cluster (80 functions)
   - Focus on gossip.rs, heartbeat timing
   - Inject via ClusterConfig

2. [ ] Update cluster_dst.rs to use production code
   - Test real gossip protocol under simulated time

**Deliverable:** Gossip/heartbeat bugs catchable in DST
**Effort:** 3-4 days
**Risk:** Low

### Phase 3: Network Abstraction (Week 4-5)
**Goal:** Enable network partition testing of production code

**Tasks:**
1. [ ] Create NetworkProvider trait
   ```rust
   #[async_trait]
   pub trait NetworkProvider: Send + Sync {
       async fn connect(&self, addr: &str) -> Result<Box<dyn Connection>>;
       async fn listen(&self, addr: &str) -> Result<Box<dyn Listener>>;
   }
   ```

2. [ ] Refactor kelpie-cluster/rpc.rs to use NetworkProvider
   - 32 functions to update
   - Production uses TokioNetworkProvider
   - Tests use SimNetwork

3. [ ] Create partition_tolerance tests with real cluster code

**Deliverable:** Network partition bugs in RPC catchable
**Effort:** 5-6 days
**Risk:** Medium (API changes)

### Phase 4: FDB Abstraction (Week 6)
**Goal:** Run CI without real FDB

**Tasks:**
1. [ ] Create StorageBackend trait
   ```rust
   #[async_trait]
   pub trait StorageBackend: Send + Sync {
       async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
       async fn set(&self, key: &[u8], value: &[u8]) -> Result<()>;
       async fn transaction(&self) -> Result<Box<dyn Transaction>>;
   }
   ```

2. [ ] Refactor kelpie-registry/fdb.rs and kelpie-storage/fdb.rs
   - 52 functions total
   - Production uses FdbBackend
   - Tests use SimStorage (implements StorageBackend)

**Deliverable:** Full DST without FDB dependency
**Effort:** 4-5 days
**Risk:** Medium

### Phase 5: Test Migration (Week 7-8)
**Goal:** All DST tests use production code

**Tasks:**
1. [ ] Migrate each test file:
   | Test File | Priority | Effort |
   |-----------|----------|--------|
   | single_activation_dst.rs | P1 | 1 day |
   | lease_dst.rs | P1 | 1 day |
   | liveness_dst.rs | P2 | 0.5 day |
   | cluster_membership_dst.rs | P2 | 1 day |
   | partition_tolerance_dst.rs | P1 | 1 day |
   | Others (13 files) | P3 | 3 days |

2. [ ] Delete mock implementations (ActivationProtocol, etc.)

3. [ ] Update CLAUDE.md with new testing patterns

**Deliverable:** True TigerBeetle/FDB-style DST
**Effort:** 7-8 days
**Risk:** Low (tests only)

## Success Criteria

After all phases:
- [ ] 100% of async functions have TimeProvider injection
- [ ] 100% of network I/O goes through NetworkProvider
- [ ] 100% of storage I/O goes through StorageBackend trait
- [ ] 0 DST tests use mock protocols (HashMap-based state)
- [ ] CI runs full DST without FDB/real network
- [ ] `DST_SEED=X cargo test` reproduces any DST test

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | Incremental over Big Bang | Lower risk, continuous value | Takes longer total |
| Start | Storage first | Most async functions, high impact | Delays network testing |
| Start | Skip FDB abstraction initially | Can test with MemoryStorage | Can't test FDB-specific bugs |
| Phase 1 | Skip TimeProvider for memory/kv/fdb | No time ops in these modules | Less code change, same DST value |
| Phase 1 | Create new test file instead of modifying fdb_transaction_dst | Cleaner separation, keeps SimStorage tests intact | Two test patterns coexist |

## Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| API breakage | Medium | High | Feature flags, deprecation period |
| Performance regression | Low | Medium | Benchmark before/after |
| Incomplete migration | Medium | Medium | Track coverage metrics |
| Team unfamiliarity | Low | Low | Document patterns in CLAUDE.md |

## Alternatives Considered

### Alternative 1: Simulation-only testing (Current State)
- **Pros:** Already done, fast tests
- **Cons:** Doesn't test production code, false confidence
- **Decision:** Rejected - defeats purpose of DST

### Alternative 2: Integration tests with real FDB
- **Pros:** Tests real code
- **Cons:** Non-deterministic, slow, requires infrastructure
- **Decision:** Keep as supplement, not replacement

### Alternative 3: Madsim automatic interception
- **Pros:** Less code changes
- **Cons:** Madsim doesn't intercept all I/O, magic behavior
- **Decision:** Rejected - explicit injection more reliable

## What Can Be Tested After Each Phase

| Phase | What Becomes Testable |
|-------|----------------------|
| 1 | Storage timing: WAL durability, transaction timeouts |
| 2 | Cluster timing: Gossip protocol, heartbeat failures |
| 3 | Network: Partition handling, message loss, reordering |
| 4 | Full stack: Everything without external dependencies |
| 5 | Complete: All production code under simulation |

## Next Steps

1. Review and approve this plan
2. Create GitHub issue for Phase 1
3. Start with TimeProvider injection in kelpie-storage
4. Validate pattern with one reference test
5. Continue to subsequent phases

## References

- [TigerBeetle Simulation Testing](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/SIMULATION.md)
- [FoundationDB Testing Paper](https://www.foundationdb.org/files/fdb-paper.pdf)
- [Deterministic Simulation Testing](https://notes.eatonphil.com/2024-08-20-deterministic-simulation-testing.html)
- Issue #62: I/O Provider Injection (completed - partial)
- Issue #43: TLA+ Invariant Bridge (completed)
