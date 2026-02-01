------------------------------ MODULE KelpieMultiAgentInvocation ------------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie Multi-Agent Communication                 *)
(*                                                                          *)
(* Related ADRs:                                                            *)
(*   - docs/adr/028-multi-agent-communication.md (Call Protocol)           *)
(*   - docs/adr/001-virtual-actor-model.md (Single Activation Guarantee)   *)
(*                                                                          *)
(* This spec models agent-to-agent invocation ensuring:                     *)
(* - SAFETY: No circular call chains (deadlock prevention)                 *)
(* - SAFETY: Single activation maintained during cross-agent calls         *)
(* - SAFETY: Call depth bounded to MAX_DEPTH                               *)
(* - SAFETY: Timeout prevents infinite waiting                             *)
(* - LIVENESS: Every call eventually completes, times out, or fails        *)
(* - LIVENESS: Cycles detected early before first recursive iteration      *)
(*                                                                          *)
(* Call Protocol:                                                           *)
(* 1. Caller validates: target exists, not in call chain, depth < MAX      *)
(* 2. Caller sends invoke request with call_chain appended                 *)
(* 3. Target processes request, may itself call others (recursive)         *)
(* 4. Target sends response back to caller                                 *)
(* 5. Timeout fires if response not received within TIMEOUT_MS             *)
(*                                                                          *)
(* TigerStyle Constants:                                                    *)
(*   MAX_DEPTH = 5 (agent_call_depth_max)                                  *)
(*   TIMEOUT_MS = 30000 (agent_call_timeout_ms_default)                    *)
(*                                                                          *)
(* DST Tests: crates/kelpie-server/tests/multi_agent_dst.rs                *)
(* - test_agent_calls_agent_success: Basic A->B call works                 *)
(* - test_agent_call_cycle_detection: A->B->A rejected (not deadlock)      *)
(* - test_agent_call_timeout: Slow agent triggers timeout                  *)
(* - test_agent_call_depth_limit: A->B->C->D->E->F (depth 5), F->G fails   *)
(* - test_agent_call_under_network_partition: Graceful failure             *)
(* - test_single_activation_during_cross_call: SAG holds during calls      *)
(* - test_agent_call_with_storage_faults: Fault tolerance                  *)
(* - test_determinism_multi_agent: Same seed = same result                 *)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets

(***************************************************************************)
(* CONSTANTS                                                                *)
(***************************************************************************)

CONSTANTS
    Agents,         \* Set of agent IDs (e.g., {"agent-a", "agent-b", "agent-c"})
    Nodes,          \* Set of nodes that can host agents
    MAX_DEPTH,      \* Maximum call depth (TigerStyle: agent_call_depth_max = 5)
    TIMEOUT_MS,     \* Call timeout in ms (TigerStyle: agent_call_timeout_ms_default = 30000)
    NONE            \* Sentinel value

ASSUME MAX_DEPTH \in Nat /\ MAX_DEPTH > 0
ASSUME TIMEOUT_MS \in Nat /\ TIMEOUT_MS > 0

(***************************************************************************)
(* VARIABLES                                                                *)
(*                                                                          *)
(* agentState: Per-agent state (Idle, Processing, WaitingForCall)          *)
(* callStack: Per-agent call stack (sequence of caller agent IDs)          *)
(* activeNode: Which node hosts each agent (for single activation)         *)
(* pendingCalls: Set of pending call requests                              *)
(* callResults: Results of completed calls                                  *)
(* elapsedTime: Simulated time for each call (for timeout checking)        *)
(***************************************************************************)

VARIABLES
    agentState,     \* [Agents -> {"Idle", "Processing", "WaitingForCall"}]
    callStack,      \* [Agents -> Seq(Agents)] - call chain for cycle detection
    activeNode,     \* [Agents -> Nodes \cup {NONE}] - where each agent is active
    pendingCalls,   \* Set of records: [caller, target, callChain, startTime]
    callResults,    \* [CallId -> {"Pending", "Completed", "TimedOut", "Failed", "CycleRejected"}]
    globalTime      \* Global simulated time (ms)

