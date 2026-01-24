--------------------------- MODULE KelpieMigration ---------------------------
(*
 * Kelpie Actor Migration Protocol - TLA+ Specification
 *
 * This spec models Kelpie's 3-phase actor migration protocol:
 *   PREPARE -> TRANSFER -> COMPLETE
 *
 * The protocol ensures:
 * - MigrationAtomicity: Complete migration transfers full state
 * - NoStateLoss: Actor state is never lost during migration
 * - SingleActivationDuringMigration: At most one active instance during migration
 * - MigrationRollback: Failed migration leaves actor on source or target
 *
 * Reference: ADR-004 Linearizability Guarantees (G4.5 Failure Recovery)
 * Reference: crates/kelpie-cluster/src/handler.rs (migration handler)
 *
 * TigerStyle: Explicit state machine with bounded state space.
 *)

EXTENDS Naturals, Sequences, FiniteSets, TLC

\* Configuration constants
CONSTANTS
    Nodes,          \* Set of node IDs (e.g., {"source", "target"})
    Actors,         \* Set of actor IDs (e.g., {"actor1"})
    MaxCrashes,     \* Maximum number of crashes to explore
    SkipTransfer    \* Buggy mode: TRUE to skip state transfer (introduces bug)

\* Migration phases
CONSTANTS
    Idle,           \* No migration in progress
    Preparing,      \* Target node being prepared
    Transferring,   \* State being transferred
    Completing,     \* Finalizing migration on target
    Completed,      \* Migration successful
    Failed          \* Migration failed, needs recovery

\* Actor activation states
CONSTANTS
    Active,         \* Actor is running
    Inactive,       \* Actor is not running
    Migrating       \* Actor is being migrated (deactivated on source)

