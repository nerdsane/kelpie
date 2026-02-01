# Task: Add liveness properties to KelpieRegistry.tla

**Created:** 2026-01-24 12:30:00
**State:** COMPLETE

---

## Vision Alignment

**Vision files read:** CONSTRAINTS.md

**Relevant constraints/guidance:**
- Simulation-first development (CONSTRAINTS.md §1)
- Explicit over implicit (CONSTRAINTS.md §5)
- Quality over speed (CONSTRAINTS.md §6)

---

## Task Description

GitHub issue #13: Add liveness properties to KelpieRegistry.tla

The TLA+ specification for the Kelpie Registry needs to be created with:
1. Safety properties (SingleActivation, PlacementConsistency)
2. Liveness properties (EventualFailureDetection) - nodes that crash are eventually detected
3. Cache model for node-local placement caches with CacheCoherence safety property
4. TLC model checker verification with documented results

---

## Options & Decisions [REQUIRED]

### Decision 1: Cache Coherence Model

**Context:** How should we model node-local caches and coherence?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Strong consistency | Cache invalidation via global registry | Simple, correct by construction | Doesn't model real-world bugs |
| B: Eventually consistent | Cache entries have TTL, async invalidation | Models real bugs, useful for finding issues | More complex spec |
| C: Explicit invalidation | Cache miss triggers refresh | Middle ground | May miss some bugs |

**Decision:** Option B - Eventually consistent caches with async invalidation

**Trade-offs accepted:**
- More complex TLA+ spec
- More states to explore (longer TLC runtime)
- Better bug-finding capability justifies complexity

### Decision 2: Fairness Assumptions

**Context:** What fairness assumptions for liveness properties?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Weak fairness | Eventually each enabled action happens | Standard, widely used | May not find all bugs |
| B: Strong fairness | Each infinitely-often enabled action happens | Stronger guarantees | Harder to satisfy |

**Decision:** Option A - Weak fairness for heartbeat checking

**Trade-offs accepted:**
- Weak fairness is standard for failure detectors
- Strong fairness would be harder to implement in practice

---

## Quick Decision Log [REQUIRED]

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 12:30 | Create spec from scratch | No existing TLA+ spec found | Start with known-good patterns |
| 12:35 | Use weak fairness | Standard for failure detection | Weaker guarantees OK |
| 12:40 | Model 3 nodes, 3 actors | Balance between coverage and TLC runtime | Limited state space |

---

## Implementation Plan

### Phase 1: Create TLA+ Specification
- [x] Define state variables (nodes, placements, caches, heartbeats)
- [x] Define node states (Active, Suspect, Failed)
- [x] Define actions (RegisterNode, ClaimActor, Heartbeat, DetectFailure)
- [x] Add cache model with stale entries
- [x] Define safety invariants (SingleActivation, CacheCoherence)
- [x] Define liveness property (EventualFailureDetection)

### Phase 2: Create TLC Configuration
- [x] Create KelpieRegistry.cfg with constants
- [x] Configure safety and liveness checks
- [x] Set appropriate state space bounds

### Phase 3: Run TLC and Verify
- [x] Run TLC model checker
- [x] Document state count and verification time
- [x] Fix any issues found

### Phase 4: Documentation
- [x] Create docs/tla/README.md
- [x] Document spec structure and verification results

### Phase 5: Create PR
- [ ] Commit changes
- [ ] Push to branch
- [ ] Create PR with 'Closes #13'

---

## Checkpoints

- [x] Codebase understood
- [x] Plan approved (self)
- [x] **Options & Decisions filled in**
- [x] **Quick Decision Log maintained**
- [x] Implemented
- [ ] Tests passing (N/A - TLA+ spec)
- [ ] Clippy clean (N/A - TLA+ spec)
- [ ] Code formatted (N/A - TLA+ spec)
- [ ] /no-cap passed (N/A - TLA+ spec)
- [x] Vision aligned
- [ ] **What to Try section updated**
- [ ] Committed

---

## Test Requirements

**TLC Verification:**
- [ ] All safety invariants pass
- [ ] All liveness properties pass
- [ ] State space fully explored
- [ ] No deadlocks found

**Commands:**
```bash
# Run TLC model checker
cd docs/tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieRegistry.cfg KelpieRegistry.tla
```

---

## What to Try [REQUIRED - UPDATE AFTER EACH PHASE]

### Works Now
| What | How to Try | Expected Result |
|------|------------|-----------------|
| TLA+ spec | Open docs/tla/KelpieRegistry.tla | Valid TLA+ syntax |
| TLC config | Open docs/tla/KelpieRegistry.cfg | Valid config |
| TLC verification | `cd docs/tla && java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieRegistry.cfg KelpieRegistry.tla` | "No error has been found" |

### Doesn't Work Yet
| What | Why | When Expected |
|------|-----|---------------|
| N/A | All features complete | N/A |

### Known Limitations
- Default model uses 2 nodes, 2 actors for fast verification (~1 second)
- Bounded heartbeat counter (0..MaxHeartbeatMiss) for finite state space
- Simplified heartbeat model (counter threshold instead of real time)

---

## Completion Notes

**Verification Status:**
- TLC: **PASSED** - All safety invariants and liveness properties verified
- States explored: 22,845 generated, 6,174 distinct
- Search depth: 19
- Time: ~1 second
- No deadlocks found

**Key Decisions Made:**
- Eventually consistent cache model (caches can be stale, but eventually corrected)
- Weak fairness for HeartbeatTick, DetectFailure, and InvalidateCache actions
- Bounded heartbeat counter to ensure finite state space

**Commit:** See git log
**PR:** Closes #13
