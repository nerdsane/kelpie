--------------------------- MODULE KelpieClusterMembership ---------------------------
(*
 * TLA+ Specification for Kelpie Cluster Membership Protocol
 *
 * This specification models the cluster membership protocol including:
 * - Node join/leave operations
 * - Heartbeat-based failure detection
 * - Network partitions
 * - Membership view consistency
 *
 * Author: Kelpie Team
 * GitHub Issue: #11
 * Created: 2026-01-24
 *
 * TigerStyle: Explicit states, bounded operations, safety invariants.
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

\* Constants defined in .cfg file
CONSTANTS
    Nodes,          \* Set of all possible node IDs
    MaxViewNum      \* Maximum view number (bounds state space)

\* Use BUGGY_MODE constant to toggle between safe and buggy behavior
\* This is set in the .cfg file
CONSTANTS BUGGY_MODE

\* Node states
CONSTANTS
    Joining,
    Active,
    Leaving,
    Failed,
    Left

\* Special value for "no primary"
CONSTANTS NoPrimary

(*
 * Variables:
 * - nodeState: function from Nodes to their current state
 * - membershipView: function from Nodes to their view of active members
 * - viewNum: function from Nodes to their view number
 * - believesPrimary: function from Nodes to BOOLEAN - does this node believe it's primary?
 * - primaryTerm: function from Nodes to their primary term (epoch number)
 *               Higher term = newer primary claim. Used for conflict resolution.
 * - partitioned: set of pairs (n1, n2) where communication is blocked
 * - heartbeatReceived: function tracking which nodes received heartbeats
 *)
VARIABLES
    nodeState,
    membershipView,
    viewNum,
    believesPrimary,
    primaryTerm,
    partitioned,
    heartbeatReceived

vars == <<nodeState, membershipView, viewNum, believesPrimary, primaryTerm, partitioned, heartbeatReceived>>

\* ============================================================================
\* Helper Operators
\* ============================================================================

\* Maximum of a set of integers
Max(S) == CHOOSE x \in S : \A y \in S : x >= y

\* Check if two nodes can communicate (not partitioned)
CanCommunicate(n1, n2) ==
    /\ <<n1, n2>> \notin partitioned
    /\ <<n2, n1>> \notin partitioned

\* Get the set of active nodes from a node's view
ActiveInView(n) == membershipView[n]

\* Get all nodes that are actually in Active state
ActuallyActive == {n \in Nodes : nodeState[n] = Active}

\* Check if any node can reach a primary (used for fencing)
\* A node m "knows about a primary" if m can communicate with any primary
KnowsAboutPrimary(m) ==
    \E p \in Nodes :
        /\ believesPrimary[p]
        /\ (CanCommunicate(m, p) \/ m = p)

\* Check if a node has a valid (non-stale) primary claim
\* A primary claim is valid only if:
\* 1. The primary can still reach a majority
\* 2. The primary has the highest term among all primaries (Raft-style terms)
HasValidPrimaryClaim(p) ==
    /\ believesPrimary[p]
    /\ nodeState[p] = Active
    /\ LET clusterSize == Cardinality(Nodes)
           reachableByP == {m \in Nodes : CanCommunicate(p, m) \/ m = p}
           reachableActiveByP == {m \in reachableByP : nodeState[m] = Active}
           hasMajority == 2 * Cardinality(reachableActiveByP) > clusterSize
           \* Check that p has highest term among all primaries
           allPrimaries == {n \in Nodes : believesPrimary[n]}
           hasHighestTerm == \A q \in allPrimaries : primaryTerm[p] >= primaryTerm[q]
       IN hasMajority /\ hasHighestTerm

\* Check if a node can be elected as primary
\* Safe version: requires majority of the ENTIRE cluster to be reachable
\*               AND no node anywhere has a valid primary claim
\* Buggy version: can become primary without checking
CanBecomePrimary(n) ==
    IF BUGGY_MODE THEN
        \* BUGGY: Node can declare itself primary without quorum check
        nodeState[n] = Active
    ELSE
        \* SAFE: Node needs to:
        \* 1. Be active
        \* 2. Be able to reach a majority of ALL nodes in the cluster
        \*    (not just its view - prevents shrinking quorum attack)
        \* 3. No node anywhere in the cluster has a valid primary claim
        \*    (a claim is valid only if that node can still reach majority)
        \*    This prevents split-brain: minority primary must step down first
        LET clusterSize == Cardinality(Nodes)
            reachableNodes == {m \in Nodes : CanCommunicate(n, m) \/ m = n}
            reachableActive == {m \in reachableNodes : nodeState[m] = Active}
            reachableCount == Cardinality(reachableActive)
            \* Global check: no valid primary exists anywhere
            existsValidPrimary == \E p \in Nodes : HasValidPrimaryClaim(p)
        IN /\ nodeState[n] = Active
           /\ 2 * reachableCount > clusterSize  \* Majority of cluster reachable
           /\ ~existsValidPrimary                \* No valid primary anywhere in cluster

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ nodeState \in [Nodes -> {Joining, Active, Leaving, Failed, Left}]
    /\ membershipView \in [Nodes -> SUBSET Nodes]
    /\ viewNum \in [Nodes -> 0..MaxViewNum]
    /\ believesPrimary \in [Nodes -> BOOLEAN]
    /\ primaryTerm \in [Nodes -> 0..MaxViewNum]  \* Term bounded same as viewNum
    /\ partitioned \subseteq (Nodes \X Nodes)
    /\ heartbeatReceived \in [Nodes -> BOOLEAN]

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ nodeState = [n \in Nodes |-> Left]
    /\ membershipView = [n \in Nodes |-> {}]
    /\ viewNum = [n \in Nodes |-> 0]
    /\ believesPrimary = [n \in Nodes |-> FALSE]
    /\ primaryTerm = [n \in Nodes |-> 0]
    /\ partitioned = {}
    /\ heartbeatReceived = [n \in Nodes |-> FALSE]

\* ============================================================================
\* Actions
\* ============================================================================

(*
 * NodeJoin: A node requests to join the cluster
 *
 * Preconditions:
 * - Node is in Left state
 * - There's at least one active node to contact (or this is first node)
 *
 * Effects:
 * - Node transitions to Joining state
 * - If first node, immediately becomes Active
 *)
NodeJoin(n) ==
    /\ nodeState[n] = Left
    /\ IF ActuallyActive = {} THEN
           \* First node - join immediately as Active and becomes primary with term 1
           /\ nodeState' = [nodeState EXCEPT ![n] = Active]
           /\ membershipView' = [membershipView EXCEPT ![n] = {n}]
           /\ viewNum' = [viewNum EXCEPT ![n] = 1]
           /\ believesPrimary' = [believesPrimary EXCEPT ![n] = TRUE]
           /\ primaryTerm' = [primaryTerm EXCEPT ![n] = 1]
       ELSE
           \* Not first - need to coordinate with active nodes
           /\ nodeState' = [nodeState EXCEPT ![n] = Joining]
           /\ UNCHANGED <<membershipView, viewNum, believesPrimary, primaryTerm>>
    /\ UNCHANGED <<partitioned, heartbeatReceived>>

(*
 * NodeJoinComplete: A joining node becomes fully active
 *
 * Preconditions:
 * - Node is in Joining state
 * - Can communicate with at least one active node
 *
 * Effects:
 * - Node becomes Active
 * - Joins the membership view
 *)
NodeJoinComplete(n) ==
    /\ nodeState[n] = Joining
    /\ \E active \in ActuallyActive : CanCommunicate(n, active)
    /\ LET newView == ActuallyActive \union {n}
           currentMax == Max({viewNum[m] : m \in ActuallyActive})
           newViewNum == 1 + currentMax
       IN
           /\ newViewNum <= MaxViewNum  \* Guard: don't exceed bound
           /\ nodeState' = [nodeState EXCEPT ![n] = Active]
           /\ membershipView' = [m \in Nodes |->
                   IF m = n THEN newView
                   ELSE IF nodeState[m] = Active /\ CanCommunicate(n, m)
                        THEN newView
                        ELSE membershipView[m]]
           /\ viewNum' = [m \in Nodes |->
                   IF m = n \/ (nodeState[m] = Active /\ CanCommunicate(n, m))
                   THEN newViewNum
                   ELSE viewNum[m]]
    /\ UNCHANGED <<believesPrimary, primaryTerm, partitioned, heartbeatReceived>>

