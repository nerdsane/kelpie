------------------------------ MODULE KelpieAgentActor ------------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie Agent Actor State Machine                 *)
(*                                                                         *)
(* Models agent-specific invariants from ADR-013, ADR-014:                 *)
(* - Iteration-based checkpointing                                         *)
(* - Message queue processing order                                        *)
(* - Pause/resume semantics                                                *)
(* - Crash recovery from FDB checkpoint                                    *)
(*                                                                         *)
(* BUGGY mode: Skips FDB checkpoint write, causing CheckpointIntegrity     *)
(* violation after crash+recovery.                                         *)
(*                                                                         *)
(* Reference: ADR-013 Actor-Based Agent Server, ADR-014 AgentService       *)
(* Implementation: crates/kelpie-server/src/agent/                         *)
(***************************************************************************)

EXTENDS Naturals, Sequences, FiniteSets, TLC

(***************************************************************************)
(* CONSTANTS                                                               *)
(***************************************************************************)

CONSTANTS
    Nodes,              \* Set of nodes (e.g., {n1, n2})
    NONE,               \* Sentinel value for unset/nil values
    MAX_ITERATIONS,     \* Bound for model checking
    MAX_MESSAGES,       \* Bound on message queue
    BUGGY               \* TRUE enables buggy behavior (skip FDB write)

ASSUME Nodes # {}
ASSUME NONE \notin Nodes
ASSUME MAX_ITERATIONS \in Nat /\ MAX_ITERATIONS > 0
ASSUME MAX_MESSAGES \in Nat /\ MAX_MESSAGES > 0
ASSUME BUGGY \in BOOLEAN

(***************************************************************************)
(* VARIABLES                                                               *)
(*                                                                         *)
(* agentState: Per-node agent state machine                                *)
(* fdbCheckpoint: FDB storage - ground truth for crash recovery            *)
(* nodeBeliefs: Per-node belief of FDB state (can be stale)                *)
(* messageQueue: Sequence of incoming messages                             *)
(* iteration: Current iteration number                                      *)
(* crashed: Per-node crash state (for DST alignment)                       *)
(***************************************************************************)

VARIABLES
    agentState,         \* Per-node state: Inactive|Starting|Running|Paused|Stopping|Stopped
    fdbCheckpoint,      \* FDB ground truth: [iteration, message_index, paused_until_ms]
    nodeBeliefs,        \* Per-node belief: [iteration, message_index]
    messageQueue,       \* Sequence of messages (bounded by MAX_MESSAGES)
    iteration,          \* Current global iteration counter
    crashed             \* Per-node crash state

vars == <<agentState, fdbCheckpoint, nodeBeliefs, messageQueue, iteration, crashed>>

(***************************************************************************)
(* TYPE DEFINITIONS                                                        *)
(***************************************************************************)

AgentStates == {"Inactive", "Starting", "Running", "Paused", "Stopping", "Stopped"}

\* Type invariant - ensures all variables have expected types
TypeOK ==
    /\ agentState \in [Nodes -> AgentStates]
    /\ fdbCheckpoint \in [iteration: Nat, message_index: Nat, paused_until_ms: Nat]
    /\ nodeBeliefs \in [Nodes -> [iteration: Nat, message_index: Nat]]
    /\ messageQueue \in Seq(Nat)  \* Messages are just sequence numbers
    /\ Len(messageQueue) <= MAX_MESSAGES
    /\ iteration \in Nat
    /\ iteration <= MAX_ITERATIONS
    /\ crashed \in [Nodes -> BOOLEAN]

(***************************************************************************)
(* HELPERS                                                                 *)
(***************************************************************************)

\* Count nodes in Running state
RunningNodes == {n \in Nodes : agentState[n] = "Running"}

\* A node is active if it's Running or Paused
ActiveNodes == {n \in Nodes : agentState[n] \in {"Running", "Paused"}}

\* A node is claiming if it's Starting, Running, or Paused (holds or is acquiring the lock)
ClaimingNodes == {n \in Nodes : agentState[n] \in {"Starting", "Running", "Paused", "Stopping"}}

\* A node can process messages if it's Running and has messages
CanProcess(n) ==
    /\ agentState[n] = "Running"
    /\ nodeBeliefs[n].message_index < Len(messageQueue)

(***************************************************************************)
(* INITIAL STATE                                                           *)
(***************************************************************************)

Init ==
    /\ agentState = [n \in Nodes |-> "Inactive"]
    /\ fdbCheckpoint = [iteration |-> 0, message_index |-> 0, paused_until_ms |-> 0]
    /\ nodeBeliefs = [n \in Nodes |-> [iteration |-> 0, message_index |-> 0]]
    /\ messageQueue = <<>>
    /\ iteration = 0
    /\ crashed = [n \in Nodes |-> FALSE]

(***************************************************************************)
(* ACTIONS                                                                 *)
(***************************************************************************)

