# Parallel TLA+ Agent Handoff

**Created**: 2026-01-24
**Status**: Ready for Execution

## Overview

10 git worktrees created for parallel TLA+ spec development. Each Claude agent works independently on one issue, then creates a PR to master.

**All commands use:** `--model opus --dangerously-skip-permissions`

## Worktree Summary

| Issue | Worktree | Branch | Task |
|-------|----------|--------|------|
| #6 | kelpie-issue-6 | tla/kelpieLease | Create KelpieLease.tla |
| #7 | kelpie-issue-7 | tla/kelpieMigration | Create KelpieMigration.tla |
| #8 | kelpie-issue-8 | tla/kelpieActorLifecycle | Create KelpieActorLifecycle.tla |
| #9 | kelpie-issue-9 | tla/kelpieFDBTransaction | Create KelpieFDBTransaction.tla |
| #10 | kelpie-issue-10 | tla/kelpieTeleport | Create KelpieTeleport.tla |
| #11 | kelpie-issue-11 | tla/kelpieClusterMembership | Create KelpieClusterMembership.tla |
| #12 | kelpie-issue-12 | tla/singleActivationLiveness | Add liveness to KelpieSingleActivation.tla |
| #13 | kelpie-issue-13 | tla/registryLiveness | Add liveness to KelpieRegistry.tla |
| #14 | kelpie-issue-14 | tla/actorStateFix | Fix RollbackCorrectness in KelpieActorState.tla |
| #15 | kelpie-issue-15 | tla/walLiveness | Add liveness to KelpieWAL.tla |

---

## Launch Commands

Run these in separate iTerm tabs to start parallel Claude agents.

---

### Issue #6: KelpieLease.tla (High Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-6 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #6: Create KelpieLease.tla spec.

CONTEXT:
- ADR-002 requires atomic lease acquisition/renewal (G2.2)
- ADR-004 requires lease-based ownership for single activation (G4.2)

REQUIRED INVARIANTS:
- LeaseUniqueness: At most one valid lease per actor at any time
- RenewalRequiresOwnership: Only lease holder can renew
- ExpiredLeaseClaimable: Expired lease can be claimed by any node
- LeaseValidityBounds: Lease expiry time within configured bounds

DELIVERABLES:
1. Create docs/tla/KelpieLease.tla with Safe and Buggy variants
2. Create docs/tla/KelpieLease.cfg and KelpieLease_Buggy.cfg
3. Add liveness property: EventualLeaseResolution
4. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieLease.cfg KelpieLease.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieLease_Buggy.cfg KelpieLease.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail LeaseUniqueness (find counterexample)
   - Document state count and verification time in README
5. Update docs/tla/README.md with new spec and TLC results
6. Create PR to master with 'Closes #6' in description

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (example structure)
- docs/adr/002-foundationdb-integration.md (G2.2)
- docs/adr/004-linearizability-guarantees.md (G4.2)
- crates/kelpie-registry/src/fdb.rs (lease implementation)"
```

---

### Issue #7: KelpieMigration.tla (High Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-7 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #7: Create KelpieMigration.tla spec.

CONTEXT:
- ADR-004 requires failure recovery (G4.5)
- 3-phase migration: PREPARE → TRANSFER → COMPLETE
- Must handle node failures during any phase

REQUIRED INVARIANTS:
- MigrationAtomicity: Migration complete → full state transferred
- NoStateLoss: No actor state lost during migration
- SingleActivationDuringMigration: At most one active during migration
- MigrationRollback: Failed migration → actor active on source or target

DELIVERABLES:
1. Create docs/tla/KelpieMigration.tla with Safe and Buggy variants
2. Create docs/tla/KelpieMigration.cfg and KelpieMigration_Buggy.cfg
3. Add liveness: EventualMigrationCompletion, EventualRecovery
4. Model crash faults during each phase
5. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieMigration.cfg KelpieMigration.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieMigration_Buggy.cfg KelpieMigration.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail MigrationAtomicity
   - Document state count and verification time
6. Update docs/tla/README.md
7. Create PR to master with 'Closes #7'

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (example)
- docs/adr/004-linearizability-guarantees.md (G4.5)
- crates/kelpie-cluster/src/handler.rs (migration handler)
- crates/kelpie-registry/src/lib.rs (placement management)"
```

---