(*
 * NodeLeave: A node gracefully leaves the cluster
 *
 * Preconditions:
 * - Node is in Active state
 *
 * Effects:
 * - Node transitions to Leaving state
 *)
NodeLeave(n) ==
    /\ nodeState[n] = Active
    /\ nodeState' = [nodeState EXCEPT ![n] = Leaving]
    \* If this node believes it's primary, it resigns
    /\ believesPrimary' = [believesPrimary EXCEPT ![n] = FALSE]
    /\ UNCHANGED <<membershipView, viewNum, primaryTerm, partitioned, heartbeatReceived>>

(*
 * NodeLeaveComplete: A leaving node finishes leaving
 *
 * Preconditions:
 * - Node is in Leaving state
 *
 * Effects:
 * - Node becomes Left
 * - Other nodes update their views
 *)
NodeLeaveComplete(n) ==
    /\ nodeState[n] = Leaving
    \* Guard: ensure we don't exceed MaxViewNum
    /\ \A m \in Nodes : nodeState[m] = Active /\ m # n => viewNum[m] < MaxViewNum
    /\ nodeState' = [nodeState EXCEPT ![n] = Left]
    /\ LET newView == ActuallyActive \ {n}
       IN membershipView' = [m \in Nodes |->
               IF nodeState[m] = Active /\ m # n
               THEN newView
               ELSE IF m = n THEN {}
               ELSE membershipView[m]]
    /\ viewNum' = [m \in Nodes |->
           IF nodeState[m] = Active /\ m # n
           THEN viewNum[m] + 1
           ELSE viewNum[m]]
    /\ UNCHANGED <<believesPrimary, primaryTerm, partitioned, heartbeatReceived>>

