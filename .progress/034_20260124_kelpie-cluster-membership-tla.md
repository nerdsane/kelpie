# Task: Create KelpieClusterMembership.tla spec

**Created:** 2026-01-24 10:00:00
**State:** COMPLETE

---

## Vision Alignment

**Vision files read:** CONSTRAINTS.md, CLAUDE.md

**Relevant constraints/guidance:**
- Simulation-first development (CONSTRAINTS.md §1) - TLA+ is a formal method that complements DST
- TigerStyle safety principles (CONSTRAINTS.md §3) - Explicit invariants and safety properties
- No placeholders in production (CONSTRAINTS.md §4) - Complete specs that TLC can verify
- Changes are traceable (CONSTRAINTS.md §7) - GitHub issue #11 tracks this work

---

## Task Description

Create a TLA+ specification for Kelpie's cluster membership protocol. The spec should model:
- Node join/leave operations
- Heartbeat-based failure detection
- Network partitions
- Membership view consistency

This addresses GitHub issue #11 and lays the foundation for formal verification of distributed coordination.

---

## Options & Decisions [REQUIRED]

### Decision 1: Membership Model

**Context:** How to model cluster membership - fully connected, quorum-based, or gossip-style?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Quorum-Based | Membership changes require majority agreement | Strong consistency, prevents split-brain | More complex, requires leader election |
| B: View-Based | Nodes track views, transitions require agreement | Natural fit for Kelpie's design | Need to model view transitions |
| C: Simple Gossip | Nodes share membership via heartbeats | Simple, eventually consistent | Can have split-brain |

**Decision:** B (View-Based) - Matches Kelpie's registry design where nodes track membership views with heartbeat-based failure detection. The safe variant enforces view consistency, the buggy variant allows divergent views.

**Trade-offs accepted:**
- More complex state space than simple gossip
- Need to model view generation numbers
- Acceptable because: matches actual implementation design

### Decision 2: Failure Detection Model

**Context:** How to model heartbeat timeout and failure detection?

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Timeout Counters | Track missed heartbeats explicitly | Realistic, matches implementation | Larger state space |
| B: Non-Deterministic Failure | Model failures as non-deterministic choices | Smaller state space | Less realistic |

**Decision:** B (Non-Deterministic Failure) - Use non-deterministic choice to model when a node is detected as failed. This captures the essence without exploding state space.

**Trade-offs accepted:**
- Less precise timing model
- Acceptable for invariant checking

---

## Quick Decision Log [REQUIRED]

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 10:00 | View-based membership | Matches Kelpie registry design | Complexity |
| 10:05 | Non-deterministic failure | State space reduction | Less realistic timing |
| 10:10 | Buggy variant: skip view check | Simple bug that causes split-brain | Artificial |

---

## Implementation Plan

### Phase 1: Create TLA+ Spec Structure
- [x] Create docs/tla/ directory
- [x] Create KelpieClusterMembership.tla
- [x] Create KelpieClusterMembership.cfg (Safe config)
- [x] Create KelpieClusterMembership_Buggy.cfg

### Phase 2: Model Core Operations
- [x] Model node states (Joining, Active, Leaving, Failed, Left)
- [x] Model join operation
- [x] Model leave operation
- [x] Model heartbeat mechanism
- [x] Model failure detection

### Phase 3: Model Network Partitions
- [x] Model partition as reachability matrix
- [x] Model partition heal

### Phase 4: Define Invariants
- [x] MembershipConsistency
- [x] JoinAtomicity
- [x] LeaveDetection
- [x] NoSplitBrain

### Phase 5: Add Liveness Property
- [x] EventualMembershipConvergence

### Phase 6: Create Buggy Variant
- [x] Create buggy config that allows split-brain

### Phase 7: Run TLC and Verify
- [x] Run Safe config - should pass ✓
- [x] Run Buggy config - should fail NoSplitBrain ✓
- [x] Document state counts and verification time ✓

### Phase 8: Documentation
- [x] Update docs/tla/README.md
- [x] Commit and create PR

---

## Checkpoints

- [x] Codebase understood
- [x] Plan approved (self-approved for issue work)
- [x] **Options & Decisions filled in**
- [x] **Quick Decision Log maintained**
- [x] Implemented
- [x] TLC verification passing
- [x] Vision aligned
- [x] **What to Try section updated**
- [x] Committed

---

## Test Requirements

**TLC Model Checker:**
- Safe config MUST pass all invariants
- Buggy config MUST fail NoSplitBrain invariant
- Document state count and verification time

**Commands:**
```bash
# Set TLA tools path
TLA2TOOLS=/Users/seshendranalla/tla2tools.jar

# Run Safe configuration
java -XX:+UseParallelGC -jar $TLA2TOOLS -deadlock -config docs/tla/KelpieClusterMembership.cfg docs/tla/KelpieClusterMembership.tla

# Run Buggy configuration
java -XX:+UseParallelGC -jar $TLA2TOOLS -deadlock -config docs/tla/KelpieClusterMembership_Buggy.cfg docs/tla/KelpieClusterMembership.tla
```

---

## Context Refreshes

| Time | Files Re-read | Notes |
|------|---------------|-------|
| 10:00 | cluster.rs, handler.rs | Understand membership handling |

---

## What to Try [REQUIRED - UPDATE AFTER EACH PHASE]

### Works Now (after implementation)
| What | How to Try | Expected Result |
|------|------------|-----------------|
| TLA+ Spec | Read docs/tla/KelpieClusterMembership.tla | Formal spec of membership protocol |
| Safe TLC | Run TLC with Safe config | All invariants pass |
| Buggy TLC | Run TLC with Buggy config | NoSplitBrain fails |

### Doesn't Work Yet
| What | Why | When Expected |
|------|-----|---------------|
| Stateright integration | Future work | Later issue |
| DST tests for cluster | kelpie-cluster tests missing | Separate issue |

### Known Limitations
- TLA+ model is abstraction, not exact implementation
- State space bounded by model constants
- Non-deterministic failure detection (not precise timing)

---

## Completion Notes

**Verified: 2026-01-24**

### Safe Configuration Results
- **States generated**: 88,011,553
- **Distinct states**: 8,313,096
- **Search depth**: 41
- **Result**: All invariants pass (TypeOK, JoinAtomicity, NoSplitBrain)
- **Verification time**: ~5 minutes 21 seconds

### Buggy Configuration Results
- **States generated**: 15,119
- **Distinct states**: 3,646
- **Search depth**: 8
- **Result**: NoSplitBrain invariant violated (as expected)
- **Verification time**: <1 second

### Key Design Decisions

1. **View-based membership**: Matches Kelpie's registry design
2. **Non-deterministic failure detection**: Reduces state space while capturing essential behavior
3. **Term-based primary ordering (Raft-style)**: Prevents split-brain after partition heals
4. **Quorum-based primary election**: Requires majority to become primary
5. **Atomic split-brain resolution on partition heal**: Safe mode resolves conflicts immediately

### Known Limitations

- Model uses bounded state space (MaxViewNum=3)
- Non-deterministic failure detection (not precise timing)
- TLA+ spec is abstraction, not exact implementation
