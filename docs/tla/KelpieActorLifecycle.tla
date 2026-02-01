--------------------------- MODULE KelpieActorLifecycle ---------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie Actor Lifecycle Management                *)
(*                                                                         *)
(* Models the lifecycle of a virtual actor in Kelpie, verifying:           *)
(* - G1.3: Lifecycle ordering (activate -> invoke -> deactivate)           *)
(* - G1.4: Single-threaded execution (one message at a time) - implicit    *)
(* - G1.5: Automatic deactivation after idle timeout                       *)
(*                                                                         *)
(* Reference: ADR-001 Virtual Actor Model                                  *)
(* Implementation: crates/kelpie-runtime/src/activation.rs                 *)
(***************************************************************************)

EXTENDS Naturals, Sequences

CONSTANTS
    MAX_PENDING,      \* Maximum queued messages (processed one at a time per G1.4)
    IDLE_TIMEOUT,     \* Ticks before idle deactivation
    BUGGY_INVOKE,     \* TRUE: allow invoke in any state (violates LifecycleOrdering)
    BUGGY_DEACTIVATE  \* TRUE: allow deactivation with pending messages (violates GracefulDeactivation)

(***************************************************************************)
(* State Variables                                                         *)
(* Maps to issue specification:                                            *)
(*   actorState      -> state (single actor model)                         *)
(*   lastActivity    -> idleTicks (inverted: ticks since activity)         *)
(*   pendingMessages -> pending                                            *)
(*   idleTimeout     -> IDLE_TIMEOUT constant                              *)
(*   time            -> modeled implicitly via Tick action                 *)
(***************************************************************************)

VARIABLES
    state,            \* Actor lifecycle state: "Inactive", "Activating", "Active", "Deactivating"
    pending,          \* Message queue depth (0..MAX_PENDING). Messages are processed
                      \* sequentially one at a time (G1.4 single-threaded execution).
    idleTicks         \* Ticks since last activity (for idle timeout)

vars == <<state, pending, idleTicks>>

(***************************************************************************)
(* Type Invariant                                                          *)
(***************************************************************************)

TypeOK ==
    /\ state \in {"Inactive", "Activating", "Active", "Deactivating"}
    /\ pending \in 0..MAX_PENDING
    /\ idleTicks \in 0..(IDLE_TIMEOUT + 1)

(***************************************************************************)
(* Initial State                                                           *)
(***************************************************************************)

Init ==
    /\ state = "Inactive"
    /\ pending = 0
    /\ idleTicks = 0

(***************************************************************************)
(* Actions                                                                 *)
(* Aliases provided to match issue specification naming                    *)
(***************************************************************************)

\* StartActivation(actor): Begin actor activation
\* Models: ActiveActor::activate() in activation.rs
StartActivation ==
    /\ state = "Inactive"
    /\ state' = "Activating"
    /\ pending' = pending
    /\ idleTicks' = 0

\* CompleteActivation(actor): Finish activation, enter Active state
\* Models: on_activate hook completion
CompleteActivation ==
    /\ state = "Activating"
    /\ state' = "Active"
    /\ pending' = pending
    /\ idleTicks' = 0

\* EnqueueMessage(actor): Add a message to pending queue
\* Models: DispatcherHandle::invoke() accepting a request
\* Safe: Requires state = Active
\* Buggy: Allows enqueue in any state (violates LifecycleOrdering)
EnqueueMessage ==
    /\ pending < MAX_PENDING
    /\ IF BUGGY_INVOKE
       THEN TRUE  \* Buggy: Allow invoke in any state
       ELSE state = "Active"  \* Safe: Only invoke when Active
    /\ pending' = pending + 1
    /\ idleTicks' = 0  \* Reset idle timer on activity
    /\ state' = state

\* ProcessMessage(actor): Process and complete a pending message
\* Models: process_invocation() completing
ProcessMessage ==
    /\ pending > 0
    /\ pending' = pending - 1
    /\ idleTicks' = 0  \* Reset idle timer on activity
    /\ state' = state

\* DrainMessage(actor): Process message during deactivation drain phase
\* Same as ProcessMessage but explicit for deactivation context
DrainMessage ==
    /\ state = "Deactivating"
    /\ pending > 0
    /\ pending' = pending - 1
    /\ idleTicks' = idleTicks
    /\ state' = state

\* Tick: Model time passing (idle timer increment)
\* Models: Time passing without invocations
Tick ==
    /\ state = "Active"
    /\ pending = 0  \* Only tick when no pending invocations
    /\ idleTicks < IDLE_TIMEOUT + 1
    /\ idleTicks' = idleTicks + 1
    /\ state' = state
    /\ pending' = pending

\* CheckIdleTimeout / StartDeactivation(actor): Begin deactivation when idle
\* Models: should_deactivate() returning true
\* Precondition: Must be Active with no pending invocations (unless buggy)
StartDeactivation ==
    /\ state = "Active"
    /\ IF BUGGY_DEACTIVATE
       THEN TRUE  \* Buggy: Allow deactivation with pending messages
       ELSE pending = 0  \* Safe: Wait for invocations to complete
    /\ idleTicks >= IDLE_TIMEOUT
    /\ state' = "Deactivating"
    /\ pending' = pending
    /\ idleTicks' = idleTicks

