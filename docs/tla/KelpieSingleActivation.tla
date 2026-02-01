------------------------------ MODULE KelpieSingleActivation ------------------------------
(***************************************************************************)
(* TLA+ Specification for Kelpie Single Activation Guarantee                *)
(*                                                                          *)
(* Related ADRs:                                                            *)
(*   - docs/adr/001-virtual-actor-model.md (Single Activation Guarantee)   *)
(*   - docs/adr/004-linearizability-guarantees.md (CP Semantics)           *)
(*                                                                          *)
(* This spec models the distributed actor activation protocol ensuring:     *)
(* - SAFETY: At most one node can activate an actor at any time            *)
(* - LIVENESS: Every claim eventually results in activation or rejection   *)
(*                                                                          *)
(* Key insight: Single activation relies on FDB's optimistic concurrency   *)
(* control (OCC). A claim transaction reads the current holder, attempts   *)
(* to write itself as holder, and commits. If another node modified the    *)
(* key since the read, commit fails (conflict).                            *)
(*                                                                          *)
(* FDB Transaction Semantics Modeled:                                       *)
(* 1. Read phase: snapshot read captures current version                   *)
(* 2. Write phase: prepare mutation (not yet visible)                      *)
(* 3. Commit phase: atomic commit iff version unchanged, else abort        *)
(*                                                                          *)
(* TigerStyle: All constants have explicit units and bounds.               *)
(*                                                                          *)
(* DST Tests: crates/kelpie-dst/tests/single_activation_dst.rs             *)
(* - test_concurrent_activation_single_winner: N nodes race, exactly 1 wins*)
(* - test_concurrent_activation_with_*_faults: Invariant holds under faults*)
(* - test_single_activation_with_network_partition: Under network split    *)
(* - test_single_activation_with_crash_recovery: After crash and recovery  *)
(* - test_release_and_reactivation: Release followed by new claim          *)
(* - test_consistent_holder_invariant: Storage state matches node belief   *)
(***************************************************************************)

EXTENDS Integers, FiniteSets

(***************************************************************************)
(* CONSTANTS                                                                *)
(***************************************************************************)

CONSTANT
    Nodes,          \* Set of nodes that can activate actors (e.g., {n1, n2})
    NONE,           \* Sentinel value for "no holder"
    BUGGY           \* TRUE: skip version check in CommitClaim (violates SingleActivation)

\* Type constraints on constants
ASSUME BUGGY \in BOOLEAN

(***************************************************************************)
(* VARIABLES                                                                *)
(*                                                                          *)
(* fdb_holder: The current holder stored in FDB (ground truth)             *)
(* fdb_version: Monotonic version counter for OCC (bounded for checking)   *)
(* node_state: Per-node state machine (Idle, Reading, Committing, Active)  *)
(* node_read_version: Version seen by node during read phase               *)
(***************************************************************************)

VARIABLES
    fdb_holder,         \* Current holder in FDB storage
    fdb_version,        \* FDB key version (for OCC)
    node_state,         \* State machine per node
    node_read_version   \* Version each node read

vars == <<fdb_holder, fdb_version, node_state, node_read_version>>

(***************************************************************************)
(* TYPE INVARIANT                                                           *)
(***************************************************************************)

NodeState == {"Idle", "Reading", "Committing", "Active"}

TypeOK ==
    /\ fdb_holder \in Nodes \cup {NONE}
    /\ fdb_version \in Nat
    /\ node_state \in [Nodes -> NodeState]
    /\ node_read_version \in [Nodes -> Nat]

\* Helper: nodes currently in a claiming state (Reading or Committing)
Claiming(n) == node_state[n] \in {"Reading", "Committing"}

(***************************************************************************)
(* INITIAL STATE                                                            *)
(***************************************************************************)

Init ==
    /\ fdb_holder = NONE
    /\ fdb_version = 0
    /\ node_state = [n \in Nodes |-> "Idle"]
    /\ node_read_version = [n \in Nodes |-> 0]

(***************************************************************************)
(* ACTIONS                                                                  *)
(*                                                                          *)
(* FDB Transaction Model (Simplified OCC):                                  *)
(* 1. StartClaim: Node initiates claim, enters Reading state               *)
(* 2. ReadFDB: Node reads current holder and version (snapshot read)       *)
(* 3. CommitClaim: Node attempts atomic commit                             *)
(*    - Success: version unchanged AND no holder -> become holder          *)
(*    - Failure: version changed OR has holder -> return to Idle           *)
(* 4. Release: Active node releases the actor                              *)
(***************************************************************************)

\* A node initiates a claim for the actor
\* Precondition: node is Idle and not already the holder
StartClaim(n) ==
    /\ node_state[n] = "Idle"
    /\ fdb_holder # n                           \* Don't claim if already holder
    /\ node_state' = [node_state EXCEPT ![n] = "Reading"]
    /\ UNCHANGED <<fdb_holder, fdb_version, node_read_version>>

\* Node reads current state from FDB (snapshot read)
\* This captures the version at read time for later conflict detection
ReadFDB(n) ==
    /\ node_state[n] = "Reading"
    /\ node_read_version' = [node_read_version EXCEPT ![n] = fdb_version]
    /\ node_state' = [node_state EXCEPT ![n] = "Committing"]
    /\ UNCHANGED <<fdb_holder, fdb_version>>

\* Node attempts to commit claim transaction
\* FDB commit semantics: succeeds only if key unchanged since read
\* BUGGY mode: skips version check AND uses stale holder value from read time,
\* allowing split-brain (multiple active nodes)
CommitClaim(n) ==
    /\ node_state[n] = "Committing"
    /\ LET read_ver == node_read_version[n]
           current_ver == fdb_version
           current_holder == fdb_holder
       IN
       IF BUGGY THEN
           \* BUGGY: Uses stale information from read phase, doesn't re-check
           \* at commit time. The bug is that we check against read_ver=0 only,
           \* not against the actual current state. This models a TOCTOU race.
           \* If holder was NONE when we read (read_ver=0 implies no writes yet),
           \* we blindly commit even if someone else committed since then.
           IF read_ver = 0 THEN
               \* We read when there was no holder - commit blindly!
               \* This overwrites any holder that was set between our read and commit
               /\ fdb_holder' = n
               /\ fdb_version' = fdb_version + 1
               /\ node_state' = [node_state EXCEPT ![n] = "Active"]
           ELSE
               \* We read when there was already a holder (version > 0), fail
               /\ node_state' = [node_state EXCEPT ![n] = "Idle"]
               /\ UNCHANGED <<fdb_holder, fdb_version>>
       ELSE
           \* SAFE: Full OCC with version checking
           \/ \* SUCCESS: No conflict (version same) and no current holder
              /\ read_ver = current_ver
              /\ current_holder = NONE
              /\ fdb_holder' = n
              /\ fdb_version' = fdb_version + 1      \* Version bumps on write
              /\ node_state' = [node_state EXCEPT ![n] = "Active"]
           \/ \* FAILURE: Version conflict (someone wrote since our read)
              /\ read_ver # current_ver
              /\ node_state' = [node_state EXCEPT ![n] = "Idle"]
              /\ UNCHANGED <<fdb_holder, fdb_version>>
           \/ \* FAILURE: Already has a holder
              /\ read_ver = current_ver
              /\ current_holder # NONE
              /\ node_state' = [node_state EXCEPT ![n] = "Idle"]
              /\ UNCHANGED <<fdb_holder, fdb_version>>
    /\ UNCHANGED <<node_read_version>>

\* Active node releases the actor
Release(n) ==
    /\ node_state[n] = "Active"
    /\ fdb_holder = n
    /\ fdb_holder' = NONE
    /\ fdb_version' = fdb_version + 1           \* Version bumps on write
    /\ node_state' = [node_state EXCEPT ![n] = "Idle"]
    /\ UNCHANGED <<node_read_version>>

(***************************************************************************)
(* NEXT STATE RELATION                                                      *)
(***************************************************************************)

Next ==
    \E n \in Nodes:
        \/ StartClaim(n)
        \/ ReadFDB(n)
        \/ CommitClaim(n)
        \/ Release(n)

(***************************************************************************)
(* FAIRNESS - Required for Liveness                                         *)
(*                                                                          *)
(* Weak Fairness (WF): If an action is continuously enabled, it eventually *)
(* executes. This models FDB's guarantee that transactions eventually      *)
(* complete (no permanent starvation under normal operation).              *)
(*                                                                          *)
(* We require weak fairness on ReadFDB and CommitClaim to ensure that      *)
(* once a node starts claiming, it eventually completes (success or fail). *)
(***************************************************************************)

Fairness ==
    /\ \A n \in Nodes: WF_vars(ReadFDB(n))
    /\ \A n \in Nodes: WF_vars(CommitClaim(n))

(***************************************************************************)
(* SAFETY PROPERTIES                                                        *)
(***************************************************************************)

\* CRITICAL SAFETY: At most one node is active at any time
\* This is THE key guarantee of single activation
SingleActivation ==
    Cardinality({n \in Nodes : node_state[n] = "Active"}) <= 1

\* The FDB holder is consistent with node states
\* If a node thinks it's active, FDB agrees
ConsistentHolder ==
    \A n \in Nodes:
        node_state[n] = "Active" => fdb_holder = n

\* Combined safety invariant
SafetyInvariant == SingleActivation /\ ConsistentHolder /\ TypeOK

(***************************************************************************)
(* LIVENESS PROPERTIES                                                      *)
(*                                                                          *)
(* EventualActivation: If a node starts claiming, it eventually resolves   *)
(* (either becomes Active or returns to Idle).                             *)
(*                                                                          *)
(* Temporal operators:                                                      *)
(* - [] (always/globally): property holds in all future states             *)
(* - <> (eventually): property holds in some future state                  *)
(* - ~> (leads-to): P ~> Q means [](P => <>Q)                              *)
(*                                                                          *)
(* The liveness property ensures no claim hangs forever.                   *)
(***************************************************************************)

\* Every claim attempt eventually resolves
\* If a node is claiming (Reading or Committing), it eventually becomes
\* either Active (success) or Idle (failure)
EventualActivation ==
    \A n \in Nodes:
        Claiming(n) ~> (node_state[n] = "Active" \/ node_state[n] = "Idle")

\* Alternative: No node stays in claiming state forever
NoStuckClaims ==
    \A n \in Nodes:
        [](Claiming(n) => <>(~Claiming(n)))

(***************************************************************************)
(* SPECIFICATION                                                            *)
(***************************************************************************)

\* Full specification with fairness for liveness checking
Spec == Init /\ [][Next]_vars /\ Fairness

\* Safety-only specification (no fairness, no liveness)
SafetySpec == Init /\ [][Next]_vars

(***************************************************************************)
(* STATE CONSTRAINT (for bounded model checking)                           *)
(* Limits version growth to keep state space finite                        *)
(***************************************************************************)

\* Use this as a state constraint in TLC to bound the version
\* This doesn't affect correctness - just makes checking tractable
StateConstraint == fdb_version <= 10

(***************************************************************************)
(* THEOREMS (for documentation)                                            *)
(***************************************************************************)

\* Safety theorem: SingleActivation holds in all reachable states
THEOREM Spec => []SingleActivation

\* Liveness theorem: Every claim eventually resolves
THEOREM Spec => EventualActivation

=============================================================================
