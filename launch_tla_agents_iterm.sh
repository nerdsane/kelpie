#!/bin/bash
# Launch 10 Claude agents in iTerm2 tabs with --dangerously-skip-permissions

osascript <<'EOF'
tell application "iTerm"
    activate

    -- Create new window with first tab
    set newWindow to (create window with default profile)

    tell current session of newWindow
        write text "cd /Users/seshendranalla/Development/kelpie-issue-6 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #6: Create KelpieLease.tla spec.

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
4. Verify: Safe passes, Buggy fails LeaseUniqueness
5. Update docs/tla/README.md with new spec
6. Create PR to master with Closes #6 in description

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (example structure)
- docs/adr/002-foundationdb-integration.md (G2.2)
- docs/adr/004-linearizability-guarantees.md (G4.2)
- crates/kelpie-registry/src/fdb.rs (lease implementation)\""
    end tell

    -- Tab 2: Issue #7
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-7 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #7: Create KelpieMigration.tla spec.

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
5. Verify: Safe passes, Buggy fails MigrationAtomicity
6. Update docs/tla/README.md
7. Create PR to master with Closes #7

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (example)
- docs/adr/004-linearizability-guarantees.md (G4.5)
- crates/kelpie-cluster/src/handler.rs (migration handler)
- crates/kelpie-registry/src/lib.rs (placement management)\""
        end tell
    end tell

    -- Tab 3: Issue #8
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-8 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #8: Create KelpieActorLifecycle.tla spec.

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
5. Verify: Safe passes, Buggy fails LifecycleOrdering
6. Update docs/tla/README.md
7. Create PR to master with Closes #8

REFERENCE FILES:
- docs/tla/KelpieActorState.tla (related state transitions)
- docs/adr/001-virtual-actor-model.md (G1.3, G1.5)
- crates/kelpie-runtime/src/dispatcher.rs (lifecycle management)\""
        end tell
    end tell

    -- Tab 4: Issue #9
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-9 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #9: Create KelpieFDBTransaction.tla spec.

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
6. Update docs/tla/README.md
7. Create PR to master with Closes #9

REFERENCE FILES:
- docs/adr/002-foundationdb-integration.md (G2.4)
- docs/adr/004-linearizability-guarantees.md (G4.1)
- crates/kelpie-storage/src/fdb.rs (transaction wrapper)\""
        end tell
    end tell

    -- Tab 5: Issue #10
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-10 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #10: Create KelpieTeleport.tla spec.

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
6. Update docs/tla/README.md
7. Create PR to master with Closes #10

REFERENCE FILES:
- docs/adr/020-consolidated-vm-crate.md (G20.1, G20.2)
- docs/adr/021-snapshot-type-system.md (G21.1, G21.2)\""
        end tell
    end tell

    -- Tab 6: Issue #11
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-11 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #11: Create KelpieClusterMembership.tla spec.

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
6. Update docs/tla/README.md
7. Create PR to master with Closes #11

REFERENCE FILES:
- crates/kelpie-cluster/src/lib.rs (cluster coordination)
- crates/kelpie-cluster/src/handler.rs (membership handling)
- docs/tla/KelpieRegistry.tla (node state model)\""
        end tell
    end tell

    -- Tab 7: Issue #12
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-12 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #12: Add liveness properties to KelpieSingleActivation.tla.

CONTEXT:
- Current spec has only safety properties
- Missing: EventualActivation - claims eventually resolve
- Also need to model FDB transaction semantics explicitly

REQUIRED CHANGES:
1. Add EventualActivation liveness property:
   - Every claim eventually results in activation or rejection
   - Use <>[] (eventually always) or []<> (always eventually)
2. Model FDB transaction semantics explicitly (do not just assume atomicity)
3. Update SPECIFICATION to include liveness
4. Verify with TLC that liveness holds for Safe, potentially violated for Buggy

DELIVERABLES:
1. Update docs/tla/KelpieSingleActivation.tla with liveness
2. Update docs/tla/KelpieSingleActivation.cfg to check liveness
3. Run TLC and document state count
4. Update docs/tla/README.md with new properties
5. Create PR to master with Closes #12

REFERENCE FILES:
- docs/tla/KelpieSingleActivation.tla (current spec)
- docs/tla/README.md (liveness properties needed section)\""
        end tell
    end tell

    -- Tab 8: Issue #13
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-13 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #13: Add liveness properties to KelpieRegistry.tla.

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
3. Run TLC and document state count
4. Update docs/tla/README.md
5. Create PR to master with Closes #13

REFERENCE FILES:
- docs/tla/KelpieRegistry.tla (current spec)
- crates/kelpie-registry/src/fdb.rs (cache implementation)
- docs/tla/README.md (fixes needed section)\""
        end tell
    end tell

    -- Tab 9: Issue #14
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-14 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #14: Fix RollbackCorrectness invariant in KelpieActorState.tla.

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
4. Run TLC and verify Safe passes, Buggy fails
5. Update docs/tla/README.md
6. Create PR to master with Closes #14

REFERENCE FILES:
- docs/tla/KelpieActorState.tla (current spec, line with RollbackCorrectness == TRUE)
- docs/adr/008-transaction-api.md (G8.2)
- crates/kelpie-storage/src/lib.rs (rollback implementation)\""
        end tell
    end tell

    -- Tab 10: Issue #15
    tell current window
        set newTab to (create tab with default profile)
        tell current session of newTab
            write text "cd /Users/seshendranalla/Development/kelpie-issue-15 && claude --model opus --dangerously-skip-permissions \"Work on GitHub issue #15: Add liveness properties to KelpieWAL.tla.

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
3. Run TLC and document state count
4. Update docs/tla/README.md
5. Create PR to master with Closes #15

REFERENCE FILES:
- docs/tla/KelpieWAL.tla (current spec)
- crates/kelpie-storage/src/wal.rs (WAL implementation)
- docs/tla/README.md (fixes needed section)\""
        end tell
    end tell

end tell
EOF

echo "All 10 Claude agents launched in iTerm2 tabs with --dangerously-skip-permissions!"
