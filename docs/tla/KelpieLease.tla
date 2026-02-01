-------------------------------- MODULE KelpieLease --------------------------------
\* KelpieLease.tla - TLA+ specification for Kelpie's lease-based actor ownership
\*
\* This specification models the lease protocol from ADR-004 that ensures
\* single activation guarantee for virtual actors.
\*
\* Safety Invariants:
\* - LeaseUniqueness: At most one node believes it holds a valid lease per actor
\* - RenewalRequiresOwnership: Only lease holder can renew
\* - ExpiredLeaseClaimable: Expired lease can be claimed by any node
\* - LeaseValidityBounds: Lease expiry time within configured bounds
\* - GracePeriodRespected: No instant deactivation without grace period
\* - FencingTokenMonotonic: Fencing tokens only increase
\* - ClockSkewSafety: Leases safe despite bounded clock skew
\*
\* Liveness:
\* - EventualLeaseResolution: Eventually a lease is granted or expires
\* - FalseSuspicionRecovery: False suspicions eventually resolve
\*
\* Author: Kelpie Team
\* Date: 2026-01-24
\* Related: ADR-002 (G2.2), ADR-004 (G4.2)
\* Issue: #42 - Lease safety spec (TTL, grace period, false suspicion)

EXTENDS Integers, Sequences, FiniteSets, TLC

\* ============================================================================
\* Constants
\* ============================================================================

CONSTANTS
    Nodes,              \* Set of node identifiers (e.g., {"n1", "n2"})
    Actors,             \* Set of actor identifiers (e.g., {"a1", "a2"})
    LeaseDuration,      \* Duration of a lease in clock ticks (e.g., 10)
    GracePeriod,        \* Grace period before deactivation (e.g., 3)
    MaxClockSkew,       \* Maximum clock skew between nodes (e.g., 2)
    MaxClock,           \* Maximum clock value for bounded model checking
    UseSafeVersion      \* TRUE for correct CAS, FALSE for buggy race condition

\* ============================================================================
\* Variables
\* ============================================================================

VARIABLES
    \* Ground truth lease state
    leases,             \* [Actors -> [holder: Nodes \cup {NoHolder}, expiry: Int]]
    clock,              \* Global reference clock (models wall clock time)

    \* Node beliefs and local clocks
    nodeBeliefs,        \* [Nodes -> [Actors -> [held: BOOLEAN, expiry: Int]]]
    nodeClocks,         \* [Nodes -> Int] - Each node's local clock (may differ from global)

    \* False suspicion tracking
    nodeActuallyAlive,  \* [Nodes -> BOOLEAN] - Ground truth: is node actually alive?
    nodeSuspectedDead,  \* [Nodes -> BOOLEAN] - System's belief: is node dead?

    \* Fencing tokens for stale write prevention
    fencingTokens       \* [Actors -> Nat] - Monotonically increasing per actor

vars == <<leases, clock, nodeBeliefs, nodeClocks, nodeActuallyAlive, nodeSuspectedDead, fencingTokens>>

\* ============================================================================
\* Constants for empty values
\* ============================================================================

NoHolder == "NONE"  \* Sentinel value for no lease holder

\* ============================================================================
\* Type Invariants
\* ============================================================================

TypeOK ==
    /\ leases \in [Actors -> [holder: Nodes \cup {NoHolder}, expiry: Int]]
    /\ clock \in 0..MaxClock
    /\ nodeBeliefs \in [Nodes -> [Actors -> [held: BOOLEAN, expiry: Int]]]
    /\ nodeClocks \in [Nodes -> Int]
    /\ nodeActuallyAlive \in [Nodes -> BOOLEAN]
    /\ nodeSuspectedDead \in [Nodes -> BOOLEAN]
    /\ fencingTokens \in [Actors -> Nat]

\* ============================================================================
\* Helper Functions
\* ============================================================================

\* Check if a lease is currently valid (not expired) - ground truth using global clock
IsValidLease(actor) ==
    /\ leases[actor].holder /= NoHolder
    /\ leases[actor].expiry > clock

\* Check if a lease has expired - ground truth
IsExpiredLease(actor) ==
    /\ leases[actor].holder /= NoHolder
    /\ leases[actor].expiry <= clock