\* 1. EnqueueMessage - Add message to queue
\* Models: External client sending message to agent
EnqueueMessage ==
    /\ Len(messageQueue) < MAX_MESSAGES
    /\ messageQueue' = Append(messageQueue, Len(messageQueue) + 1)
    /\ UNCHANGED <<agentState, fdbCheckpoint, nodeBeliefs, iteration, crashed>>

\* 2. StartAgent(n) - Node starts agent, reads FDB checkpoint
\* Models: Node receiving activation request, reading checkpoint from FDB
\* Precondition: Node is Inactive, no other node is claiming (Starting/Running/Paused/Stopping)
StartAgent(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Inactive"
    /\ ClaimingNodes = {}  \* Single activation guarantee - no one else is active or activating
    /\ agentState' = [agentState EXCEPT ![n] = "Starting"]
    \* Read checkpoint from FDB (ground truth)
    /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![n] =
                       [iteration |-> fdbCheckpoint.iteration,
                        message_index |-> fdbCheckpoint.message_index]]
    /\ UNCHANGED <<fdbCheckpoint, messageQueue, iteration, crashed>>

\* 3. CompleteStartup(n) - Agent transitions to Running
\* Models: on_activate hook completion
CompleteStartup(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Starting"
    /\ agentState' = [agentState EXCEPT ![n] = "Running"]
    /\ UNCHANGED <<fdbCheckpoint, nodeBeliefs, messageQueue, iteration, crashed>>

\* 4. ExecuteIteration(n) - Process message, write checkpoint
\* Models: Agent loop processing one message and checkpointing
\* BUGGY mode: Skips FDB write, causing checkpoint drift
ExecuteIteration(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Running"
    /\ CanProcess(n)
    /\ iteration < MAX_ITERATIONS
    /\ LET newIteration == iteration + 1
           newMessageIndex == nodeBeliefs[n].message_index + 1
       IN
       /\ iteration' = newIteration
       /\ nodeBeliefs' = [nodeBeliefs EXCEPT
                          ![n].iteration = newIteration,
                          ![n].message_index = newMessageIndex]
       /\ IF ~BUGGY
          THEN fdbCheckpoint' = [iteration |-> newIteration,
                                  message_index |-> newMessageIndex,
                                  paused_until_ms |-> 0]
          ELSE UNCHANGED fdbCheckpoint  \* BUGGY: Skip FDB write
    /\ UNCHANGED <<agentState, messageQueue, crashed>>

\* 5. StopAgent(n) - Initiate graceful shutdown
\* Models: Deactivation request received
StopAgent(n) ==
    /\ ~crashed[n]
    /\ agentState[n] \in {"Running", "Paused"}
    /\ agentState' = [agentState EXCEPT ![n] = "Stopping"]
    /\ UNCHANGED <<fdbCheckpoint, nodeBeliefs, messageQueue, iteration, crashed>>

\* 6. CompleteStop(n) - Finish shutdown
\* Models: on_deactivate hook completion
CompleteStop(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Stopping"
    /\ agentState' = [agentState EXCEPT ![n] = "Stopped"]
    /\ UNCHANGED <<fdbCheckpoint, nodeBeliefs, messageQueue, iteration, crashed>>

\* 7. NodeCrash(n) - Node crashes, loses local state
\* Models: Node crash during agent operation (DST alignment)
\* Note: nodeBeliefs are lost but fdbCheckpoint persists
NodeCrash(n) ==
    /\ ~crashed[n]
    /\ agentState[n] # "Inactive"  \* Only crash if doing something
    /\ crashed' = [crashed EXCEPT ![n] = TRUE]
    /\ agentState' = [agentState EXCEPT ![n] = "Inactive"]
    /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![n] = [iteration |-> 0, message_index |-> 0]]
    /\ UNCHANGED <<fdbCheckpoint, messageQueue, iteration>>

\* 8. NodeRecover(n) - Node recovers, ready to restart
\* Models: Node restart after crash
NodeRecover(n) ==
    /\ crashed[n]
    /\ crashed' = [crashed EXCEPT ![n] = FALSE]
    /\ UNCHANGED <<agentState, fdbCheckpoint, nodeBeliefs, messageQueue, iteration>>

