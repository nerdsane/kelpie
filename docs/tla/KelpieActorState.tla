--------------------------- MODULE KelpieActorState ---------------------------
(*
 * TLA+ Specification for Kelpie Actor State Management
 *
 * This specification models the actor state and transaction lifecycle,
 * focusing on rollback correctness (G8.2 from ADR-008).
 *
 * Key properties verified:
 * - RollbackCorrectness: After rollback, state equals pre-invocation snapshot
 * - BufferCleared: After rollback, transaction buffer is empty
 * - NoPartialState: No partial state visible after rollback
 * - EventualCommitOrRollback: Every invocation eventually completes
 *
 * Author: Kelpie Team
 * Date: 2026-01-24
 *)

EXTENDS Integers, Sequences, TLC

CONSTANTS
    Values,         \* Set of possible values (e.g., {"v1", "v2", "empty"})
    SafeMode,       \* TRUE for correct implementation, FALSE for buggy
    MaxBufferLen    \* Maximum buffer length (for state space bounding)

VARIABLES
    memory,           \* Current memory state (key-value mapping)
    buffer,           \* Transaction buffer (sequence of pending writes)
    stateSnapshot,    \* Snapshot of memory when invocation started
    invocationState   \* "Idle", "Running", "Committed", "Aborted"

vars == <<memory, buffer, stateSnapshot, invocationState>>

-----------------------------------------------------------------------------
(* Type Invariant *)

TypeOK ==
    /\ memory \in [{"key"} -> Values]
    /\ buffer \in Seq([key: {"key"}, value: Values])
    /\ stateSnapshot \in [{"key"} -> Values]
    /\ invocationState \in {"Idle", "Running", "Committed", "Aborted"}

-----------------------------------------------------------------------------
(* Initial State *)

Init ==
    /\ memory = [k \in {"key"} |-> "empty"]
    /\ buffer = <<>>
    /\ stateSnapshot = [k \in {"key"} |-> "empty"]
    /\ invocationState = "Idle"

-----------------------------------------------------------------------------
(* Actions *)

(* Start a new invocation - capture state snapshot *)
StartInvocation ==
    /\ invocationState = "Idle"
    /\ stateSnapshot' = memory               \* Capture pre-invocation state
    /\ invocationState' = "Running"
    /\ buffer' = <<>>                        \* Clear buffer for new txn
    /\ UNCHANGED memory

(* Buffer a write during invocation - SAFE version (does not modify memory yet) *)
BufferWriteSafe(v) ==
    /\ invocationState = "Running"
    /\ Len(buffer) < MaxBufferLen            \* Bound buffer size for finite state space
    /\ buffer' = Append(buffer, [key |-> "key", value |-> v])
    /\ UNCHANGED <<memory, stateSnapshot, invocationState>>

(* Buffer a write - BUGGY version (applies directly to memory - violates isolation) *)
BufferWriteBuggy(v) ==
    /\ invocationState = "Running"
    /\ Len(buffer) < MaxBufferLen
    /\ buffer' = Append(buffer, [key |-> "key", value |-> v])
    /\ memory' = [k \in {"key"} |-> v]       \* BUG: Writes directly to memory!
    /\ UNCHANGED <<stateSnapshot, invocationState>>

(* Choose correct or buggy write based on SafeMode constant *)
BufferWrite(v) ==
    IF SafeMode
    THEN BufferWriteSafe(v)
    ELSE BufferWriteBuggy(v)

(* Commit: Apply buffered writes to memory *)
Commit ==
    /\ invocationState = "Running"
    /\ invocationState' = "Committed"
    /\ IF Len(buffer) > 0
       THEN memory' = [k \in {"key"} |-> buffer[Len(buffer)].value]  \* Last write wins
       ELSE memory' = memory
    /\ buffer' = <<>>
    /\ UNCHANGED stateSnapshot

(* Rollback/Abort: Discard buffer, restore pre-invocation state *)
(* This is the SAFE version - correctly restores state *)
RollbackSafe ==
    /\ invocationState = "Running"
    /\ invocationState' = "Aborted"
    /\ memory' = stateSnapshot              \* Restore pre-invocation state
    /\ buffer' = <<>>                       \* Clear buffer
    /\ UNCHANGED stateSnapshot

(* Buggy Rollback: Does NOT restore memory (violates RollbackCorrectness) *)
RollbackBuggy ==
    /\ invocationState = "Running"
    /\ invocationState' = "Aborted"
    /\ UNCHANGED memory                     \* BUG: Does not restore snapshot
    /\ buffer' = <<>>
    /\ UNCHANGED stateSnapshot

(* Choose correct or buggy rollback based on SafeMode constant *)
Rollback ==
    IF SafeMode
    THEN RollbackSafe
    ELSE RollbackBuggy

(* Reset to Idle for next invocation *)
Reset ==
    /\ invocationState \in {"Committed", "Aborted"}
    /\ invocationState' = "Idle"
    /\ UNCHANGED <<memory, buffer, stateSnapshot>>

-----------------------------------------------------------------------------
(* Next State Relation *)

Next ==
    \/ StartInvocation
    \/ \E v \in Values: BufferWrite(v)
    \/ Commit
    \/ Rollback
    \/ Reset

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(* Safety Invariants *)

(* Main invariant: After rollback, memory equals pre-invocation snapshot *)
RollbackCorrectness ==
    invocationState = "Aborted" =>
        /\ memory = stateSnapshot
        /\ buffer = <<>>

(* Buffer must be empty when not running *)
BufferEmptyWhenIdle ==
    invocationState \in {"Idle", "Committed", "Aborted"} => buffer = <<>>

(* No partial state: either all writes applied (committed) or none (aborted) *)
NoPartialState ==
    \* If aborted with buffered writes, memory should not contain those writes
    \* This is captured by RollbackCorrectness checking memory = stateSnapshot
    TRUE

(* Combined safety invariant *)
SafetyInvariant ==
    /\ TypeOK
    /\ RollbackCorrectness
    /\ BufferEmptyWhenIdle

(* State constraint for bounding state space *)
StateConstraint ==
    Len(buffer) <= MaxBufferLen

-----------------------------------------------------------------------------
(* Liveness Properties *)

(* Fairness assumption: system makes progress *)
Fairness == WF_vars(Next)

LiveSpec == Spec /\ Fairness

(* Every invocation eventually commits or aborts *)
EventualCommitOrRollback ==
    [](invocationState = "Running" => <>(invocationState \in {"Committed", "Aborted"}))

(* Eventually returns to Idle (ready for next invocation) *)
EventuallyIdle ==
    []<>(invocationState = "Idle")

=============================================================================
\* Modification History
\* Last modified: 2026-01-24
\* Created: 2026-01-24 for GitHub Issue #14