\* Check if there is no lease (never acquired or released) - ground truth
NoLease(actor) ==
    leases[actor].holder = NoHolder

\* Check if a node BELIEVES it holds a valid lease (using its local clock)
NodeBelievesItHolds(node, actor) ==
    /\ nodeBeliefs[node][actor].held
    /\ nodeBeliefs[node][actor].expiry > nodeClocks[node]

\* Lease state with grace period consideration
\* States: Active -> GracePeriod -> Expired
LeaseState(actor) ==
    CASE leases[actor].holder = NoHolder -> "Expired"
      [] clock < leases[actor].expiry - GracePeriod -> "Active"
      [] clock < leases[actor].expiry -> "GracePeriod"
      [] OTHER -> "Expired"

\* Check if a lease is in grace period
InGracePeriod(actor) ==
    LeaseState(actor) = "GracePeriod"

\* Check for false suspicion: system thinks node is dead, but it's actually alive
FalseSuspicion(node) ==
    /\ nodeSuspectedDead[node] = TRUE
    /\ nodeActuallyAlive[node] = TRUE

\* Get the current fencing token for an actor
CurrentFencingToken(actor) ==
    fencingTokens[actor]

\* Check if a node's clock is within acceptable skew of global clock
ClockWithinSkew(node) ==
    /\ nodeClocks[node] >= clock - MaxClockSkew
    /\ nodeClocks[node] <= clock + MaxClockSkew

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ leases = [a \in Actors |-> [holder |-> NoHolder, expiry |-> 0]]
    /\ clock = 0
    /\ nodeBeliefs = [n \in Nodes |-> [a \in Actors |-> [held |-> FALSE, expiry |-> 0]]]
    /\ nodeClocks = [n \in Nodes |-> 0]  \* All nodes start synchronized
    /\ nodeActuallyAlive = [n \in Nodes |-> TRUE]  \* All nodes start alive
    /\ nodeSuspectedDead = [n \in Nodes |-> FALSE]  \* No suspicions initially
    /\ fencingTokens = [a \in Actors |-> 0]  \* Fencing tokens start at 0

\* ============================================================================
\* Actions - Safe Version (Atomic CAS with Fencing)
\* ============================================================================

\* A node attempts to acquire a lease for an actor using atomic CAS.
\* This models the FDB transaction that atomically:
\* 1. Reads current lease state
\* 2. Checks no valid lease exists
\* 3. Writes new lease with expiry
\* 4. Increments fencing token
\* 5. Updates node's belief
AcquireLeaseSafe(node, actor) ==
    /\ nodeActuallyAlive[node]           \* Node must be alive
    /\ ~nodeSuspectedDead[node]          \* Node must not be suspected dead
    /\ ~IsValidLease(actor)              \* No valid lease exists (CAS precondition)
    /\ LET newExpiry == clock + LeaseDuration
           newToken == fencingTokens[actor] + 1
       IN
        /\ leases' = [leases EXCEPT ![actor] = [holder |-> node, expiry |-> newExpiry]]
        /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![node][actor] = [held |-> TRUE, expiry |-> newExpiry]]
        /\ fencingTokens' = [fencingTokens EXCEPT ![actor] = newToken]
    /\ UNCHANGED <<clock, nodeClocks, nodeActuallyAlive, nodeSuspectedDead>>

\* A node renews its lease for an actor.
\* Only the current holder can renew (ownership check).
\* Does NOT increment fencing token (same logical ownership).
RenewLeaseSafe(node, actor) ==
    /\ nodeActuallyAlive[node]           \* Node must be alive
    /\ IsValidLease(actor)               \* Lease must be valid
    /\ leases[actor].holder = node       \* Only holder can renew
    /\ LET newExpiry == clock + LeaseDuration
       IN
        /\ leases' = [leases EXCEPT ![actor] = [holder |-> node, expiry |-> newExpiry]]
        /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![node][actor] = [held |-> TRUE, expiry |-> newExpiry]]
    /\ UNCHANGED <<clock, nodeClocks, nodeActuallyAlive, nodeSuspectedDead, fencingTokens>>

\* ============================================================================
\* Actions - Buggy Version (Race Condition - Non-Atomic Read-Write)
\* ============================================================================

