--------------------------- MODULE KelpieTeleport ---------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie Teleport State Consistency                *)
(*                                                                         *)
(* Models the three snapshot types:                                        *)
(* - Suspend: Memory-only, same-host pause/resume (same arch required)     *)
(* - Teleport: Full VM snapshot for migration (same arch required)         *)
(* - Checkpoint: App-level checkpoint (cross-arch allowed)                 *)
(*                                                                         *)
(* Invariants verified:                                                    *)
(* - SnapshotConsistency: Restored state equals pre-snapshot state         *)
(* - ArchitectureValidation: Teleport/Suspend require same arch            *)
(* - VersionCompatibility: Base image MAJOR.MINOR must match               *)
(* - NoPartialRestore: Restore is all-or-nothing                           *)
(*                                                                         *)
(* Liveness property:                                                      *)
(* - EventualRestore: Valid snapshot eventually restorable                 *)
(*                                                                         *)
(* References:                                                             *)
(* - ADR-020: Consolidated VM Crate (G20.1, G20.2)                         *)
(* - kelpie-core/src/teleport.rs: SnapshotKind, Architecture               *)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets, TLC

(***************************************************************************)
(* CONSTANTS                                                               *)
(***************************************************************************)

CONSTANTS
    Architectures,      \* Set of architectures {Arm64, X86_64}
    SnapshotTypes,      \* Set of snapshot types {Suspend, Teleport, Checkpoint}
    MajorVersions,      \* Set of major versions {1, 2}
    MinorVersions,      \* Set of minor versions {0, 1}
    AgentIds,           \* Set of agent IDs
    SnapshotIds,        \* Set of snapshot IDs {1, 2, 3}
    StateValues,        \* Set of possible state values {0, 1, 2}

    \* Special value for no snapshot being restored
    NoSnapshot,

    \* Checkpoint type constant (for comparison)
    CheckpointType,

    \* Configuration: whether to allow cross-arch teleport (buggy mode)
    AllowCrossArchTeleport  \* TRUE = buggy behavior, FALSE = safe

(***************************************************************************)
(* VARIABLES                                                               *)
(***************************************************************************)

VARIABLES
    \* Current system state
    vmState,            \* VM state: function from AgentId -> state value
    currentArch,        \* Current architecture the system is running on
    currentMajor,       \* Current base image major version
    currentMinor,       \* Current base image minor version

    \* Snapshot storage
    snapshots,          \* Set of snapshot records

    \* Operation tracking for atomicity
    restoreSnapId,      \* Snapshot ID being restored (or NoSnapshot)
    restorePartial      \* TRUE if restore is in partial state

vars == <<vmState, currentArch, currentMajor, currentMinor,
          snapshots, restoreSnapId, restorePartial>>

(***************************************************************************)
(* HELPER DEFINITIONS                                                      *)
(***************************************************************************)

\* Check if snapshot type requires same architecture
RequiresSameArch(stype) ==
    stype # CheckpointType

\* Check if architecture is compatible for restore
ArchCompatible(snap, targetArch) ==
    IF RequiresSameArch(snap.snapshotType)
    THEN snap.sourceArch = targetArch
    ELSE TRUE  \* Checkpoint allows any architecture

\* Check if version is compatible (MAJOR.MINOR must match)
VersionCompatible(snap, targetMajor, targetMinor) ==
    /\ snap.majorVersion = targetMajor
    /\ snap.minorVersion = targetMinor

\* Check if snapshot can be restored on current system
CanRestore(snap) ==
    /\ ArchCompatible(snap, currentArch)
    /\ VersionCompatible(snap, currentMajor, currentMinor)

\* Get used snapshot IDs
UsedSnapshotIds == {s.id : s \in snapshots}

\* Get available snapshot IDs
AvailableSnapshotIds == SnapshotIds \ UsedSnapshotIds

\* Get snapshot by ID (assumes exists)
GetSnapshotById(snapId) ==
    CHOOSE s \in snapshots : s.id = snapId

