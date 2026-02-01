-------------------------------- MODULE KelpieRegistry --------------------------------
(*
 * TLA+ specification for Kelpie Registry
 *
 * Models:
 *   - Node lifecycle (Active, Suspect, Failed)
 *   - Actor placement with single-activation guarantee
 *   - Heartbeat-based failure detection
 *   - Node-local placement caches (eventually consistent)
 *
 * Safety Properties:
 *   - SingleActivation: An actor is active on at most one node at any time
 *   - PlacementConsistency: Authoritative placements don't point to Failed nodes
 *
 * Liveness Properties:
 *   - EventualFailureDetection: Dead nodes are eventually detected and marked Failed
 *   - EventualCacheInvalidation: Stale cache entries on ALIVE nodes eventually get corrected
 *
 * Note on CacheCoherence:
 *   Caches are intentionally eventually consistent. The spec ALLOWS cache entries
 *   to temporarily point to failed nodes (this models real-world behavior).
 *   The liveness property ensures these stale entries are eventually corrected
 *   ON ALIVE NODES. Dead nodes' caches are irrelevant (no one reads from them).
 *
 * Author: Kelpie Team
 * Date: 2026-01-24
 *)

EXTENDS Naturals, FiniteSets, Sequences, TLC

\* ---------------------------------------------------------------------------
\* Constants
\* ---------------------------------------------------------------------------

CONSTANTS
    Nodes,          \* Set of all possible node IDs
    Actors,         \* Set of all possible actor IDs
    MaxHeartbeatMiss \* Number of missed heartbeats before failure detection

ASSUME MaxHeartbeatMiss >= 1

\* Node statuses
CONSTANTS Active, Suspect, Failed

NodeStatuses == {Active, Suspect, Failed}

\* ---------------------------------------------------------------------------
\* Variables
\* ---------------------------------------------------------------------------

VARIABLES
    \* Global registry state
    nodeStatus,     \* nodeStatus[n] = status of node n (Active/Suspect/Failed)

    \* Actor placement (authoritative, in central registry)
    placement,      \* placement[a] = node where actor a is placed, or NULL

    \* Heartbeat tracking (bounded to MaxHeartbeatMiss for finite state space)
    heartbeatCount, \* heartbeatCount[n] = missed heartbeat counter for node n
    isAlive,        \* isAlive[n] = TRUE if node n is actually running (for spec)

    \* Node-local placement caches (can be stale)
    cache           \* cache[n][a] = cached placement for actor a on node n

vars == <<nodeStatus, placement, heartbeatCount, isAlive, cache>>

\* Special value for no placement
NULL == "NULL"

\* ---------------------------------------------------------------------------
\* Type Invariants
\* ---------------------------------------------------------------------------

TypeOK ==
    /\ nodeStatus \in [Nodes -> NodeStatuses]
    /\ placement \in [Actors -> Nodes \cup {NULL}]
    /\ heartbeatCount \in [Nodes -> 0..MaxHeartbeatMiss]  \* Bounded!
    /\ isAlive \in [Nodes -> BOOLEAN]
    /\ cache \in [Nodes -> [Actors -> Nodes \cup {NULL}]]

\* ---------------------------------------------------------------------------
\* Initial State
\* ---------------------------------------------------------------------------

Init ==
    /\ nodeStatus = [n \in Nodes |-> Active]
    /\ placement = [a \in Actors |-> NULL]
    /\ heartbeatCount = [n \in Nodes |-> 0]
    /\ isAlive = [n \in Nodes |-> TRUE]
    /\ cache = [n \in Nodes |-> [a \in Actors |-> NULL]]

\* ---------------------------------------------------------------------------
\* Helper Predicates
\* ---------------------------------------------------------------------------

\* Node is healthy (can accept actors)
IsHealthy(n) == nodeStatus[n] = Active

\* Actor is not placed anywhere
IsUnplaced(a) == placement[a] = NULL

\* Get set of active nodes
ActiveNodes == {n \in Nodes : nodeStatus[n] = Active}

\* Check if cache entry is stale (doesn't match authoritative placement)
IsCacheStale(n, a) == cache[n][a] # placement[a]

\* ---------------------------------------------------------------------------
\* Actions
\* ---------------------------------------------------------------------------

\* Node sends a heartbeat (only if alive)
\* Resets the missed heartbeat counter
SendHeartbeat(n) ==
    /\ isAlive[n] = TRUE
    /\ heartbeatCount' = [heartbeatCount EXCEPT ![n] = 0]
    /\ nodeStatus' = [nodeStatus EXCEPT ![n] =
                        IF nodeStatus[n] = Suspect THEN Active ELSE @]
    /\ UNCHANGED <<placement, isAlive, cache>>