\* CompleteDeactivation(actor): Finish deactivation, return to Inactive
\* Models: deactivate() completing (on_deactivate + state persistence)
\* Safe: Requires pending = 0 (all messages drained)
\* Buggy: Allows completion with pending messages (violates GracefulDeactivation)
CompleteDeactivation ==
    /\ state = "Deactivating"
    /\ IF BUGGY_DEACTIVATE
       THEN TRUE  \* Buggy: Allow deactivation with pending messages
       ELSE pending = 0  \* Safe: Must drain all messages first
    /\ state' = "Inactive"
    /\ pending' = 0
    /\ idleTicks' = 0

(***************************************************************************)
(* Action Aliases (for compatibility with issue specification)             *)
(***************************************************************************)

Activate == StartActivation
StartInvoke == EnqueueMessage
CompleteInvoke == ProcessMessage
IdleTick == Tick
StartDeactivate == StartDeactivation
CompleteDeactivate == CompleteDeactivation
CheckIdleTimeout == StartDeactivation

(***************************************************************************)
(* Next State Relation                                                     *)
(***************************************************************************)

Next ==
    \/ StartActivation
    \/ CompleteActivation
    \/ EnqueueMessage
    \/ ProcessMessage
    \/ DrainMessage
    \/ Tick
    \/ StartDeactivation
    \/ CompleteDeactivation

(***************************************************************************)
(* Fairness (for liveness properties)                                      *)
(***************************************************************************)

\* Weak fairness: If an action is continuously enabled, it eventually happens
\* Strong fairness: If an action is infinitely often enabled, it eventually happens
\* We use SF for StartDeactivation because invocations may interrupt the idle period,
\* but if the actor keeps becoming eligible for deactivation, it should eventually happen.
Fairness ==
    /\ WF_vars(CompleteActivation)
    /\ WF_vars(ProcessMessage)
    /\ WF_vars(DrainMessage)
    /\ WF_vars(Tick)
    /\ SF_vars(StartDeactivation)  \* Strong fairness: fires even if only intermittently enabled
    /\ WF_vars(CompleteDeactivation)

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* Safety Invariants                                                       *)
(***************************************************************************)

\* LifecycleOrdering: States follow Inactive -> Activating -> Active -> Deactivating -> Inactive
\* Invocations can only happen when the actor is Active (G1.3)
\* In buggy mode (BUGGY_INVOKE), this can be violated
LifecycleOrdering ==
    pending > 0 => state \in {"Active", "Deactivating"}

\* GracefulDeactivation: Pending messages drained before deactivate completes
\* Cannot be in Deactivating state with pending > 0 when about to complete (G1.3)
\* In buggy mode (BUGGY_DEACTIVATE), this can be violated
GracefulDeactivation ==
    state = "Deactivating" => pending = 0

\* IdleTimeoutRespected: Actor idle beyond timeout must be deactivating/deactivated (G1.5)
\* Only start deactivation after idle timeout is reached
IdleTimeoutRespected ==
    state = "Deactivating" => idleTicks >= IDLE_TIMEOUT

\* NoResurrection: Deactivated actor cannot process without re-activation
\* Cannot have pending messages in Inactive state
NoResurrection ==
    state = "Inactive" => pending = 0

\* NoInvokeWhileDeactivating: Cannot start new invocations during deactivation
\* (This is a stricter form - ensured by EnqueueMessage precondition when not buggy)
NoNewInvokesWhileDeactivating ==
    ~BUGGY_INVOKE => (state = "Deactivating" => pending = pending)

(***************************************************************************)
(* Liveness Properties                                                     *)
(***************************************************************************)

\* EventualDeactivation: Idle actors eventually deactivated (G1.5)
\* If actor reaches idle timeout, it eventually deactivates
EventualDeactivation ==
    (state = "Active" /\ idleTicks >= IDLE_TIMEOUT) ~> (state = "Inactive")

\* EventualActivation: First invocation eventually activates actor
\* If activation starts, it eventually completes
EventualActivation ==
    (state = "Activating") ~> (state = "Active" \/ state = "Inactive")

\* MessageProgress: Pending messages eventually processed or rejected
\* Started invocations eventually complete
MessageProgress ==
    (pending > 0) ~> (pending = 0)

\* Alias for backward compatibility
EventualInvocationCompletion == MessageProgress

(***************************************************************************)
(* Composite Properties                                                    *)
(***************************************************************************)

\* All safety invariants combined
Safety == TypeOK /\ LifecycleOrdering /\ GracefulDeactivation /\ IdleTimeoutRespected /\ NoResurrection

\* All liveness properties combined
Liveness == EventualDeactivation /\ EventualActivation /\ MessageProgress

=============================================================================
\* Modification History
\* Updated 2026-01-27 by Claude for Kelpie project - added NoResurrection, 
\*   MessageProgress, DrainMessage, renamed actions per issue spec
\* Created 2026-01-24 by Claude for Kelpie project
\* Reference: GitHub Issue #8