### Issue #8: KelpieActorLifecycle.tla (Medium Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-8 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #8: Create KelpieActorLifecycle.tla spec.

CONTEXT:
- ADR-001 requires automatic deactivation after idle timeout (G1.5)
- ADR-001 requires lifecycle ordering: activate → invoke → deactivate (G1.3)

REQUIRED INVARIANTS:
- LifecycleOrdering: No invoke without activate, no deactivate during invoke
- IdleTimeoutRespected: Idle > timeout → eventually deactivated
- GracefulDeactivation: Active invocations complete before deactivate

DELIVERABLES:
1. Create docs/tla/KelpieActorLifecycle.tla with Safe and Buggy variants
2. Create config files
3. Add liveness: EventualDeactivation
4. Model idle timer, concurrent invocations
5. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorLifecycle.cfg KelpieActorLifecycle.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorLifecycle_Buggy.cfg KelpieActorLifecycle.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail LifecycleOrdering
   - Document state count and verification time
6. Update docs/tla/README.md
7. Create PR to master with 'Closes #8'

REFERENCE FILES:
- docs/tla/KelpieActorState.tla (related state transitions)
- docs/adr/001-virtual-actor-model.md (G1.3, G1.5)
- crates/kelpie-runtime/src/dispatcher.rs (lifecycle management)"
```

---

### Issue #9: KelpieFDBTransaction.tla (Medium Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-9 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #9: Create KelpieFDBTransaction.tla spec.

CONTEXT:
- ADR-002 requires transaction conflict detection (G2.4)
- ADR-004 requires operations appear atomic (G4.1)
- Currently specs ASSUME FDB atomicity - need to MODEL it

REQUIRED INVARIANTS:
- SerializableIsolation: Concurrent transactions appear serial
- ConflictDetection: Conflicting writes detected and one aborted
- AtomicCommit: Transaction commits atomically or not at all
- ReadYourWrites: Transaction sees its own uncommitted writes

DELIVERABLES:
1. Create docs/tla/KelpieFDBTransaction.tla
2. Model: begin, read, write, commit, abort
3. Model conflict detection and retry
4. Add liveness: EventualCommit (non-conflicting txns commit)
5. Create Safe (correct conflict detection) and Buggy (missing detection) variants
6. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieFDBTransaction.cfg KelpieFDBTransaction.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieFDBTransaction_Buggy.cfg KelpieFDBTransaction.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail ConflictDetection or SerializableIsolation
   - Document state count and verification time
7. Update docs/tla/README.md
8. Create PR to master with 'Closes #9'

REFERENCE FILES:
- docs/adr/002-foundationdb-integration.md (G2.4)
- docs/adr/004-linearizability-guarantees.md (G4.1)
- crates/kelpie-storage/src/fdb.rs (transaction wrapper)"
```

---

### Issue #10: KelpieTeleport.tla (Lower Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-10 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #10: Create KelpieTeleport.tla spec.

CONTEXT:
- ADR-020 requires teleport state consistency (G20.1, G20.2)
- ADR-021 requires architecture validation on restore (G21.1, G21.2)
- Three snapshot types: Suspend, Teleport, Checkpoint

REQUIRED INVARIANTS:
- SnapshotConsistency: Restored state = pre-snapshot state
- ArchitectureValidation: Teleport requires same arch, Checkpoint allows cross-arch
- VersionCompatibility: Base image MAJOR.MINOR must match
- NoPartialRestore: Restore is all-or-nothing

DELIVERABLES:
1. Create docs/tla/KelpieTeleport.tla
2. Model three snapshot types with different constraints
3. Model architecture and version checks
4. Add liveness: EventualRestore (valid snapshot eventually restorable)
5. Create Safe and Buggy variants (Buggy: cross-arch Teleport allowed)
6. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieTeleport.cfg KelpieTeleport.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieTeleport_Buggy.cfg KelpieTeleport.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail ArchitectureValidation
   - Document state count and verification time
7. Update docs/tla/README.md
8. Create PR to master with 'Closes #10'

REFERENCE FILES:
- docs/adr/020-consolidated-vm-crate.md (G20.1, G20.2)
- docs/adr/021-snapshot-type-system.md (G21.1, G21.2)"
```

---

### Issue #11: KelpieClusterMembership.tla (Lower Priority)
```bash
cd /Users/seshendranalla/Development/kelpie-issue-11 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #11: Create KelpieClusterMembership.tla spec.