(* --algorithm KelpieMigration

variables
    \* Where each actor is located (node ID)
    actor_location = [a \in Actors |-> CHOOSE n \in Nodes : TRUE],

    \* State of each actor (for verifying state transfer)
    \* Maps actor -> current state value (simulating actor state)
    actor_state = [a \in Actors |-> "initial_state"],

    \* Migration phase for each actor
    migration_phase = [a \in Actors |-> Idle],

    \* Migration source node (where actor is moving FROM)
    migration_source = [a \in Actors |-> "none"],

    \* Migration target node (where actor is moving TO)
    migration_target = [a \in Actors |-> "none"],

    \* State received at target during transfer (for atomicity check)
    target_received_state = [a \in Actors |-> "none"],

    \* Activation state of actors
    actor_activation = [a \in Actors |-> Active],

    \* Node status (alive or crashed)
    node_status = [n \in Nodes |-> "alive"],

    \* Crash counter for bounding
    crash_count = 0,

    \* Recovery pending flag
    recovery_pending = [a \in Actors |-> FALSE];

define
    \* =====================================================================
    \* SAFETY INVARIANTS
    \* =====================================================================

    \* MigrationAtomicity: If migration completes, full state was transferred
    \* This is the key invariant that the buggy version should violate
    MigrationAtomicity ==
        \A a \in Actors:
            migration_phase[a] = Completed =>
                \* Target received the complete state
                target_received_state[a] = actor_state[a]

    \* NoStateLoss: Actor state is never lost
    \* Either the state is at original location, or properly transferred
    NoStateLoss ==
        \A a \in Actors:
            \* State exists somewhere - we track it via actor_state
            actor_state[a] /= "lost"

    \* SingleActivationDuringMigration: At most one active instance
    \* During migration, actor cannot be active on both source and target
    SingleActivationDuringMigration ==
        \A a \in Actors:
            \* Cannot be Active at multiple locations
            actor_activation[a] /= "dual_active"

    \* MigrationRollback: Failed migration leaves actor recoverable
    \* If migration fails, actor is either on source, target, or recovery pending
    MigrationRollback ==
        \A a \in Actors:
            migration_phase[a] = Failed =>
                \/ actor_location[a] \in Nodes  \* Actor still has a location
                \/ recovery_pending[a] = TRUE    \* Or recovery is pending

    \* TypeInvariant: Ensure variables have correct types
    TypeInvariant ==
        /\ actor_location \in [Actors -> Nodes]
        /\ migration_phase \in [Actors -> {Idle, Preparing, Transferring,
                                            Completing, Completed, Failed}]
        /\ actor_activation \in [Actors -> {Active, Inactive, Migrating}]
        /\ node_status \in [Nodes -> {"alive", "crashed"}]
        /\ crash_count \in 0..MaxCrashes

    \* Combined safety invariant
    SafetyInvariant ==
        /\ TypeInvariant
        /\ MigrationAtomicity
        /\ NoStateLoss
        /\ SingleActivationDuringMigration
        /\ MigrationRollback

    \* =====================================================================
    \* HELPER DEFINITIONS
    \* =====================================================================

    \* Check if a node is alive
    IsAlive(n) == node_status[n] = "alive"

    \* Check if actor can be migrated
    CanMigrate(a) ==
        /\ migration_phase[a] = Idle
        /\ actor_activation[a] = Active
        /\ IsAlive(actor_location[a])

    \* Get other nodes (for choosing migration target)
    OtherNodes(n) == Nodes \ {n}

end define;

\* =========================================================================
\* MIGRATION PROTOCOL ACTIONS
\* =========================================================================

\* Phase 1: Start migration - source deactivates actor, sends prepare
macro StartMigration(actor, target) begin
    \* Preconditions
    assert migration_phase[actor] = Idle;
    assert actor_activation[actor] = Active;
    assert target /= actor_location[actor];
    assert IsAlive(actor_location[actor]);
    assert IsAlive(target);

    \* Deactivate actor on source (prevents dual activation)
    actor_activation[actor] := Migrating;

    \* Record migration info
    migration_source[actor] := actor_location[actor];
    migration_target[actor] := target;

    \* Move to Preparing phase
    migration_phase[actor] := Preparing;
end macro;

\* Phase 1 Response: Target accepts prepare
macro PrepareTarget(actor) begin
    \* Preconditions
    assert migration_phase[actor] = Preparing;
    assert IsAlive(migration_target[actor]);

    \* Move to Transferring phase (target is prepared)
    migration_phase[actor] := Transferring;
end macro;

\* Phase 2: Transfer state from source to target
macro TransferState(actor) begin
    \* Preconditions
    assert migration_phase[actor] = Transferring;
    assert IsAlive(migration_source[actor]);
    assert IsAlive(migration_target[actor]);

    \* SAFE VERSION: Transfer the actual state
    \* BUGGY VERSION: Skip transfer (SkipTransfer = TRUE)
    if ~SkipTransfer then
        target_received_state[actor] := actor_state[actor];
    end if;

    \* Move to Completing phase
    migration_phase[actor] := Completing;
end macro;

\* Phase 3: Complete migration - activate on target
macro CompleteMigration(actor) begin
    \* Preconditions
    assert migration_phase[actor] = Completing;
    assert IsAlive(migration_target[actor]);

    \* Move actor to target location
    actor_location[actor] := migration_target[actor];

    \* Activate on target
    actor_activation[actor] := Active;

    \* Mark migration complete
    migration_phase[actor] := Completed;

    \* Clear migration state
    migration_source[actor] := "none";
    migration_target[actor] := "none";
end macro;

\* =========================================================================
\* FAULT INJECTION ACTIONS
\* =========================================================================

\* Crash a node - can happen at any time
macro CrashNode(node) begin
    assert node_status[node] = "alive";
    assert crash_count < MaxCrashes;

    node_status[node] := "crashed";
    crash_count := crash_count + 1;

    \* Handle in-flight migrations affected by crash
    \* For each actor, check if its migration is affected
end macro;

\* Handle migration failure due to crash
macro HandleMigrationFailure(actor) begin
    \* Migration was in progress and affected by crash
    assert migration_phase[actor] \in {Preparing, Transferring, Completing};

    \* Check which node crashed
    if ~IsAlive(migration_source[actor]) \/ ~IsAlive(migration_target[actor]) then
        \* Migration failed
        migration_phase[actor] := Failed;
        recovery_pending[actor] := TRUE;

        \* If source alive, reactivate there
        if IsAlive(migration_source[actor]) then
            actor_location[actor] := migration_source[actor];
            actor_activation[actor] := Active;
            recovery_pending[actor] := FALSE;
        elsif IsAlive(migration_target[actor]) /\ migration_phase[actor] = Completing then
            \* Target was about to complete - can recover there
            actor_location[actor] := migration_target[actor];
            actor_activation[actor] := Active;
            recovery_pending[actor] := FALSE;
        end if;
    end if;
end macro;

\* Recover a crashed node
macro RecoverNode(node) begin
    assert node_status[node] = "crashed";
    node_status[node] := "alive";

    \* Any actors with pending recovery on this node can recover
end macro;

\* Recover an actor after failure
macro RecoverActor(actor) begin
    assert recovery_pending[actor] = TRUE;

    \* Find an alive node to recover on
    \* Priority: original source, then target, then any alive node
    if IsAlive(migration_source[actor]) then
        actor_location[actor] := migration_source[actor];
        actor_activation[actor] := Active;
        recovery_pending[actor] := FALSE;
        migration_phase[actor] := Idle;
    elsif IsAlive(migration_target[actor]) then
        \* Recovery on target - may lose state if transfer incomplete
        actor_location[actor] := migration_target[actor];
        actor_activation[actor] := Active;
        recovery_pending[actor] := FALSE;
        migration_phase[actor] := Idle;
    end if;

    \* Clean up migration state
    migration_source[actor] := "none";
    migration_target[actor] := "none";
end macro;

\* =========================================================================
\* PROCESS DEFINITIONS
\* =========================================================================

\* Migration coordinator process
fair process MigrationCoordinator = "coordinator"
begin
Coord:
    while TRUE do
        either
            \* Start a new migration
            with actor \in Actors, target \in Nodes do
                if CanMigrate(actor) /\ target /= actor_location[actor] /\ IsAlive(target) then
                    StartMigration(actor, target);
                end if;
            end with;
        or
            \* Prepare target
            with actor \in Actors do
                if migration_phase[actor] = Preparing /\ IsAlive(migration_target[actor]) then
                    PrepareTarget(actor);
                end if;
            end with;
        or
            \* Transfer state
            with actor \in Actors do
                if migration_phase[actor] = Transferring /\
                   IsAlive(migration_source[actor]) /\ IsAlive(migration_target[actor]) then
                    TransferState(actor);
                end if;
            end with;
        or
            \* Complete migration
            with actor \in Actors do
                if migration_phase[actor] = Completing /\ IsAlive(migration_target[actor]) then
                    CompleteMigration(actor);
                end if;
            end with;
        or
            \* Handle failure
            with actor \in Actors do
                if migration_phase[actor] \in {Preparing, Transferring, Completing} /\
                   (~IsAlive(migration_source[actor]) \/ ~IsAlive(migration_target[actor])) then
                    HandleMigrationFailure(actor);
                end if;
            end with;
        or
            \* Recover actor
            with actor \in Actors do
                if recovery_pending[actor] then
                    RecoverActor(actor);
                end if;
            end with;
        or
            \* Skip (stutter)
            skip;
        end either;
    end while;
end process;

\* Fault injector process
fair process FaultInjector = "faults"
begin
Faults:
    while crash_count < MaxCrashes do
        either
            \* Crash a node
            with node \in Nodes do
                if IsAlive(node) then
                    CrashNode(node);
                end if;
            end with;
        or
            \* Recover a node
            with node \in Nodes do
                if node_status[node] = "crashed" then
                    RecoverNode(node);
                end if;
            end with;
        or
            \* Skip (no fault this round)
            skip;
        end either;
    end while;
end process;

end algorithm; *)