(*
 * SendHeartbeat: An active node sends heartbeat
 *
 * Effects:
 * - Nodes that can communicate receive heartbeat
 *)
SendHeartbeat(n) ==
    /\ nodeState[n] = Active
    /\ heartbeatReceived' = [m \in Nodes |->
           IF m = n \/ (nodeState[m] \in {Active, Leaving} /\ CanCommunicate(n, m))
           THEN TRUE
           ELSE heartbeatReceived[m]]
    /\ UNCHANGED <<nodeState, membershipView, viewNum, believesPrimary, primaryTerm, partitioned>>

(*
 * DetectFailure: A node detects another node as failed (non-deterministic)
 *
 * This models timeout-based failure detection. In reality, this would
 * happen after missing heartbeats; here we model it non-deterministically.
 *
 * Preconditions:
 * - Detecting node is Active
 * - Target node is Active but hasn't received heartbeat (simulating timeout)
 * - Nodes cannot communicate (simulating partition or crash)
 *
 * Effects:
 * - Target marked as Failed from detector's perspective
 *)
DetectFailure(detector, target) ==
    /\ nodeState[detector] = Active
    /\ nodeState[target] = Active
    /\ target \in membershipView[detector]
    /\ ~CanCommunicate(detector, target)  \* Can't reach the node
    /\ ~heartbeatReceived[target]         \* No recent heartbeat
    /\ viewNum[detector] < MaxViewNum     \* Guard: don't exceed bound
    \* Update membership view to remove failed node (but always keep self)
    /\ LET newView == (membershipView[detector] \ {target}) \union {detector}
       IN membershipView' = [membershipView EXCEPT ![detector] = newView]
    /\ viewNum' = [viewNum EXCEPT ![detector] = viewNum[detector] + 1]
    /\ UNCHANGED <<nodeState, believesPrimary, primaryTerm, partitioned, heartbeatReceived>>

(*
 * MarkNodeFailed: System marks a node as failed
 *
 * This happens when enough nodes detect failure.
 *)