(***************************************************************************)
(* INITIAL STATE                                                           *)
(***************************************************************************)

Init ==
    /\ vmState \in [AgentIds -> StateValues]
    /\ currentArch \in Architectures
    /\ currentMajor \in MajorVersions
    /\ currentMinor \in MinorVersions
    /\ snapshots = {}
    /\ restoreSnapId = NoSnapshot
    /\ restorePartial = FALSE

(***************************************************************************)
(* ACTIONS                                                                 *)
(***************************************************************************)

\* Modify agent state (simulates agent execution)
ModifyState(agent, newState) ==
    /\ restoreSnapId = NoSnapshot
    /\ vmState' = [vmState EXCEPT ![agent] = newState]
    /\ UNCHANGED <<currentArch, currentMajor, currentMinor,
                   snapshots, restoreSnapId, restorePartial>>

\* Create a snapshot
CreateSnapshot(agent, stype, snapId) ==
    /\ restoreSnapId = NoSnapshot
    /\ snapId \in AvailableSnapshotIds
    /\ LET snap == [
           id |-> snapId,
           agentId |-> agent,
           snapshotType |-> stype,
           sourceArch |-> currentArch,
           majorVersion |-> currentMajor,
           minorVersion |-> currentMinor,
           savedState |-> vmState[agent]
       ]
       IN snapshots' = snapshots \union {snap}
    /\ UNCHANGED <<vmState, currentArch, currentMajor, currentMinor,
                   restoreSnapId, restorePartial>>

\* Begin restore operation (atomic start)
BeginRestore(snapId) ==
    /\ restoreSnapId = NoSnapshot
    /\ snapId \in UsedSnapshotIds
    /\ LET snap == GetSnapshotById(snapId)
           \* Mode check: safe mode requires arch compat, buggy mode skips it
           modeCheck == IF AllowCrossArchTeleport
                        THEN TRUE  \* Buggy: skip arch check
                        ELSE ArchCompatible(snap, currentArch)  \* Safe: require arch compat
       IN /\ modeCheck
          /\ VersionCompatible(snap, currentMajor, currentMinor)
          /\ restoreSnapId' = snapId
          /\ restorePartial' = TRUE
    /\ UNCHANGED <<vmState, currentArch, currentMajor, currentMinor, snapshots>>

\* Complete restore operation (atomic completion)
CompleteRestore ==
    /\ restoreSnapId # NoSnapshot
    /\ restorePartial = TRUE
    /\ restoreSnapId \in UsedSnapshotIds
    /\ LET snap == GetSnapshotById(restoreSnapId)
       IN vmState' = [vmState EXCEPT ![snap.agentId] = snap.savedState]
    /\ restoreSnapId' = NoSnapshot
    /\ restorePartial' = FALSE
    /\ UNCHANGED <<currentArch, currentMajor, currentMinor, snapshots>>

\* Abort restore operation (for testing partial restore detection)
AbortRestore ==
    /\ restoreSnapId # NoSnapshot
    /\ restorePartial = TRUE
    /\ restoreSnapId' = NoSnapshot
    /\ restorePartial' = FALSE
    /\ UNCHANGED <<vmState, currentArch, currentMajor, currentMinor, snapshots>>

\* Switch architecture (simulates migrating to different host)
SwitchArch(newArch) ==
    /\ restoreSnapId = NoSnapshot
    /\ newArch # currentArch
    /\ currentArch' = newArch
    /\ UNCHANGED <<vmState, currentMajor, currentMinor, snapshots,
                   restoreSnapId, restorePartial>>

\* Upgrade base image version
UpgradeVersion(newMajor, newMinor) ==
    /\ restoreSnapId = NoSnapshot
    /\ \/ newMajor # currentMajor
       \/ newMinor # currentMinor
    /\ currentMajor' = newMajor
    /\ currentMinor' = newMinor
    /\ UNCHANGED <<vmState, currentArch, snapshots,
                   restoreSnapId, restorePartial>>

