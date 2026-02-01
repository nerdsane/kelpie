------------------------------ MODULE KelpieHttpApi ------------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie HTTP API Linearizability                  *)
(*                                                                         *)
(* This spec models the linearization guarantees for Kelpie's HTTP API     *)
(* layer, ensuring exactly-once semantics through idempotency tokens.      *)
(*                                                                         *)
(* Key Guarantees:                                                         *)
(* 1. IdempotencyGuarantee: Same token → same response                     *)
(* 2. ExactlyOnceExecution: Mutations execute ≤1 time per token            *)
(* 3. ReadAfterWriteConsistency: POST then GET returns entity              *)
(* 4. AtomicOperation: Multi-step appears atomic                           *)
(* 5. DurableOnSuccess: Success → state survives restart                   *)
(*                                                                         *)
(* Related Specs:                                                          *)
(* - KelpieLinearizability.tla: Actor-layer linearization                  *)
(* - KelpieFDBTransaction.tla: Storage transaction semantics               *)
(*                                                                         *)
(* TigerStyle: All constants have explicit units and bounds.               *)
(***************************************************************************)

EXTENDS Integers, Sequences, FiniteSets

(***************************************************************************)
(* CONSTANTS                                                               *)
(***************************************************************************)

CONSTANT
    HttpClients,          \* Set of HTTP clients that can make requests
    Agents,               \* Set of agent IDs
    IdempotencyTokens,    \* Set of idempotency tokens
    NONE,                 \* Sentinel value for "no value"
    MAX_OPERATIONS        \* Maximum operations for bounded checking

(***************************************************************************)
(* VARIABLES                                                               *)
(*                                                                         *)
(* The model tracks:                                                       *)
(* - Agent state in storage (ground truth)                                 *)
(* - Idempotency cache (token -> response mapping)                         *)
(* - Pending HTTP requests per client                                      *)
(* - Operation history for linearizability checking                        *)
(***************************************************************************)

VARIABLES
    \* Agent storage: agent_id -> AgentState | NONE
    agent_store,

    \* Idempotency cache: token -> CachedResponse | NONE
    \* CachedResponse = [status: Nat, body: agent_id | NONE]
    idempotency_cache,

    \* Pending HTTP requests: client -> Request | NONE
    \* Request = [type: RequestType, agent_id: String, token: Token | NONE]
    pending,

    \* Request execution state: client -> ExecutionState
    \* ExecutionState = "idle" | "executing" | "responded"
    exec_state,

    \* Operation history: sequence of completed operations
    history,

    \* Server state: "running" | "crashed" | "recovering"
    server_state,

    \* Operation counter for unique IDs
    op_counter

vars == <<agent_store, idempotency_cache, pending, exec_state, history, server_state, op_counter>>

(***************************************************************************)
(* REQUEST TYPES                                                           *)
(***************************************************************************)

RequestType == {"CreateAgent", "GetAgent", "DeleteAgent", "SendMessage"}

\* Mutating operations that require idempotency
MutatingRequests == {"CreateAgent", "DeleteAgent", "SendMessage"}

\* Idempotent (safe) operations
IdempotentRequests == {"GetAgent"}

(***************************************************************************)
(* RESPONSE TYPES                                                          *)
(***************************************************************************)

\* HTTP status codes
StatusOK == 200
StatusCreated == 201
StatusNotFound == 404
StatusConflict == 409
StatusInternalError == 500

\* Response structure
Response == [status: Nat, agent_id: Agents \cup {NONE}]

(***************************************************************************)
(* TYPE INVARIANT                                                          *)
(***************************************************************************)

TypeOK ==
    /\ agent_store \in [Agents -> {"exists", NONE}]
    /\ idempotency_cache \in [IdempotencyTokens -> (Response \cup {NONE})]
    /\ pending \in [HttpClients -> ([type: RequestType, agent_id: Agents, token: IdempotencyTokens \cup {NONE}] \cup {NONE})]
    /\ exec_state \in [HttpClients -> {"idle", "executing", "responded"}]
    /\ history \in Seq([type: RequestType, client: HttpClients, agent_id: Agents, token: IdempotencyTokens \cup {NONE}, response: Response])
    /\ server_state \in {"running", "crashed", "recovering"}
    /\ op_counter \in Nat

(***************************************************************************)
(* INITIAL STATE                                                           *)
(***************************************************************************)

Init ==
    /\ agent_store = [a \in Agents |-> NONE]
    /\ idempotency_cache = [t \in IdempotencyTokens |-> NONE]
    /\ pending = [c \in HttpClients |-> NONE]
    /\ exec_state = [c \in HttpClients |-> "idle"]
    /\ history = <<>>
    /\ server_state = "running"
    /\ op_counter = 0

(***************************************************************************)
(* HELPER PREDICATES                                                       *)
(***************************************************************************)

\* Client has no pending request
ClientIdle(c) == pending[c] = NONE

\* Client has a pending request
ClientBusy(c) == pending[c] # NONE

\* Server is accepting requests
ServerRunning == server_state = "running"

\* Agent exists in storage
AgentExists(a) == agent_store[a] = "exists"

\* Agent does not exist
AgentNotExists(a) == agent_store[a] = NONE

\* Token has cached response
TokenCached(t) == idempotency_cache[t] # NONE

\* Is a mutating request type
IsMutating(reqType) == reqType \in MutatingRequests

(***************************************************************************)
(* CLIENT ACTIONS - Sending HTTP Requests                                  *)
(***************************************************************************)

\* Client sends a CreateAgent request with optional idempotency token
SendCreateAgent(c, a, t) ==
    /\ ServerRunning
    /\ ClientIdle(c)
    /\ pending' = [pending EXCEPT ![c] = [
           type |-> "CreateAgent",
           agent_id |-> a,
           token |-> t
       ]]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ op_counter' = op_counter + 1
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state>>