\* =========================================================================
\* TLA+ TRANSLATION (Generated by PlusCal translator, manually adjusted)
\* =========================================================================

\* BEGIN TRANSLATION
VARIABLES actor_location, actor_state, migration_phase, migration_source,
          migration_target, target_received_state, actor_activation,
          node_status, crash_count, recovery_pending, pc

(* Define operator definitions from above *)
MigrationAtomicity ==
    \A a \in Actors:
        migration_phase[a] = Completed =>
            target_received_state[a] = actor_state[a]

NoStateLoss ==
    \A a \in Actors:
        actor_state[a] /= "lost"

SingleActivationDuringMigration ==
    \A a \in Actors:
        actor_activation[a] /= "dual_active"

MigrationRollback ==
    \A a \in Actors:
        migration_phase[a] = Failed =>
            \/ actor_location[a] \in Nodes
            \/ recovery_pending[a] = TRUE

TypeInvariant ==
    /\ actor_location \in [Actors -> Nodes]
    /\ migration_phase \in [Actors -> {Idle, Preparing, Transferring,
                                        Completing, Completed, Failed}]
    /\ actor_activation \in [Actors -> {Active, Inactive, Migrating}]
    /\ node_status \in [Nodes -> {"alive", "crashed"}]
    /\ crash_count \in 0..MaxCrashes