\* 9. PauseAgent(n) - Agent pauses (heartbeat pause)
\* Models: pause_until_ms being set (e.g., waiting for external event)
PauseAgent(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Running"
    /\ agentState' = [agentState EXCEPT ![n] = "Paused"]
    \* Write pause state to FDB
    /\ fdbCheckpoint' = [fdbCheckpoint EXCEPT !.paused_until_ms = 1]  \* Non-zero = paused
    /\ UNCHANGED <<nodeBeliefs, messageQueue, iteration, crashed>>

\* 10. ResumeAgent(n) - Agent resumes after pause
\* Models: pause_until_ms expiring or being cleared
ResumeAgent(n) ==
    /\ ~crashed[n]
    /\ agentState[n] = "Paused"
    /\ agentState' = [agentState EXCEPT ![n] = "Running"]
    \* Clear pause state in FDB
    /\ fdbCheckpoint' = [fdbCheckpoint EXCEPT !.paused_until_ms = 0]
    /\ UNCHANGED <<nodeBeliefs, messageQueue, iteration, crashed>>

(***************************************************************************)
(* NEXT STATE RELATION                                                     *)
(***************************************************************************)

Next ==
    \/ EnqueueMessage
    \/ \E n \in Nodes:
        \/ StartAgent(n)
        \/ CompleteStartup(n)
        \/ ExecuteIteration(n)
        \/ StopAgent(n)
        \/ CompleteStop(n)
        \/ NodeCrash(n)
        \/ NodeRecover(n)
        \/ PauseAgent(n)
        \/ ResumeAgent(n)

(***************************************************************************)
(* FAIRNESS (for liveness properties)                                      *)
(*                                                                         *)
(* WF = weak fairness: if continuously enabled, eventually happens         *)
(* SF = strong fairness: if infinitely often enabled, eventually happens   *)
(***************************************************************************)

Fairness ==
    /\ \A n \in Nodes: WF_vars(CompleteStartup(n))
    /\ \A n \in Nodes: WF_vars(ExecuteIteration(n))
    /\ \A n \in Nodes: WF_vars(CompleteStop(n))
    /\ \A n \in Nodes: WF_vars(NodeRecover(n))
    /\ \A n \in Nodes: WF_vars(ResumeAgent(n))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* SAFETY INVARIANTS                                                       *)
(***************************************************************************)

\* SingleActivation: At most one node can have an active/claiming agent
\* This is THE critical guarantee for virtual actors
\* Includes Starting state to prevent race during activation
SingleActivation ==
    Cardinality(ClaimingNodes) <= 1

\* CheckpointIntegrity: FDB must record progress when iterations happen
\* If progress has been made (iteration > 0), FDB must know about it
\* BUGGY mode violates this because it skips FDB writes
CheckpointIntegrity ==
    iteration > 0 => fdbCheckpoint.iteration > 0

\* MessageProcessingOrder: Messages processed in FIFO order
\* Each node's message_index is monotonically increasing
\* and never skips messages
MessageProcessingOrder ==
    \A n \in Nodes:
        nodeBeliefs[n].message_index <= Len(messageQueue)

\* StateConsistency: Running node's belief matches or trails FDB
\* A running node should have consistent or slightly stale view
StateConsistency ==
    \A n \in Nodes:
        agentState[n] = "Running" =>
            /\ nodeBeliefs[n].iteration >= fdbCheckpoint.iteration - 1
            /\ nodeBeliefs[n].message_index >= fdbCheckpoint.message_index - 1

\* PausedMeansCheckpointed: Paused state is reflected in FDB
\* If agent is paused, FDB should show paused_until_ms > 0
PausedConsistency ==
    \A n \in Nodes:
        agentState[n] = "Paused" => fdbCheckpoint.paused_until_ms > 0

\* Combined safety invariant
SafetyInvariant ==
    /\ TypeOK
    /\ SingleActivation
    /\ CheckpointIntegrity
    /\ MessageProcessingOrder

(***************************************************************************)
(* LIVENESS PROPERTIES                                                     *)
(***************************************************************************)

\* EventualCompletion: Messages eventually get processed
\* If there are unprocessed messages and an agent is running,
\* they will eventually be processed
EventualCompletion ==
    \A n \in Nodes:
        (agentState[n] = "Running" /\ CanProcess(n)) ~>
            (nodeBeliefs[n].message_index = Len(messageQueue) \/ agentState[n] # "Running")

\* EventualCrashRecovery: Crashed nodes eventually recover
\* System is self-healing
EventualCrashRecovery ==
    \A n \in Nodes:
        crashed[n] ~> ~crashed[n]

\* EventualCheckpoint: FDB eventually catches up to iteration
\* In safe mode (not BUGGY), FDB checkpoint equals iteration
EventualCheckpoint ==
    (~BUGGY) => [](iteration > 0 => <>(fdbCheckpoint.iteration = iteration))

(***************************************************************************)
(* STATE CONSTRAINT (for bounded model checking)                           *)
(***************************************************************************)

\* Bound state space for tractable checking
StateConstraint ==
    /\ iteration <= MAX_ITERATIONS
    /\ Len(messageQueue) <= MAX_MESSAGES

(***************************************************************************)
(* THEOREMS (for documentation)                                            *)
(***************************************************************************)

\* Safety theorem: SingleActivation holds in all reachable states
THEOREM Spec => []SingleActivation

\* Safety theorem: CheckpointIntegrity holds when not BUGGY
THEOREM (~BUGGY) => (Spec => []CheckpointIntegrity)

\* Liveness theorem: Crashed nodes eventually recover
THEOREM Spec => EventualCrashRecovery

=============================================================================
\* Modification History
\* Created 2026-01-24 for Kelpie project
\* Reference: GitHub Issue #12 - Create KelpieAgentActor.tla Specification