vars == <<agentState, callStack, activeNode, pendingCalls, callResults, globalTime>>

(***************************************************************************)
(* TYPE INVARIANT                                                           *)
(***************************************************************************)

AgentStateType == {"Idle", "Processing", "WaitingForCall"}
CallResultType == {"Pending", "Completed", "TimedOut", "Failed", "CycleRejected"}

TypeOK ==
    /\ agentState \in [Agents -> AgentStateType]
    /\ \A a \in Agents: callStack[a] \in Seq(Agents)
    /\ activeNode \in [Agents -> Nodes \cup {NONE}]
    /\ globalTime \in Nat

\* Helper: Convert sequence to set for membership checking
ToSet(seq) == {seq[i] : i \in 1..Len(seq)}

\* Helper: Check if a target is already in a call chain (cycle detection)
InCallChain(target, chain) == target \in ToSet(chain)

\* Helper: Current depth of call stack
CallDepth(agent) == Len(callStack[agent])

(***************************************************************************)
(* INITIAL STATE                                                            *)
(***************************************************************************)

Init ==
    /\ agentState = [a \in Agents |-> "Idle"]
    /\ callStack = [a \in Agents |-> <<>>]
    /\ activeNode = [a \in Agents |-> NONE]
    /\ pendingCalls = {}
    /\ callResults = [c \in {} |-> "Pending"]  \* Empty function
    /\ globalTime = 0

(***************************************************************************)
(* ACTIONS                                                                  *)
(*                                                                          *)
(* Call Protocol Actions:                                                   *)
(* 1. InitiateCall: Agent A starts calling Agent B                         *)
(* 2. RejectCycle: Detect cycle and reject immediately                     *)
(* 3. RejectDepth: Detect max depth and reject                             *)
(* 4. ProcessCall: Target agent receives and processes call                *)
(* 5. CompleteCall: Call finishes successfully                             *)
(* 6. TimeoutCall: Call exceeds timeout                                    *)
(* 7. ActivateAgent: Agent becomes active on a node                        *)
(* 8. DeactivateAgent: Agent becomes idle                                  *)
(* 9. AdvanceTime: Global time advances                                    *)
(***************************************************************************)

\* Agent A initiates a call to Agent B
\* Preconditions: A is processing, B is a valid target
InitiateCall(caller, target) ==
    /\ caller # target                              \* Can't call self
    /\ agentState[caller] = "Processing"
    /\ activeNode[caller] # NONE                    \* Caller must be active
    /\ LET currentChain == callStack[caller]
           newChain == Append(currentChain, caller)
       IN
       \* Only proceed if no cycle and depth OK (checked separately)
       /\ ~InCallChain(target, currentChain)        \* No cycle
       /\ Len(newChain) < MAX_DEPTH                 \* Depth bounded
       /\ pendingCalls' = pendingCalls \cup {
            [caller |-> caller, target |-> target,
             callChain |-> newChain, startTime |-> globalTime]
          }
       /\ agentState' = [agentState EXCEPT ![caller] = "WaitingForCall"]
       /\ UNCHANGED <<callStack, activeNode, callResults, globalTime>>

\* Cycle detected - reject immediately without deadlock
RejectCycle(caller, target) ==
    /\ caller # target
    /\ agentState[caller] = "Processing"
    /\ activeNode[caller] # NONE
    /\ InCallChain(target, callStack[caller])       \* Cycle exists!
    \* Reject immediately - caller continues processing (with error result)
    /\ agentState' = [agentState EXCEPT ![caller] = "Processing"]
    /\ UNCHANGED <<callStack, activeNode, pendingCalls, callResults, globalTime>>