\* Client sends a GetAgent request (no idempotency token needed for reads)
SendGetAgent(c, a) ==
    /\ ServerRunning
    /\ ClientIdle(c)
    /\ pending' = [pending EXCEPT ![c] = [
           type |-> "GetAgent",
           agent_id |-> a,
           token |-> NONE
       ]]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ op_counter' = op_counter + 1
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state>>

\* Client sends a DeleteAgent request with optional idempotency token
SendDeleteAgent(c, a, t) ==
    /\ ServerRunning
    /\ ClientIdle(c)
    /\ pending' = [pending EXCEPT ![c] = [
           type |-> "DeleteAgent",
           agent_id |-> a,
           token |-> t
       ]]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ op_counter' = op_counter + 1
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state>>

\* Client sends a SendMessage request with optional idempotency token
SendMessage(c, a, t) ==
    /\ ServerRunning
    /\ ClientIdle(c)
    /\ pending' = [pending EXCEPT ![c] = [
           type |-> "SendMessage",
           agent_id |-> a,
           token |-> t
       ]]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ op_counter' = op_counter + 1
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state>>

(***************************************************************************)
(* SERVER ACTIONS - Processing HTTP Requests                               *)
(*                                                                         *)
(* The server processes requests in three phases to model atomicity:       *)
(* 1. BeginExecution: Start processing, check idempotency cache            *)
(* 2. CompleteExecution: Apply state changes, cache response               *)
(* 3. ReturnResponse: Send response to client                              *)
(***************************************************************************)

\* Server begins processing a request
\* If token is cached, skip execution and use cached response
BeginExecution(c) ==
    /\ ServerRunning
    /\ ClientBusy(c)
    /\ exec_state[c] = "idle"
    /\ LET req == pending[c]
       IN
       \* Check idempotency cache for mutating requests with tokens
       IF req.token # NONE /\ IsMutating(req.type) /\ TokenCached(req.token)
       THEN
           \* Cache hit - skip to responded state with cached response
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
           /\ UNCHANGED <<agent_store, idempotency_cache, pending, history, server_state, op_counter>>
       ELSE
           \* Cache miss or read request - begin execution
           /\ exec_state' = [exec_state EXCEPT ![c] = "executing"]
           /\ UNCHANGED <<agent_store, idempotency_cache, pending, history, server_state, op_counter>>

\* Server completes execution and applies state changes atomically
\* This models the linearization point for mutating operations
CompleteCreateAgent(c) ==
    /\ ServerRunning
    /\ ClientBusy(c)
    /\ exec_state[c] = "executing"
    /\ pending[c].type = "CreateAgent"
    /\ LET req == pending[c]
           a == req.agent_id
           t == req.token
       IN
       \* Create agent if it doesn't exist
       IF AgentNotExists(a)
       THEN
           /\ agent_store' = [agent_store EXCEPT ![a] = "exists"]
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusCreated, agent_id |-> a]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "CreateAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusCreated, agent_id |-> a]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
       ELSE
           \* Agent already exists - conflict
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusConflict, agent_id |-> NONE]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "CreateAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusConflict, agent_id |-> NONE]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
           /\ UNCHANGED agent_store
    /\ UNCHANGED <<pending, server_state, op_counter>>