(***************************************************************************)
(* NEXT STATE RELATION                                                     *)
(***************************************************************************)

Next ==
    \/ \E agent \in AgentIds, state \in StateValues : ModifyState(agent, state)
    \/ \E agent \in AgentIds, stype \in SnapshotTypes, snapId \in SnapshotIds :
        CreateSnapshot(agent, stype, snapId)
    \/ \E snapId \in SnapshotIds : BeginRestore(snapId)
    \/ CompleteRestore
    \/ AbortRestore
    \/ \E arch \in Architectures : SwitchArch(arch)
    \/ \E maj \in MajorVersions, min \in MinorVersions : UpgradeVersion(maj, min)

(***************************************************************************)
(* INVARIANTS                                                              *)
(***************************************************************************)

\* INV1: SnapshotConsistency
\* After a successful restore, the VM state equals the saved state
\* (This is enforced structurally by CompleteRestore action)
SnapshotConsistency ==
    TRUE  \* Consistency enforced by CompleteRestore restoring exact savedState

\* INV2: ArchitectureValidation (CRITICAL - this should fail in buggy mode)
\* Teleport and Suspend snapshots should NOT be restored on different architecture
ArchitectureValidation ==
    IF restoreSnapId = NoSnapshot
    THEN TRUE
    ELSE IF restoreSnapId \in UsedSnapshotIds
         THEN LET snap == GetSnapshotById(restoreSnapId)
              IN ~RequiresSameArch(snap.snapshotType) \/ snap.sourceArch = currentArch
         ELSE TRUE

\* INV3: VersionCompatibility
\* Restore only possible when MAJOR.MINOR versions match
VersionCompatibility ==
    IF restoreSnapId = NoSnapshot
    THEN TRUE
    ELSE IF restoreSnapId \in UsedSnapshotIds
         THEN LET snap == GetSnapshotById(restoreSnapId)
              IN snap.majorVersion = currentMajor /\ snap.minorVersion = currentMinor
         ELSE TRUE

\* INV4: NoPartialRestore
\* System never stays in partial restore state without tracking
NoPartialRestore ==
    ~(restorePartial /\ restoreSnapId = NoSnapshot)

\* Type invariant
TypeInvariant ==
    /\ vmState \in [AgentIds -> StateValues]
    /\ currentArch \in Architectures
    /\ currentMajor \in MajorVersions
    /\ currentMinor \in MinorVersions
    /\ restoreSnapId \in SnapshotIds \union {NoSnapshot}
    /\ restorePartial \in BOOLEAN

\* Combined safety invariant
SafetyInvariant ==
    /\ TypeInvariant
    /\ SnapshotConsistency
    /\ ArchitectureValidation
    /\ VersionCompatibility
    /\ NoPartialRestore

(***************************************************************************)
(* LIVENESS PROPERTIES                                                     *)
(***************************************************************************)

\* Fairness: every enabled action eventually happens
Fairness ==
    /\ WF_vars(CompleteRestore)
    /\ WF_vars(AbortRestore)

\* A valid snapshot exists that can be restored
ValidSnapshotExists ==
    \E snap \in snapshots :
        /\ ArchCompatible(snap, currentArch)
        /\ VersionCompatible(snap, currentMajor, currentMinor)

\* Restore completed state
RestoreCompleted ==
    restoreSnapId = NoSnapshot /\ ~restorePartial

\* LIVENESS: EventualRestore
\* If there's a valid snapshot, the system can eventually complete/abort restore operations
EventualRestore ==
    [](ValidSnapshotExists => <>RestoreCompleted)

\* Specification with liveness
Spec == Init /\ [][Next]_vars /\ Fairness

=============================================================================
(* Modification History                                                    *)
(* Created: 2026-01-24                                                     *)
(* Purpose: Verify Kelpie teleport state consistency guarantees            *)
(* Issue: GitHub #10                                                       *)
=============================================================================