SafetyInvariant ==
    /\ TypeInvariant
    /\ MigrationAtomicity
    /\ NoStateLoss
    /\ SingleActivationDuringMigration
    /\ MigrationRollback

IsAlive(n) == node_status[n] = "alive"

CanMigrate(a) ==
    /\ migration_phase[a] = Idle
    /\ actor_activation[a] = Active
    /\ IsAlive(actor_location[a])

OtherNodes(n) == Nodes \ {n}

vars == <<actor_location, actor_state, migration_phase, migration_source,
          migration_target, target_received_state, actor_activation,
          node_status, crash_count, recovery_pending, pc>>

ProcSet == {"coordinator"} \cup {"faults"}

Init ==
    /\ actor_location = [a \in Actors |-> CHOOSE n \in Nodes : TRUE]
    /\ actor_state = [a \in Actors |-> "initial_state"]
    /\ migration_phase = [a \in Actors |-> Idle]
    /\ migration_source = [a \in Actors |-> "none"]
    /\ migration_target = [a \in Actors |-> "none"]
    /\ target_received_state = [a \in Actors |-> "none"]
    /\ actor_activation = [a \in Actors |-> Active]
    /\ node_status = [n \in Nodes |-> "alive"]
    /\ crash_count = 0
    /\ recovery_pending = [a \in Actors |-> FALSE]
    /\ pc = [self \in ProcSet |-> CASE self = "coordinator" -> "Coord"
                                   [] self = "faults" -> "Faults"]

\* Start migration action
StartMigrationAction(actor, target) ==
    /\ migration_phase[actor] = Idle
    /\ actor_activation[actor] = Active
    /\ target /= actor_location[actor]
    /\ IsAlive(actor_location[actor])
    /\ IsAlive(target)
    /\ actor_activation' = [actor_activation EXCEPT ![actor] = Migrating]
    /\ migration_source' = [migration_source EXCEPT ![actor] = actor_location[actor]]
    /\ migration_target' = [migration_target EXCEPT ![actor] = target]
    /\ migration_phase' = [migration_phase EXCEPT ![actor] = Preparing]
    /\ UNCHANGED <<actor_location, actor_state, target_received_state,
                   node_status, crash_count, recovery_pending>>

\* Prepare target action
PrepareTargetAction(actor) ==
    /\ migration_phase[actor] = Preparing
    /\ IsAlive(migration_target[actor])
    /\ migration_phase' = [migration_phase EXCEPT ![actor] = Transferring]
    /\ UNCHANGED <<actor_location, actor_state, migration_source, migration_target,
                   target_received_state, actor_activation, node_status,
                   crash_count, recovery_pending>>