CONTEXT:
- Cluster membership is designed but not fully implemented
- Need to model node join/leave and membership view consistency
- Foundation for future distributed coordination

REQUIRED INVARIANTS:
- MembershipConsistency: All nodes eventually agree on membership
- JoinAtomicity: Node fully joined or not at all
- LeaveDetection: Failed/leaving node eventually removed from view
- NoSplitBrain: Partitioned nodes do not both think they are primary

DELIVERABLES:
1. Create docs/tla/KelpieClusterMembership.tla
2. Model: join, leave, heartbeat, failure detection
3. Model network partitions
4. Add liveness: EventualMembershipConvergence
5. Create Safe and Buggy variants (Buggy: split-brain possible)
6. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieClusterMembership.cfg KelpieClusterMembership.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieClusterMembership_Buggy.cfg KelpieClusterMembership.tla
   - Safe MUST pass all invariants
   - Buggy MUST fail NoSplitBrain
   - Document state count and verification time
7. Update docs/tla/README.md
8. Create PR to master with 'Closes #11'

REFERENCE FILES:
- crates/kelpie-cluster/src/lib.rs (cluster coordination)
- crates/kelpie-cluster/src/handler.rs (membership handling)
- docs/tla/KelpieRegistry.tla (node state model)"
```

---

### Issue #12: Add Liveness to KelpieSingleActivation.tla
```bash
cd /Users/seshendranalla/Development/kelpie-issue-12 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #12: Add liveness properties to KelpieSingleActivation.tla.

CONTEXT:
- Current spec has only safety properties
- Missing: EventualActivation - claims eventually resolve
- Also need to model FDB transaction semantics explicitly

REQUIRED CHANGES:
1. Add EventualActivation liveness property:
   - Every claim eventually results in activation or rejection
   - Use temporal operators: <>[] (eventually always) or []<> (always eventually)
2. Model FDB transaction semantics explicitly (do not just assume atomicity)
3. Update SPECIFICATION to include liveness
4. Verify with TLC that liveness holds for Safe, potentially violated for Buggy

DELIVERABLES:
1. Update docs/tla/KelpieSingleActivation.tla with liveness
2. Update docs/tla/KelpieSingleActivation.cfg to check liveness
3. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieSingleActivation.cfg KelpieSingleActivation.tla
   - Spec MUST pass all safety AND liveness properties
   - Document state count, verification time, and fairness assumptions
4. Update docs/tla/README.md with new properties
5. Create PR to master with 'Closes #12'

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (current spec)
- docs/tla/README.md (liveness properties needed section)"
```

---

### Issue #13: Add Liveness to KelpieRegistry.tla
```bash
cd /Users/seshendranalla/Development/kelpie-issue-13 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #13: Add liveness properties to KelpieRegistry.tla.

CONTEXT:
- Current spec has only safety properties
- Missing: EventualFailureDetection
- Also missing: node cache model for cache coherence bugs

REQUIRED CHANGES:
1. Add EventualFailureDetection liveness property:
   - Dead nodes eventually detected and removed
2. Add node cache model:
   - Each node has local placement cache
   - Model cache invalidation
   - Add CacheCoherence safety property
3. Update SPECIFICATION to include liveness
4. Verify with TLC

DELIVERABLES:
1. Update docs/tla/KelpieRegistry.tla with liveness and cache model
2. Update config files
3. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieRegistry.cfg KelpieRegistry.tla
   - Spec MUST pass all safety AND liveness properties
   - Document state count, verification time, and fairness assumptions
4. Update docs/tla/README.md
5. Create PR to master with 'Closes #13'

REFERENCE FILES:
- docs/tla/KelpieRegistry.tla (current spec)
- crates/kelpie-registry/src/fdb.rs (cache implementation)
- docs/tla/README.md (fixes needed section)"
```

---

### Issue #14: Fix RollbackCorrectness in KelpieActorState.tla
```bash
cd /Users/seshendranalla/Development/kelpie-issue-14 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #14: Fix RollbackCorrectness invariant in KelpieActorState.tla.

CONTEXT:
- Current RollbackCorrectness invariant returns TRUE unconditionally
- This is a placeholder that needs real implementation
- Must verify: rollback restores pre-invocation state