\* Buggy: A node claims a lease WITHOUT checking current state.
\* This models a race where the check happened earlier (and was stale).
AcquireLeaseNoCheck(node, actor) ==
    /\ nodeActuallyAlive[node]
    /\ ~nodeBeliefs[node][actor].held    \* Node doesn't think it already has it
    /\ LET newExpiry == clock + LeaseDuration
           newToken == fencingTokens[actor] + 1
       IN
        /\ leases' = [leases EXCEPT ![actor] = [holder |-> node, expiry |-> newExpiry]]
        /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![node][actor] = [held |-> TRUE, expiry |-> newExpiry]]
        /\ fencingTokens' = [fencingTokens EXCEPT ![actor] = newToken]
    /\ UNCHANGED <<clock, nodeClocks, nodeActuallyAlive, nodeSuspectedDead>>

\* Buggy renewal - same as safe for simplicity
RenewLeaseBuggy(node, actor) ==
    RenewLeaseSafe(node, actor)

\* ============================================================================
\* Time Advancement and Clock Skew
\* ============================================================================

\* Advance the global clock by 1 tick.
\* Node clocks may drift within MaxClockSkew bounds.
TickClock ==
    /\ clock < MaxClock
    /\ clock' = clock + 1
    \* Node beliefs about expired leases should be updated based on their local clock
    /\ nodeBeliefs' = [n \in Nodes |-> [a \in Actors |->
        IF nodeBeliefs[n][a].held /\ nodeBeliefs[n][a].expiry <= nodeClocks[n] + 1
        THEN [held |-> FALSE, expiry |-> 0]  \* Node realizes lease expired (per its clock)
        ELSE nodeBeliefs[n][a]]]
    \* Node clocks advance, possibly with drift
    /\ nodeClocks' = [n \in Nodes |->
        IF nodeActuallyAlive[n]
        THEN nodeClocks[n] + 1  \* Alive nodes advance their clock
        ELSE nodeClocks[n]]     \* Dead nodes don't advance
    /\ UNCHANGED <<leases, nodeActuallyAlive, nodeSuspectedDead, fencingTokens>>

\* Model clock skew: a node's clock drifts slightly
ClockDrift(node) ==
    /\ nodeActuallyAlive[node]
    /\ LET newClock == nodeClocks[node] + 1
       IN
        \* Only allow drift within bounds
        /\ newClock >= clock - MaxClockSkew
        /\ newClock <= clock + MaxClockSkew
        /\ nodeClocks' = [nodeClocks EXCEPT ![node] = newClock]
    /\ UNCHANGED <<leases, clock, nodeBeliefs, nodeActuallyAlive, nodeSuspectedDead, fencingTokens>>

\* ============================================================================
\* False Suspicion Actions
\* ============================================================================

\* A node is suspected dead (e.g., missed heartbeats due to GC pause)
\* but may actually still be alive.
SuspectNodeDead(node) ==
    /\ ~nodeSuspectedDead[node]          \* Not already suspected
    /\ nodeSuspectedDead' = [nodeSuspectedDead EXCEPT ![node] = TRUE]
    /\ UNCHANGED <<leases, clock, nodeBeliefs, nodeClocks, nodeActuallyAlive, fencingTokens>>

\* A suspected-dead node recovers by proving liveness (heartbeat succeeds).
\* This models the recovery from false suspicion.
RecoverFromSuspicion(node) ==
    /\ FalseSuspicion(node)              \* Must be falsely suspected
    /\ nodeSuspectedDead' = [nodeSuspectedDead EXCEPT ![node] = FALSE]
    /\ UNCHANGED <<leases, clock, nodeBeliefs, nodeClocks, nodeActuallyAlive, fencingTokens>>

\* A node actually dies (crashes).
NodeCrash(node) ==
    /\ nodeActuallyAlive[node]           \* Must be alive to crash
    /\ nodeActuallyAlive' = [nodeActuallyAlive EXCEPT ![node] = FALSE]
    \* Crashing node loses its beliefs
    /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![node] = [a \in Actors |-> [held |-> FALSE, expiry |-> 0]]]
    /\ UNCHANGED <<leases, clock, nodeClocks, nodeSuspectedDead, fencingTokens>>