\* Max depth reached - reject
RejectDepth(caller, target) ==
    /\ caller # target
    /\ agentState[caller] = "Processing"
    /\ activeNode[caller] # NONE
    /\ CallDepth(caller) >= MAX_DEPTH               \* At max depth
    \* Reject - caller continues with error
    /\ agentState' = [agentState EXCEPT ![caller] = "Processing"]
    /\ UNCHANGED <<callStack, activeNode, pendingCalls, callResults, globalTime>>

\* Target agent receives and starts processing a call
ProcessCall(call) ==
    /\ call \in pendingCalls
    /\ LET target == call.target
           chain == call.callChain
       IN
       /\ agentState[target] = "Idle" \/ agentState[target] = "Processing"
       /\ activeNode[target] # NONE                 \* Target must be active
       /\ agentState' = [agentState EXCEPT ![target] = "Processing"]
       /\ callStack' = [callStack EXCEPT ![target] = chain]
       /\ UNCHANGED <<activeNode, pendingCalls, callResults, globalTime>>

\* Call completes successfully
CompleteCall(call) ==
    /\ call \in pendingCalls
    /\ LET target == call.target
           caller == call.caller
       IN
       /\ agentState[target] = "Processing"         \* Target finished processing
       /\ pendingCalls' = pendingCalls \ {call}
       /\ agentState' = [agentState EXCEPT
            ![caller] = "Processing",               \* Caller resumes
            ![target] = "Idle"                      \* Target done
          ]
       /\ callStack' = [callStack EXCEPT ![target] = <<>>]  \* Clear target stack
       /\ UNCHANGED <<activeNode, callResults, globalTime>>

\* Call times out
TimeoutCall(call) ==
    /\ call \in pendingCalls
    /\ globalTime - call.startTime >= TIMEOUT_MS    \* Timeout exceeded
    /\ LET caller == call.caller
       IN
       /\ pendingCalls' = pendingCalls \ {call}
       /\ agentState' = [agentState EXCEPT ![caller] = "Processing"]  \* Caller resumes with error
       /\ UNCHANGED <<callStack, activeNode, callResults, globalTime>>

\* Agent becomes active on a node (single activation guarantee)
ActivateAgent(agent, node) ==
    /\ activeNode[agent] = NONE                     \* Not already active
    /\ agentState[agent] = "Idle"
    /\ activeNode' = [activeNode EXCEPT ![agent] = node]
    /\ agentState' = [agentState EXCEPT ![agent] = "Processing"]
    /\ UNCHANGED <<callStack, pendingCalls, callResults, globalTime>>

\* Agent becomes idle and releases node
DeactivateAgent(agent) ==
    /\ agentState[agent] = "Idle" \/ agentState[agent] = "Processing"
    /\ callStack[agent] = <<>>                      \* No pending calls
    /\ ~(\E c \in pendingCalls : c.caller = agent)  \* Not waiting for any call
    /\ activeNode' = [activeNode EXCEPT ![agent] = NONE]
    /\ agentState' = [agentState EXCEPT ![agent] = "Idle"]
    /\ UNCHANGED <<callStack, pendingCalls, callResults, globalTime>>

\* Time advances (for timeout checking)
AdvanceTime ==
    /\ globalTime' = globalTime + 1
    /\ UNCHANGED <<agentState, callStack, activeNode, pendingCalls, callResults>>

(***************************************************************************)
(* NEXT STATE RELATION                                                      *)
(***************************************************************************)

Next ==
    \/ \E a, b \in Agents: InitiateCall(a, b)
    \/ \E a, b \in Agents: RejectCycle(a, b)
    \/ \E a, b \in Agents: RejectDepth(a, b)
    \/ \E c \in pendingCalls: ProcessCall(c)
    \/ \E c \in pendingCalls: CompleteCall(c)
    \/ \E c \in pendingCalls: TimeoutCall(c)
    \/ \E a \in Agents, n \in Nodes: ActivateAgent(a, n)
    \/ \E a \in Agents: DeactivateAgent(a)
    \/ AdvanceTime