MarkNodeFailed(n) ==
    /\ nodeState[n] = Active
    \* At least one active node has removed n from its view
    /\ \E detector \in ActuallyActive :
           /\ detector # n
           /\ n \notin membershipView[detector]
    /\ nodeState' = [nodeState EXCEPT ![n] = Failed]
    /\ membershipView' = [membershipView EXCEPT ![n] = {}]  \* Clear view on failure
    /\ viewNum' = [viewNum EXCEPT ![n] = 0]                  \* Reset view number
    /\ believesPrimary' = [believesPrimary EXCEPT ![n] = FALSE]  \* Lose primary status
    /\ primaryTerm' = [primaryTerm EXCEPT ![n] = 0]           \* Reset term
    /\ UNCHANGED <<partitioned, heartbeatReceived>>

(*
 * NodeRecover: A failed node recovers and rejoins
 *)
NodeRecover(n) ==
    /\ nodeState[n] = Failed
    /\ nodeState' = [nodeState EXCEPT ![n] = Left]
    /\ membershipView' = [membershipView EXCEPT ![n] = {}]
    /\ viewNum' = [viewNum EXCEPT ![n] = 0]
    /\ primaryTerm' = [primaryTerm EXCEPT ![n] = 0]
    /\ UNCHANGED <<believesPrimary, partitioned, heartbeatReceived>>

(*
 * ElectPrimary: An active node becomes primary
 *
 * The CanBecomePrimary function handles the Safe vs Buggy logic:
 * - Safe: requires quorum and no existing primary in reachable set
 * - Buggy: just needs to be active
 *
 * When becoming primary, the node gets a new term higher than all existing terms.
 * This ensures newer primaries always have precedence (Raft-style epochs).
 *)
ElectPrimary(n) ==
    /\ ~believesPrimary[n]  \* Not already primary
    /\ CanBecomePrimary(n)
    /\ IF BUGGY_MODE THEN
           \* BUGGY: Don't increment term - allows multiple primaries with same term
           /\ believesPrimary' = [believesPrimary EXCEPT ![n] = TRUE]
           /\ UNCHANGED primaryTerm
       ELSE
           \* SAFE: Increment term to establish ordering
           LET maxTerm == Max({primaryTerm[m] : m \in Nodes})
               newTerm == maxTerm + 1
           IN
               /\ newTerm <= MaxViewNum  \* Guard: don't exceed bound
               /\ believesPrimary' = [believesPrimary EXCEPT ![n] = TRUE]
               /\ primaryTerm' = [primaryTerm EXCEPT ![n] = newTerm]
    /\ UNCHANGED <<nodeState, membershipView, viewNum, partitioned, heartbeatReceived>>