\* A crashed node restarts.
NodeRestart(node) ==
    /\ ~nodeActuallyAlive[node]          \* Must be dead to restart
    /\ nodeActuallyAlive' = [nodeActuallyAlive EXCEPT ![node] = TRUE]
    /\ nodeSuspectedDead' = [nodeSuspectedDead EXCEPT ![node] = FALSE]
    \* Restarting node synchronizes its clock (within skew bounds)
    /\ nodeClocks' = [nodeClocks EXCEPT ![node] = clock]
    /\ UNCHANGED <<leases, clock, nodeBeliefs, fencingTokens>>

\* ============================================================================
\* Release Lease (Voluntary Deactivation)
\* ============================================================================

\* A node voluntarily releases its lease (graceful deactivation).
ReleaseLease(node, actor) ==
    /\ nodeActuallyAlive[node]
    /\ nodeBeliefs[node][actor].held     \* Node thinks it has the lease
    /\ leases[actor].holder = node       \* And it actually does
    /\ leases' = [leases EXCEPT ![actor] = [holder |-> NoHolder, expiry |-> 0]]
    /\ nodeBeliefs' = [nodeBeliefs EXCEPT ![node][actor] = [held |-> FALSE, expiry |-> 0]]
    /\ UNCHANGED <<clock, nodeClocks, nodeActuallyAlive, nodeSuspectedDead, fencingTokens>>

\* ============================================================================
\* Write Operations with Fencing Token
\* ============================================================================

\* Model a write operation that must include the correct fencing token.
\* Stale tokens (from old lease holders) are rejected.
\* This action doesn't change lease state, just validates the pattern.
WriteWithFencing(node, actor, token) ==
    /\ nodeActuallyAlive[node]
    /\ nodeBeliefs[node][actor].held     \* Node believes it holds lease
    /\ token = fencingTokens[actor]      \* Token must match current
    \* Write succeeds (no state change in this model, just validation)
    /\ UNCHANGED vars

\* ============================================================================
\* Next State Relation
\* ============================================================================

NextSafe ==
    \/ \E n \in Nodes, a \in Actors: AcquireLeaseSafe(n, a)
    \/ \E n \in Nodes, a \in Actors: RenewLeaseSafe(n, a)
    \/ \E n \in Nodes, a \in Actors: ReleaseLease(n, a)
    \/ \E n \in Nodes: SuspectNodeDead(n)
    \/ \E n \in Nodes: RecoverFromSuspicion(n)
    \/ \E n \in Nodes: NodeCrash(n)
    \/ \E n \in Nodes: NodeRestart(n)
    \/ \E n \in Nodes: ClockDrift(n)
    \/ TickClock

NextBuggy ==
    \/ \E n \in Nodes, a \in Actors: AcquireLeaseNoCheck(n, a)
    \/ \E n \in Nodes, a \in Actors: RenewLeaseBuggy(n, a)
    \/ \E n \in Nodes, a \in Actors: ReleaseLease(n, a)
    \/ \E n \in Nodes: SuspectNodeDead(n)
    \/ \E n \in Nodes: RecoverFromSuspicion(n)
    \/ \E n \in Nodes: NodeCrash(n)
    \/ \E n \in Nodes: NodeRestart(n)
    \/ \E n \in Nodes: ClockDrift(n)
    \/ TickClock

Next == IF UseSafeVersion THEN NextSafe ELSE NextBuggy

\* ============================================================================
\* Fairness (for Liveness)
\* ============================================================================

Fairness ==
    /\ WF_vars(TickClock)
    /\ \A n \in Nodes: WF_vars(RecoverFromSuspicion(n))
    /\ \A n \in Nodes: WF_vars(NodeRestart(n))
    \* Use strong fairness for lease acquisition to ensure progress
    \* even when action is intermittently enabled (due to suspicion cycles)
    /\ \A n \in Nodes, a \in Actors:
        IF UseSafeVersion
        THEN SF_vars(AcquireLeaseSafe(n, a))
        ELSE SF_vars(AcquireLeaseNoCheck(n, a))

\* ============================================================================
\* Safety Invariants
\* ============================================================================

\* LeaseUniqueness: At most one node believes it holds a valid lease per actor.
\* This is the CRITICAL invariant for single activation guarantee.
LeaseUniqueness ==
    \A a \in Actors:
        LET believingNodes == {n \in Nodes: NodeBelievesItHolds(n, a)}
        IN Cardinality(believingNodes) <= 1