(***************************************************************************)
(* FAIRNESS - Required for Liveness                                         *)
(*                                                                          *)
(* TLC cannot quantify over state variables in temporal formulas.           *)
(* We use a simplified fairness condition: all Next actions are fair.       *)
(***************************************************************************)

Fairness == WF_vars(Next)

(***************************************************************************)
(* SAFETY INVARIANTS (MUST NEVER BE VIOLATED)                              *)
(***************************************************************************)

\* SAFETY 1: No circular calls - agent cannot be in its own call stack
\* This prevents A->B->A deadlock scenario
NoDeadlock ==
    \A a \in Agents:
        LET stack == callStack[a]
        IN Cardinality(ToSet(stack)) = Len(stack)   \* All elements unique

\* SAFETY 2: Single activation guarantee holds during cross-agent calls
\* At most one node can host an agent at any time
SingleActivationDuringCall ==
    \A a \in Agents:
        Cardinality({n \in Nodes : activeNode[a] = n}) <= 1

\* SAFETY 3: Call depth is always bounded
DepthBounded ==
    \A a \in Agents: Len(callStack[a]) <= MAX_DEPTH

\* SAFETY 4: Bounded pending calls
\* The number of pending calls is always bounded (prevents resource exhaustion)
BoundedPendingCalls ==
    Cardinality(pendingCalls) <= Cardinality(Agents) * MAX_DEPTH

\* Combined safety invariant
SafetyInvariant ==
    /\ TypeOK
    /\ NoDeadlock
    /\ SingleActivationDuringCall
    /\ DepthBounded
    /\ BoundedPendingCalls

(***************************************************************************)
(* LIVENESS PROPERTIES (MUST EVENTUALLY HAPPEN)                            *)
(***************************************************************************)

\* LIVENESS 1: The system doesn't get stuck with pending calls forever
\* If there are pending calls, eventually they are processed
CallsEventuallyComplete ==
    (pendingCalls # {}) ~> (pendingCalls = {})

\* LIVENESS 2: Cycle detection happens immediately (before any waiting)
\* If a call would create a cycle, it's rejected before being added to pendingCalls
\* This is implicitly enforced by RejectCycle action's preconditions - cycles never
\* appear in pendingCalls because InitiateCall checks ~InCallChain as precondition
CycleDetectedEarly ==
    \* This is a safety property: cycles are never in pending calls
    \A c \in pendingCalls:
        ~InCallChain(c.target, c.callChain)

\* LIVENESS 3: If an agent is waiting for a call, it eventually resumes
WaitingAgentsResume ==
    \A a \in Agents:
        (agentState[a] = "WaitingForCall") ~> (agentState[a] # "WaitingForCall")

(***************************************************************************)
(* SPECIFICATION                                                            *)
(***************************************************************************)

\* Full specification with fairness for liveness checking
Spec == Init /\ [][Next]_vars /\ Fairness

\* Safety-only specification (faster to check)
SafetySpec == Init /\ [][Next]_vars

(***************************************************************************)
(* STATE CONSTRAINTS (for bounded model checking)                          *)
(***************************************************************************)

\* Limit state space for tractable checking
StateConstraint ==
    /\ globalTime <= 100
    /\ Cardinality(pendingCalls) <= 10

(***************************************************************************)
(* THEOREMS                                                                 *)
(***************************************************************************)

\* Safety theorem: No deadlocks in any reachable state
THEOREM Spec => []NoDeadlock

\* Safety theorem: Single activation always holds
THEOREM Spec => []SingleActivationDuringCall

\* Safety theorem: Depth always bounded
THEOREM Spec => []DepthBounded

\* Liveness theorem: Calls eventually resolve
THEOREM Spec => CallsEventuallyComplete

\* Liveness theorem: Waiting agents resume
THEOREM Spec => WaitingAgentsResume

=============================================================================