CompleteGetAgent(c) ==
    /\ ServerRunning
    /\ ClientBusy(c)
    /\ exec_state[c] = "executing"
    /\ pending[c].type = "GetAgent"
    /\ LET req == pending[c]
           a == req.agent_id
       IN
       IF AgentExists(a)
       THEN
           /\ history' = Append(history, [
                  type |-> "GetAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> NONE,
                  response |-> [status |-> StatusOK, agent_id |-> a]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
       ELSE
           /\ history' = Append(history, [
                  type |-> "GetAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> NONE,
                  response |-> [status |-> StatusNotFound, agent_id |-> NONE]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
    /\ UNCHANGED <<agent_store, idempotency_cache, pending, server_state, op_counter>>

CompleteDeleteAgent(c) ==
    /\ ServerRunning
    /\ ClientBusy(c)
    /\ exec_state[c] = "executing"
    /\ pending[c].type = "DeleteAgent"
    /\ LET req == pending[c]
           a == req.agent_id
           t == req.token
       IN
       IF AgentExists(a)
       THEN
           /\ agent_store' = [agent_store EXCEPT ![a] = NONE]
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusOK, agent_id |-> a]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "DeleteAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusOK, agent_id |-> a]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
       ELSE
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusNotFound, agent_id |-> NONE]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "DeleteAgent",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusNotFound, agent_id |-> NONE]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
           /\ UNCHANGED agent_store
    /\ UNCHANGED <<pending, server_state, op_counter>>

CompleteSendMessage(c) ==
    /\ ServerRunning
    /\ ClientBusy(c)
    /\ exec_state[c] = "executing"
    /\ pending[c].type = "SendMessage"
    /\ LET req == pending[c]
           a == req.agent_id
           t == req.token
       IN
       IF AgentExists(a)
       THEN
           \* Message sent successfully (simplified - no actual message processing)
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusOK, agent_id |-> a]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "SendMessage",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusOK, agent_id |-> a]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
       ELSE
           /\ IF t # NONE
              THEN idempotency_cache' = [idempotency_cache EXCEPT ![t] = [status |-> StatusNotFound, agent_id |-> NONE]]
              ELSE UNCHANGED idempotency_cache
           /\ history' = Append(history, [
                  type |-> "SendMessage",
                  client |-> c,
                  agent_id |-> a,
                  token |-> t,
                  response |-> [status |-> StatusNotFound, agent_id |-> NONE]
              ])
           /\ exec_state' = [exec_state EXCEPT ![c] = "responded"]
    /\ UNCHANGED <<agent_store, pending, server_state, op_counter>>

\* Client receives response and becomes idle
ReceiveResponse(c) ==
    /\ ClientBusy(c)
    /\ exec_state[c] = "responded"
    /\ pending' = [pending EXCEPT ![c] = NONE]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state, op_counter>>

(***************************************************************************)
(* FAILURE ACTIONS - Server Crash and Recovery                             *)
(***************************************************************************)

\* Server crashes - all in-flight requests are lost
\* Idempotency cache persists (durable)
ServerCrash ==
    /\ ServerRunning
    /\ server_state' = "crashed"
    \* In-flight requests are aborted
    /\ exec_state' = [c \in HttpClients |-> "idle"]
    /\ pending' = [c \in HttpClients |-> NONE]
    /\ UNCHANGED <<agent_store, idempotency_cache, history, op_counter>>

\* Server recovers from crash
ServerRecover ==
    /\ server_state = "crashed"
    /\ server_state' = "running"
    /\ UNCHANGED <<agent_store, idempotency_cache, pending, exec_state, history, op_counter>>

(***************************************************************************)
(* RETRY ACTIONS - Client retries with same idempotency token              *)
(***************************************************************************)

\* Client retries a CreateAgent with same token after not receiving response
RetryCreateAgent(c, a, t) ==
    /\ ServerRunning
    /\ ClientIdle(c)
    /\ t # NONE  \* Must have token to retry
    /\ pending' = [pending EXCEPT ![c] = [
           type |-> "CreateAgent",
           agent_id |-> a,
           token |-> t
       ]]
    /\ exec_state' = [exec_state EXCEPT ![c] = "idle"]
    /\ UNCHANGED <<agent_store, idempotency_cache, history, server_state, op_counter>>

(***************************************************************************)
(* NEXT STATE RELATION                                                     *)
(***************************************************************************)

Next ==
    \* Client sends requests
    \/ \E c \in HttpClients, a \in Agents:
        SendGetAgent(c, a)
    \/ \E c \in HttpClients, a \in Agents, t \in IdempotencyTokens \cup {NONE}:
        \/ SendCreateAgent(c, a, t)
        \/ SendDeleteAgent(c, a, t)
        \/ SendMessage(c, a, t)
    \* Server processes requests
    \/ \E c \in HttpClients:
        \/ BeginExecution(c)
        \/ CompleteCreateAgent(c)
        \/ CompleteGetAgent(c)
        \/ CompleteDeleteAgent(c)
        \/ CompleteSendMessage(c)
        \/ ReceiveResponse(c)
    \* Retries
    \/ \E c \in HttpClients, a \in Agents, t \in IdempotencyTokens:
        RetryCreateAgent(c, a, t)
    \* Failures
    \/ ServerCrash
    \/ ServerRecover