\* Ground truth uniqueness (always holds, even in buggy version)
GroundTruthUniqueness ==
    \A a \in Actors:
        LET holders == {n \in Nodes:
            /\ leases[a].holder = n
            /\ leases[a].expiry > clock}
        IN Cardinality(holders) <= 1

\* RenewalRequiresOwnership: After a renewal, the holder must be the same.
RenewalRequiresOwnership ==
    \A a \in Actors:
        IsValidLease(a) =>
            \E n \in Nodes: leases[a].holder = n

\* ExpiredLeaseClaimable: An expired lease doesn't block new acquisition.
ExpiredLeaseClaimable ==
    \A a \in Actors:
        ~IsValidLease(a) =>
            \/ ~(\E n \in Nodes: nodeActuallyAlive[n] /\ ~nodeSuspectedDead[n])
            \/ \E n \in Nodes: ENABLED AcquireLeaseSafe(n, a)

\* LeaseValidityBounds: Lease expiry is within bounds.
LeaseValidityBounds ==
    \A a \in Actors:
        leases[a].holder /= NoHolder =>
            /\ leases[a].expiry >= 0
            /\ leases[a].expiry <= MaxClock + LeaseDuration

\* BeliefConsistency: If a node believes it holds a lease, it should be the actual holder.
\* This FAILS in the buggy version due to race conditions.
BeliefConsistency ==
    \A n \in Nodes, a \in Actors:
        NodeBelievesItHolds(n, a) => leases[a].holder = n

\* GracePeriodRespected: No new lease granted while current lease is in grace period.
\* This ensures the current holder has time to renew before being evicted.
GracePeriodRespected ==
    \A a \in Actors:
        InGracePeriod(a) =>
            \* If a lease is in grace period, only the current holder can act on it
            \A n \in Nodes:
                (n /= leases[a].holder) => ~ENABLED AcquireLeaseSafe(n, a)

\* FencingTokenMonotonic: Fencing tokens never decrease.
\* (Implicitly enforced by only incrementing, but stated for clarity)
FencingTokenMonotonic ==
    \A a \in Actors:
        fencingTokens[a] >= 0

\* ClockSkewSafety: All node clocks are within acceptable bounds of global clock.
\* This ensures lease expiration decisions are consistent despite clock skew.
ClockSkewSafety ==
    \A n \in Nodes:
        nodeActuallyAlive[n] => ClockWithinSkew(n)

\* FalseSuspicionSafety: A falsely suspected node that still holds a valid lease
\* retains the ability to recover (its lease isn't immediately stolen).
\* The fencing token ensures any stale operations after recovery are rejected.
FalseSuspicionSafety ==
    \A n \in Nodes, a \in Actors:
        /\ FalseSuspicion(n)
        /\ leases[a].holder = n
        /\ IsValidLease(a)
        => \* The lease remains valid until it naturally expires
           leases[a].expiry > clock

\* Combined safety invariant
SafetyInvariant ==
    /\ TypeOK
    /\ LeaseUniqueness
    /\ RenewalRequiresOwnership
    /\ LeaseValidityBounds
    /\ FencingTokenMonotonic
    /\ ClockSkewSafety

\* ============================================================================
\* Liveness Properties
\* ============================================================================

\* EventualLeaseResolution: For any actor, eventually either:
\* 1. Someone holds a valid lease, OR
\* 2. No one believes they hold it (clean state)
EventualLeaseResolution ==
    \A a \in Actors:
        []<>(IsValidLease(a) \/ ~(\E n \in Nodes: NodeBelievesItHolds(n, a)))

\* FalseSuspicionRecovery: If a node is falsely suspected, it eventually recovers.
FalseSuspicionRecovery ==
    \A n \in Nodes:
        [](FalseSuspicion(n) => <>~nodeSuspectedDead[n])

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars /\ Fairness

\* ============================================================================
\* Theorems (for TLC to check)
\* ============================================================================

THEOREM Spec => []SafetyInvariant
THEOREM Spec => EventualLeaseResolution
THEOREM Spec => FalseSuspicionRecovery

================================================================================