(*
 * PrimaryStepDown: A primary loses quorum and must step down
 *
 * This is CRITICAL for preventing split-brain during partitions:
 * - A primary continuously monitors whether it can reach a majority
 * - If it loses quorum (e.g., due to partition), it MUST step down
 * - This allows the majority partition to safely elect a new primary
 *
 * In BUGGY_MODE, primaries never step down (they don't check quorum),
 * which allows split-brain when combined with ElectPrimary.
 *)
PrimaryStepDown(n) ==
    /\ believesPrimary[n]     \* Must be primary
    /\ nodeState[n] = Active  \* Must be active
    /\ ~BUGGY_MODE            \* Only in safe mode - buggy mode never steps down
    /\ LET clusterSize == Cardinality(Nodes)
           reachableNodes == {m \in Nodes : CanCommunicate(n, m) \/ m = n}
           reachableActive == {m \in reachableNodes : nodeState[m] = Active}
           reachableCount == Cardinality(reachableActive)
       IN 2 * reachableCount <= clusterSize  \* Lost majority - must step down
    /\ believesPrimary' = [believesPrimary EXCEPT ![n] = FALSE]
    /\ UNCHANGED <<nodeState, membershipView, viewNum, primaryTerm, partitioned, heartbeatReceived>>

(*
 * CreatePartition: Network partition occurs between two nodes
 *)
CreatePartition(n1, n2) ==
    /\ n1 # n2
    /\ <<n1, n2>> \notin partitioned
    /\ partitioned' = partitioned \union {<<n1, n2>>, <<n2, n1>>}
    /\ UNCHANGED <<nodeState, membershipView, viewNum, believesPrimary, primaryTerm, heartbeatReceived>>

(*
 * HealPartition: Network partition heals
 *
 * In safe mode, if healing creates a situation where two primaries
 * can communicate, one must step down atomically. This models the
 * real-world behavior where connection restoration triggers immediate
 * leader conflict detection and resolution.
 *
 * In buggy mode, split-brain persists after heal.
 *)
HealPartition(n1, n2) ==
    /\ <<n1, n2>> \in partitioned
    /\ LET newPartitioned == partitioned \ {<<n1, n2>>, <<n2, n1>>}
           \* After healing, check if n1 and n2 can communicate
           \* (they can if no other partition separates them)
           canCommunicateAfterHeal ==
               /\ <<n1, n2>> \notin newPartitioned
               /\ <<n2, n1>> \notin newPartitioned
           \* Check if both believe they're primary
           bothPrimary == believesPrimary[n1] /\ believesPrimary[n2]
           \* Deterministic choice: pick one to step down using CHOOSE
           \* (will consistently pick the same one given the same inputs)
           nodeToStepDown == CHOOSE x \in {n1, n2} : TRUE
       IN
           /\ partitioned' = newPartitioned
           /\ IF ~BUGGY_MODE /\ canCommunicateAfterHeal /\ bothPrimary
              THEN
                  \* Safe mode: resolve split-brain atomically
                  believesPrimary' = [believesPrimary EXCEPT
                      ![nodeToStepDown] = FALSE]
              ELSE
                  UNCHANGED believesPrimary
    /\ UNCHANGED <<nodeState, membershipView, viewNum, primaryTerm, heartbeatReceived>>

(*
 * ResetHeartbeats: Clear heartbeat flags (models heartbeat interval)
 *)
ResetHeartbeats ==
    /\ heartbeatReceived' = [n \in Nodes |-> FALSE]
    /\ UNCHANGED <<nodeState, membershipView, viewNum, believesPrimary, primaryTerm, partitioned>>

(*
 * SyncViews: Active nodes synchronize membership views when they can communicate
 *
 * This models view synchronization that should happen in the safe protocol.
 * After syncing, both nodes should be in the view (they're both active and
 * can communicate).
 *)
SyncViews(n1, n2) ==
    /\ nodeState[n1] = Active
    /\ nodeState[n2] = Active
    /\ CanCommunicate(n1, n2)
    /\ viewNum[n1] # viewNum[n2]  \* Views differ
    /\ LET higherViewNum == Max({viewNum[n1], viewNum[n2]})
       IN
           /\ higherViewNum < MaxViewNum  \* Guard: don't exceed bound
           /\ LET nodeWithHigherView == IF viewNum[n1] > viewNum[n2] THEN n1 ELSE n2
                  nodeWithLowerView == IF viewNum[n1] > viewNum[n2] THEN n2 ELSE n1
                  \* Merge views: take higher view but ensure both communicating nodes are included
                  mergedView == membershipView[nodeWithHigherView] \union {n1, n2}
              IN
                  /\ membershipView' = [membershipView EXCEPT
                         ![nodeWithLowerView] = mergedView,
                         ![nodeWithHigherView] = mergedView]
                  /\ viewNum' = [viewNum EXCEPT
                         ![nodeWithLowerView] = higherViewNum + 1,
                         ![nodeWithHigherView] = higherViewNum + 1]
    /\ UNCHANGED <<nodeState, believesPrimary, primaryTerm, partitioned, heartbeatReceived>>

\* ============================================================================
\* Next State Relation
\* ============================================================================