REQUIRED CHANGES:
1. Implement actual RollbackCorrectness invariant:
   - After rollback, memory state = state before invocation started
   - Buffer is cleared
   - No partial state changes visible
2. Add test case that would catch violations
3. Add liveness property: EventualCommitOrRollback
4. Update Buggy variant to violate RollbackCorrectness

DELIVERABLES:
1. Update docs/tla/KelpieActorState.tla with real RollbackCorrectness
2. Add liveness property
3. Update config files
4. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorState.cfg KelpieActorState.tla
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieActorState_Buggy.cfg KelpieActorState.tla
   - Safe MUST pass RollbackCorrectness
   - Buggy MUST FAIL RollbackCorrectness (find counterexample)
   - Document state count and verification time
5. Update docs/tla/README.md
6. Create PR to master with 'Closes #14'

REFERENCE FILES:
- docs/tla/KelpieActorState.tla (current spec, line with RollbackCorrectness == TRUE)
- docs/adr/008-transaction-api.md (G8.2)
- crates/kelpie-storage/src/lib.rs (rollback implementation)"
```

---

### Issue #15: Add Liveness to KelpieWAL.tla
```bash
cd /Users/seshendranalla/Development/kelpie-issue-15 && claude --model opus --dangerously-skip-permissions "Work on GitHub issue #15: Add liveness properties to KelpieWAL.tla.

CONTEXT:
- Current spec has only safety properties
- Missing: EventualRecovery - pending entries eventually recovered
- Missing: concurrent client model

REQUIRED CHANGES:
1. Add EventualRecovery liveness property:
   - All pending entries eventually processed (completed or failed)
2. Model concurrent clients:
   - Multiple clients appending simultaneously
   - Verify idempotency under concurrency
3. Add EventualCompletion:
   - Started operations eventually complete or fail
4. Update SPECIFICATION to include liveness
5. Verify with TLC

DELIVERABLES:
1. Update docs/tla/KelpieWAL.tla with liveness and concurrent clients
2. Update config files
3. RUN TLC MODEL CHECKER - MANDATORY:
   - Run: java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -config KelpieWAL.cfg KelpieWAL.tla
   - Spec MUST pass all safety AND liveness properties
   - Document state count, verification time, and fairness assumptions
4. Update docs/tla/README.md
5. Create PR to master with 'Closes #15'

REFERENCE FILES:
- docs/tla/KelpieWAL.tla (current spec)
- crates/kelpie-storage/src/wal.rs (WAL implementation)
- docs/tla/README.md (fixes needed section)"
```

---

## TLC Installation

If TLC is not available, install it:

```bash
# Download TLA+ tools
curl -LO https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar
mv tla2tools.jar docs/tla/

# Or via Homebrew
brew install tla-plus-toolbox

# Find existing jar
find /opt/homebrew -name "tla2tools.jar" 2>/dev/null
find /Applications -name "tla2tools.jar" 2>/dev/null
```

---

## Completion Workflow

When an agent completes:
1. Agent runs TLC to verify specs (both Safe and Buggy configs)
2. Agent documents TLC output (state count, time, pass/fail)
3. Agent creates PR with `Closes #<issue>` in description
4. PR includes verification results
5. PR targets `master` branch
6. Human reviews PR for consistency
7. Merge after review

---

## Cleanup After Merge

```bash
# After all PRs merged, remove worktrees
for i in 6 7 8 9 10 11 12 13 14 15; do
  git worktree remove ../kelpie-issue-$i
done

# Delete merged branches
for branch in kelpieLease kelpieMigration kelpieActorLifecycle kelpieFDBTransaction kelpieTeleport kelpieClusterMembership singleActivationLiveness registryLiveness actorStateFix walLiveness; do
  git branch -d tla/$branch
done
```

---

## Resource Considerations

Running 10 Claude agents in parallel requires significant resources:
- Each Claude Code process uses ~200-400MB RAM
- Total: ~2-4GB RAM for all agents
- Network: Parallel API calls to Anthropic

If resources are limited, run in batches:
- **Batch 1 (High Priority)**: Issues #6, #7 (KelpieLease, KelpieMigration)
- **Batch 2 (Medium Priority)**: Issues #8, #9, #12, #14
- **Batch 3 (Lower Priority)**: Issues #10, #11, #13, #15