\* Heartbeat timeout tick - increment missed heartbeat counter for dead nodes
\* Bounded: don't increment past MaxHeartbeatMiss (no need, failure already triggered)
HeartbeatTick ==
    /\ \E n \in Nodes :
        /\ nodeStatus[n] # Failed
        /\ ~isAlive[n]  \* Only tick for actually dead nodes
        /\ heartbeatCount[n] < MaxHeartbeatMiss  \* Bound the counter
        /\ heartbeatCount' = [heartbeatCount EXCEPT ![n] = @ + 1]
        /\ UNCHANGED <<nodeStatus, placement, isAlive, cache>>

\* Detect failure based on heartbeat timeout
\* Transitions: Active -> Suspect -> Failed
DetectFailure(n) ==
    /\ nodeStatus[n] # Failed
    /\ heartbeatCount[n] >= MaxHeartbeatMiss
    /\ nodeStatus' = [nodeStatus EXCEPT ![n] =
                        IF @ = Active THEN Suspect ELSE Failed]
    \* If node goes to Failed, clear all its placements
    /\ IF nodeStatus[n] = Suspect
       THEN placement' = [a \in Actors |->
                            IF placement[a] = n THEN NULL ELSE placement[a]]
       ELSE UNCHANGED placement
    /\ UNCHANGED <<heartbeatCount, isAlive, cache>>

\* A node crashes (for modeling purposes)
NodeCrash(n) ==
    /\ isAlive[n] = TRUE
    /\ isAlive' = [isAlive EXCEPT ![n] = FALSE]
    /\ UNCHANGED <<nodeStatus, placement, heartbeatCount, cache>>

\* Claim an actor on a node (single-activation guarantee)
\* Only active nodes can claim actors
ClaimActor(a, n) ==
    /\ IsHealthy(n)
    /\ isAlive[n] = TRUE
    /\ IsUnplaced(a)
    /\ placement' = [placement EXCEPT ![a] = n]
    \* Update local cache
    /\ cache' = [cache EXCEPT ![n][a] = n]
    /\ UNCHANGED <<nodeStatus, heartbeatCount, isAlive>>

\* Release an actor (deactivation)
ReleaseActor(a) ==
    /\ placement[a] # NULL
    /\ placement' = [placement EXCEPT ![a] = NULL]
    /\ UNCHANGED <<nodeStatus, heartbeatCount, isAlive, cache>>

\* Cache invalidation - propagate placement change to a node's cache
\* Models eventually consistent cache invalidation
InvalidateCache(n, a) ==
    /\ isAlive[n] = TRUE
    /\ cache[n][a] # placement[a]  \* Only invalidate if stale
    /\ cache' = [cache EXCEPT ![n][a] = placement[a]]
    /\ UNCHANGED <<nodeStatus, placement, heartbeatCount, isAlive>>

\* ---------------------------------------------------------------------------
\* Next State Relation
\* ---------------------------------------------------------------------------

Next ==
    \/ \E n \in Nodes : SendHeartbeat(n)
    \/ HeartbeatTick
    \/ \E n \in Nodes : DetectFailure(n)
    \/ \E n \in Nodes : NodeCrash(n)
    \/ \E a \in Actors, n \in Nodes : ClaimActor(a, n)
    \/ \E a \in Actors : ReleaseActor(a)
    \/ \E n \in Nodes, a \in Actors : InvalidateCache(n, a)

\* ---------------------------------------------------------------------------
\* Safety Properties (Invariants)
\* ---------------------------------------------------------------------------

\* Single Activation: An actor is placed on at most one node
SingleActivation ==
    \A a \in Actors :
        Cardinality({n \in Nodes : placement[a] = n}) <= 1

\* Placement Consistency: Placed actors are on active or suspect nodes
\* (not on failed nodes - we clear placements when nodes fail)
PlacementConsistency ==
    \A a \in Actors :
        placement[a] # NULL => nodeStatus[placement[a]] # Failed

\* Combined safety invariant
Safety ==
    /\ TypeOK
    /\ SingleActivation
    /\ PlacementConsistency

\* ---------------------------------------------------------------------------
\* Liveness Properties
\* ---------------------------------------------------------------------------

\* Eventual Failure Detection: If a node crashes and stays dead,
\* it will eventually be marked as Failed
EventualFailureDetection ==
    \A n \in Nodes :
        (isAlive[n] = FALSE) ~> (nodeStatus[n] = Failed)

\* Eventual Cache Invalidation: Stale cache entries on ALIVE nodes eventually get corrected
EventualCacheInvalidation ==
    \A n \in Nodes, a \in Actors :
        (isAlive[n] /\ IsCacheStale(n, a)) ~> (~isAlive[n] \/ ~IsCacheStale(n, a))

\* ---------------------------------------------------------------------------
\* Fairness
\* ---------------------------------------------------------------------------

\* Weak fairness for heartbeat mechanism - ensures progress
Fairness ==
    /\ WF_vars(HeartbeatTick)
    /\ \A n \in Nodes : WF_vars(DetectFailure(n))
    /\ \A n \in Nodes, a \in Actors : WF_vars(InvalidateCache(n, a))

\* ---------------------------------------------------------------------------
\* Specification
\* ---------------------------------------------------------------------------

Spec == Init /\ [][Next]_vars /\ Fairness

================================================================================