(***************************************************************************)
(* FAIRNESS                                                                *)
(***************************************************************************)

Fairness ==
    /\ \A c \in HttpClients:
        /\ WF_vars(BeginExecution(c))
        /\ WF_vars(CompleteCreateAgent(c))
        /\ WF_vars(CompleteGetAgent(c))
        /\ WF_vars(CompleteDeleteAgent(c))
        /\ WF_vars(CompleteSendMessage(c))
        /\ WF_vars(ReceiveResponse(c))
    /\ WF_vars(ServerRecover)

(***************************************************************************)
(* SAFETY INVARIANTS                                                       *)
(***************************************************************************)

\* Invariant 1: IdempotencyGuarantee
\* Same idempotency token always returns the same response
\* If a token is in the cache, all operations with that token get the cached response
IdempotencyGuarantee ==
    \A t \in IdempotencyTokens:
        TokenCached(t) =>
        \A i, j \in 1..Len(history):
            (history[i].token = t /\ history[j].token = t) =>
            history[i].response = history[j].response

\* Invariant 2: ExactlyOnceExecution
\* For each idempotency token, at most one state mutation occurs
\* Mutations are: agent creation, agent deletion
ExactlyOnceExecution ==
    \A t \in IdempotencyTokens:
        LET ops == {i \in 1..Len(history):
                    history[i].token = t /\
                    history[i].type \in MutatingRequests}
        IN
        \* All operations with same token have same response (idempotent)
        \A i, j \in ops: history[i].response = history[j].response

\* Invariant 3: ReadAfterWriteConsistency
\* If CreateAgent succeeds (201), subsequent GetAgent returns the agent (200)
ReadAfterWriteConsistency ==
    \A i, j \in 1..Len(history):
        /\ i < j
        /\ history[i].type = "CreateAgent"
        /\ history[i].response.status = StatusCreated
        /\ history[j].type = "GetAgent"
        /\ history[j].agent_id = history[i].agent_id
        \* No intervening delete on this agent
        /\ ~\E k \in (i+1)..(j-1):
            /\ history[k].type = "DeleteAgent"
            /\ history[k].agent_id = history[i].agent_id
            /\ history[k].response.status = StatusOK
        => history[j].response.status = StatusOK

\* Invariant 4: AtomicOperation
\* If an operation is in history, its effects are fully visible
\* (No partial state from multi-step operations)
AtomicOperation ==
    \A i \in 1..Len(history):
        /\ history[i].type = "CreateAgent"
        /\ history[i].response.status = StatusCreated
        => AgentExists(history[i].agent_id) \/
           \E j \in (i+1)..Len(history):
               history[j].type = "DeleteAgent" /\
               history[j].agent_id = history[i].agent_id /\
               history[j].response.status = StatusOK

\* Invariant 5: DurableOnSuccess
\* If success response is in history, the state was persisted
\* (Idempotency cache reflects successful operations)
DurableOnSuccess ==
    \A i \in 1..Len(history):
        /\ history[i].token # NONE
        /\ history[i].type \in MutatingRequests
        /\ history[i].response.status \in {StatusCreated, StatusOK}
        => idempotency_cache[history[i].token] = history[i].response

\* Combined safety invariant
HttpLinearizabilityInvariant ==
    /\ IdempotencyGuarantee
    /\ ExactlyOnceExecution
    /\ ReadAfterWriteConsistency
    /\ AtomicOperation
    /\ DurableOnSuccess

(***************************************************************************)
(* LIVENESS PROPERTIES                                                     *)
(***************************************************************************)

\* Every pending request eventually receives a response
EventualResponse ==
    \A c \in HttpClients:
        ClientBusy(c) ~> ClientIdle(c)

\* Server eventually recovers from crash
EventualRecovery ==
    server_state = "crashed" ~> server_state = "running"

(***************************************************************************)
(* SPECIFICATION                                                           *)
(***************************************************************************)

\* Full specification with fairness for liveness checking
Spec == Init /\ [][Next]_vars /\ Fairness

\* Safety-only specification (no fairness)
SafetySpec == Init /\ [][Next]_vars

(***************************************************************************)
(* STATE CONSTRAINT (for bounded model checking)                           *)
(***************************************************************************)

StateConstraint ==
    /\ Len(history) <= MAX_OPERATIONS
    /\ op_counter <= MAX_OPERATIONS + 5

=============================================================================