Next ==
    \/ \E n \in Nodes : NodeJoin(n)
    \/ \E n \in Nodes : NodeJoinComplete(n)
    \/ \E n \in Nodes : NodeLeave(n)
    \/ \E n \in Nodes : NodeLeaveComplete(n)
    \/ \E n \in Nodes : SendHeartbeat(n)
    \/ \E d, t \in Nodes : d # t /\ DetectFailure(d, t)
    \/ \E n \in Nodes : MarkNodeFailed(n)
    \/ \E n \in Nodes : NodeRecover(n)
    \/ \E n \in Nodes : ElectPrimary(n)
    \/ \E n \in Nodes : PrimaryStepDown(n)
    \/ \E n1, n2 \in Nodes : n1 # n2 /\ CreatePartition(n1, n2)
    \/ \E n1, n2 \in Nodes : HealPartition(n1, n2)
    \/ ResetHeartbeats
    \/ \E n1, n2 \in Nodes : n1 # n2 /\ SyncViews(n1, n2)

Spec == Init /\ [][Next]_vars

\* ============================================================================
\* Safety Invariants
\* ============================================================================

(*
 * MembershipConsistency: Active nodes that both include each other in their
 * views should have consistent views.
 *
 * Note: We relax this to only require consistency when nodes mutually recognize
 * each other. During partitions, views can diverge - that's expected. The key
 * safety property is that nodes don't take conflicting actions based on
 * inconsistent views (handled by NoSplitBrain and quorum requirements).
 *)
MembershipConsistency ==
    \A n1, n2 \in Nodes :
        /\ nodeState[n1] = Active
        /\ nodeState[n2] = Active
        /\ n1 \in membershipView[n2]
        /\ n2 \in membershipView[n1]
        /\ viewNum[n1] = viewNum[n2]
        => membershipView[n1] = membershipView[n2]

(*
 * JoinAtomicity: A joining node is either fully joined (Active with
 * non-empty view) or not yet joined (Joining/Left with empty view)
 *)
JoinAtomicity ==
    \A n \in Nodes :
        \/ (nodeState[n] = Active /\ membershipView[n] # {})
        \/ (nodeState[n] \in {Joining, Left, Failed} /\ membershipView[n] = {})
        \/ nodeState[n] = Leaving  \* May have any view during transition

(*
 * LeaveDetection: A failed/leaving node is not in any active node's
 * membership view (eventually - this is actually a liveness property,
 * but we check a weaker safety version)
 *)
LeaveDetectionWeak ==
    \A n \in Nodes :
        nodeState[n] = Left =>
            \A m \in Nodes : nodeState[m] = Active => n \notin membershipView[m]

(*
 * NoSplitBrain: There is at most one VALID primary node.
 *
 * This is the KEY SAFETY INVARIANT that the buggy version violates.
 *
 * A primary claim is "valid" only if the node can reach a majority.
 * A minority primary is "stale" - it cannot commit any operations
 * without quorum, so it poses no split-brain danger.
 *
 * In the safe version, a node only becomes primary if no valid primary
 * exists anywhere in the cluster.
 * In the buggy version, any active node can become primary without
 * checking quorum, allowing multiple valid primaries.
 *)
NoSplitBrain ==
    \A n1, n2 \in Nodes :
        /\ HasValidPrimaryClaim(n1)
        /\ HasValidPrimaryClaim(n2)
        => n1 = n2

\* Combined safety invariant
Safety ==
    /\ TypeOK
    /\ MembershipConsistency
    /\ JoinAtomicity
    /\ LeaveDetectionWeak
    /\ NoSplitBrain

\* ============================================================================
\* Liveness Properties
\* ============================================================================

(*
 * EventualMembershipConvergence: If the network heals (no partitions)
 * and nodes stop joining/leaving, all active nodes eventually have
 * the same membership view.
 *
 * This requires fairness assumptions.
 *)
FairSpec == Spec /\ WF_vars(Next)

\* Network is healed
NetworkHealed == partitioned = {}

\* No membership changes happening
Stable ==
    /\ \A n \in Nodes : nodeState[n] \in {Active, Left}
    /\ NetworkHealed

\* All active nodes have same view
Converged ==
    \A n1, n2 \in Nodes :
        /\ nodeState[n1] = Active
        /\ nodeState[n2] = Active
        => membershipView[n1] = membershipView[n2]

\* Liveness: Eventually converges when stable
EventualMembershipConvergence ==
    Stable ~> Converged

=============================================================================