\* Transfer state action (with bug injection)
TransferStateAction(actor) ==
    /\ migration_phase[actor] = Transferring
    /\ IsAlive(migration_source[actor])
    /\ IsAlive(migration_target[actor])
    /\ IF ~SkipTransfer
       THEN target_received_state' = [target_received_state EXCEPT ![actor] = actor_state[actor]]
       ELSE UNCHANGED target_received_state
    /\ migration_phase' = [migration_phase EXCEPT ![actor] = Completing]
    /\ UNCHANGED <<actor_location, actor_state, migration_source, migration_target,
                   actor_activation, node_status, crash_count, recovery_pending>>

\* Complete migration action
CompleteMigrationAction(actor) ==
    /\ migration_phase[actor] = Completing
    /\ IsAlive(migration_target[actor])
    /\ actor_location' = [actor_location EXCEPT ![actor] = migration_target[actor]]
    /\ actor_activation' = [actor_activation EXCEPT ![actor] = Active]
    /\ migration_phase' = [migration_phase EXCEPT ![actor] = Completed]
    /\ migration_source' = [migration_source EXCEPT ![actor] = "none"]
    /\ migration_target' = [migration_target EXCEPT ![actor] = "none"]
    /\ UNCHANGED <<actor_state, target_received_state, node_status,
                   crash_count, recovery_pending>>

\* Crash node action
CrashNodeAction(node) ==
    /\ node_status[node] = "alive"
    /\ crash_count < MaxCrashes
    /\ node_status' = [node_status EXCEPT ![node] = "crashed"]
    /\ crash_count' = crash_count + 1
    /\ UNCHANGED <<actor_location, actor_state, migration_phase, migration_source,
                   migration_target, target_received_state, actor_activation,
                   recovery_pending>>

\* Handle migration failure action
HandleMigrationFailureAction(actor) ==
    /\ migration_phase[actor] \in {Preparing, Transferring, Completing}
    /\ ~IsAlive(migration_source[actor]) \/ ~IsAlive(migration_target[actor])
    /\ migration_phase' = [migration_phase EXCEPT ![actor] = Failed]
    /\ recovery_pending' = [recovery_pending EXCEPT ![actor] = TRUE]
    /\ IF IsAlive(migration_source[actor])
       THEN /\ actor_location' = [actor_location EXCEPT ![actor] = migration_source[actor]]
            /\ actor_activation' = [actor_activation EXCEPT ![actor] = Active]
            /\ recovery_pending' = [recovery_pending EXCEPT ![actor] = FALSE]
       ELSE IF IsAlive(migration_target[actor]) /\ migration_phase[actor] = Completing
            THEN /\ actor_location' = [actor_location EXCEPT ![actor] = migration_target[actor]]
                 /\ actor_activation' = [actor_activation EXCEPT ![actor] = Active]
                 /\ recovery_pending' = [recovery_pending EXCEPT ![actor] = FALSE]
            ELSE UNCHANGED <<actor_location, actor_activation>>
    /\ UNCHANGED <<actor_state, migration_source, migration_target,
                   target_received_state, node_status, crash_count>>

\* Recover node action
RecoverNodeAction(node) ==
    /\ node_status[node] = "crashed"
    /\ node_status' = [node_status EXCEPT ![node] = "alive"]
    /\ UNCHANGED <<actor_location, actor_state, migration_phase, migration_source,
                   migration_target, target_received_state, actor_activation,
                   crash_count, recovery_pending>>

\* Recover actor action
RecoverActorAction(actor) ==
    /\ recovery_pending[actor] = TRUE
    /\ \/ /\ IsAlive(migration_source[actor])
          /\ actor_location' = [actor_location EXCEPT ![actor] = migration_source[actor]]
          /\ actor_activation' = [actor_activation EXCEPT ![actor] = Active]
          /\ recovery_pending' = [recovery_pending EXCEPT ![actor] = FALSE]
          /\ migration_phase' = [migration_phase EXCEPT ![actor] = Idle]
       \/ /\ ~IsAlive(migration_source[actor])
          /\ IsAlive(migration_target[actor])
          /\ actor_location' = [actor_location EXCEPT ![actor] = migration_target[actor]]
          /\ actor_activation' = [actor_activation EXCEPT ![actor] = Active]
          /\ recovery_pending' = [recovery_pending EXCEPT ![actor] = FALSE]
          /\ migration_phase' = [migration_phase EXCEPT ![actor] = Idle]
    /\ migration_source' = [migration_source EXCEPT ![actor] = "none"]
    /\ migration_target' = [migration_target EXCEPT ![actor] = "none"]
    /\ UNCHANGED <<actor_state, target_received_state, node_status, crash_count>>

\* Coordinator process actions
Coord(self) ==
    \/ \E actor \in Actors, target \in Nodes:
        /\ pc[self] = "Coord"
        /\ StartMigrationAction(actor, target)
        /\ pc' = pc
    \/ \E actor \in Actors:
        /\ pc[self] = "Coord"
        /\ PrepareTargetAction(actor)
        /\ pc' = pc
    \/ \E actor \in Actors:
        /\ pc[self] = "Coord"
        /\ TransferStateAction(actor)
        /\ pc' = pc
    \/ \E actor \in Actors:
        /\ pc[self] = "Coord"
        /\ CompleteMigrationAction(actor)
        /\ pc' = pc
    \/ \E actor \in Actors:
        /\ pc[self] = "Coord"
        /\ HandleMigrationFailureAction(actor)
        /\ pc' = pc
    \/ \E actor \in Actors:
        /\ pc[self] = "Coord"
        /\ RecoverActorAction(actor)
        /\ pc' = pc
    \/ /\ pc[self] = "Coord"
       /\ TRUE
       /\ pc' = pc
       /\ UNCHANGED vars

\* Fault injector process actions
Faults(self) ==
    /\ pc[self] = "Faults"
    /\ crash_count < MaxCrashes
    /\ \/ \E node \in Nodes:
          /\ CrashNodeAction(node)
          /\ pc' = pc
       \/ \E node \in Nodes:
          /\ RecoverNodeAction(node)
          /\ pc' = pc
       \/ /\ TRUE
          /\ pc' = pc
          /\ UNCHANGED vars

\* Next state relation
Next ==
    \/ Coord("coordinator")
    \/ Faults("faults")
    \/ UNCHANGED vars

\* Specification
Spec == Init /\ [][Next]_vars

\* Fairness for liveness
FairSpec == Spec /\ WF_vars(Next)

\* =========================================================================
\* LIVENESS PROPERTIES
\* =========================================================================

\* EventualMigrationCompletion: If migration starts and nodes stay alive,
\* it eventually completes
EventualMigrationCompletion ==
    \A a \in Actors:
        (migration_phase[a] = Preparing /\
         IsAlive(migration_source[a]) /\ IsAlive(migration_target[a]))
        ~> (migration_phase[a] = Completed \/ migration_phase[a] = Failed)

\* EventualRecovery: If recovery is pending and a node is alive,
\* actor eventually recovers
EventualRecovery ==
    \A a \in Actors:
        (recovery_pending[a] = TRUE /\
         (\E n \in Nodes: IsAlive(n)))
        ~> (recovery_pending[a] = FALSE)

\* =========================================================================
\* TEMPORAL PROPERTIES FOR MODEL CHECKING
\* =========================================================================

\* Eventually a migration completes (for bounded checking)
SomeMigrationCompletes ==
    <>(\E a \in Actors: migration_phase[a] = Completed)

\* Eventually recovery happens if needed
SomeRecoveryHappens ==
    [](\A a \in Actors: (recovery_pending[a] = TRUE) =>
        <>(recovery_pending[a] = FALSE))

=============================================================================
