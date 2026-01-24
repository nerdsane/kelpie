---------------------------- MODULE KelpieFDBTransaction ----------------------------
(*
 * TLA+ Specification for FoundationDB Transaction Semantics in Kelpie
 *
 * This spec models the transaction guarantees that Kelpie relies on from FDB:
 * - Serializable isolation (concurrent transactions appear to execute serially)
 * - Conflict detection (conflicting read-write or write-write detected)
 * - Atomic commit (all-or-nothing)
 * - Read-your-writes (transaction sees its own uncommitted writes)
 *
 * References:
 * - ADR-002: FoundationDB Integration (G2.4 - conflict detection)
 * - ADR-004: Linearizability Guarantees (G4.1 - atomic operations)
 * - FDB Paper: https://www.foundationdb.org/files/fdb-paper.pdf
 *
 * TigerStyle: Explicit state, 2+ assertions per operation, bounded model.
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

\* Configuration constants
CONSTANTS
    Transactions,   \* Set of transaction IDs (e.g., {Txn1, Txn2})
    Keys,          \* Set of keys (e.g., {k1, k2})
    Values,        \* Set of possible values (e.g., {v0, v1, v2})
    InitialValue,  \* Initial value for all keys
    EnableConflictDetection  \* TRUE for correct model, FALSE for buggy

\* Transaction states
CONSTANTS IDLE, RUNNING, COMMITTED, ABORTED

\* NoValue constant for unset entries (must be defined before use)
CONSTANT NoValue

\* State variables
VARIABLES
    kvStore,       \* Global key-value store: Keys -> Values (committed state)
    txnState,      \* Transaction state: Transactions -> {IDLE, RUNNING, COMMITTED, ABORTED}
    readSet,       \* Read set per transaction: Transactions -> SUBSET Keys
    writeBuffer,   \* Buffered writes: Transactions -> [Keys -> Values]
    readSnapshot,  \* Snapshot of kvStore at transaction start: Transactions -> [Keys -> Values]
    commitOrder    \* Sequence of committed transactions (for conflict detection)

vars == <<kvStore, txnState, readSet, writeBuffer, readSnapshot, commitOrder>>

--------------------------------------------------------------------------------
(* Type invariants *)

TypeOK ==
    /\ kvStore \in [Keys -> Values]
    /\ txnState \in [Transactions -> {IDLE, RUNNING, COMMITTED, ABORTED}]
    /\ readSet \in [Transactions -> SUBSET Keys]
    /\ writeBuffer \in [Transactions -> [Keys -> Values \cup {NoValue}]]
    /\ readSnapshot \in [Transactions -> [Keys -> Values \cup {NoValue}]]
    /\ commitOrder \in Seq(Transactions)

--------------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ kvStore = [k \in Keys |-> InitialValue]
    /\ txnState = [t \in Transactions |-> IDLE]
    /\ readSet = [t \in Transactions |-> {}]
    /\ writeBuffer = [t \in Transactions |-> [k \in Keys |-> NoValue]]
    /\ readSnapshot = [t \in Transactions |-> [k \in Keys |-> NoValue]]
    /\ commitOrder = <<>>

--------------------------------------------------------------------------------
(* Helper functions *)

\* Get the value a transaction would see for a key
\* (read-your-writes: check write buffer first, then snapshot)
TxnRead(t, k) ==
    IF writeBuffer[t][k] # NoValue
    THEN writeBuffer[t][k]
    ELSE readSnapshot[t][k]

\* Check if there's a conflict between transaction t and committed transactions
\* A conflict occurs if any key in t's read set was modified by a transaction
\* that committed after t started (i.e., not in t's snapshot)
HasConflict(t) ==
    IF ~EnableConflictDetection
    THEN FALSE  \* Buggy: skip conflict detection
    ELSE
        \* Check if any committed transaction modified keys we read
        \E k \in readSet[t] : kvStore[k] # readSnapshot[t][k]

\* Get all keys written by a transaction
WrittenKeys(t) ==
    {k \in Keys : writeBuffer[t][k] # NoValue}

--------------------------------------------------------------------------------
(* Transaction operations *)

\* Begin a transaction
Begin(t) ==
    /\ txnState[t] = IDLE
    /\ txnState' = [txnState EXCEPT ![t] = RUNNING]
    /\ readSet' = [readSet EXCEPT ![t] = {}]
    /\ writeBuffer' = [writeBuffer EXCEPT ![t] = [k \in Keys |-> NoValue]]
    /\ readSnapshot' = [readSnapshot EXCEPT ![t] = kvStore]  \* Take snapshot
    /\ UNCHANGED <<kvStore, commitOrder>>

\* Read a key within a transaction
Read(t, k) ==
    /\ txnState[t] = RUNNING
    /\ readSet' = [readSet EXCEPT ![t] = @ \cup {k}]
    /\ UNCHANGED <<kvStore, txnState, writeBuffer, readSnapshot, commitOrder>>

\* Write a key within a transaction (buffered, not yet committed)
Write(t, k, v) ==
    /\ txnState[t] = RUNNING
    /\ v \in Values
    /\ writeBuffer' = [writeBuffer EXCEPT ![t][k] = v]
    /\ UNCHANGED <<kvStore, txnState, readSet, readSnapshot, commitOrder>>

\* Commit a transaction (atomic)
\* If there's a conflict, abort instead
Commit(t) ==
    /\ txnState[t] = RUNNING
    /\ IF HasConflict(t)
       THEN \* Conflict detected - abort
            /\ txnState' = [txnState EXCEPT ![t] = ABORTED]
            /\ UNCHANGED <<kvStore, commitOrder>>
       ELSE \* No conflict - commit atomically
            /\ txnState' = [txnState EXCEPT ![t] = COMMITTED]
            /\ kvStore' = [k \in Keys |->
                           IF writeBuffer[t][k] # NoValue
                           THEN writeBuffer[t][k]
                           ELSE kvStore[k]]
            /\ commitOrder' = Append(commitOrder, t)
    /\ UNCHANGED <<readSet, writeBuffer, readSnapshot>>

\* Explicitly abort a transaction
Abort(t) ==
    /\ txnState[t] = RUNNING
    /\ txnState' = [txnState EXCEPT ![t] = ABORTED]
    /\ UNCHANGED <<kvStore, readSet, writeBuffer, readSnapshot, commitOrder>>

--------------------------------------------------------------------------------
(* Next state relation *)

Next ==
    \/ \E t \in Transactions : Begin(t)
    \/ \E t \in Transactions, k \in Keys : Read(t, k)
    \/ \E t \in Transactions, k \in Keys, v \in Values : Write(t, k, v)
    \/ \E t \in Transactions : Commit(t)
    \/ \E t \in Transactions : Abort(t)

\* Fairness: every running transaction eventually commits or aborts
Fairness ==
    /\ \A t \in Transactions : WF_vars(Commit(t))
    /\ \A t \in Transactions : WF_vars(Abort(t))

Spec == Init /\ [][Next]_vars /\ Fairness

--------------------------------------------------------------------------------
(* Safety invariants *)

\* Invariant 1: Serializable Isolation
\* All committed transactions can be arranged in a serial order such that
\* each transaction sees only the effects of transactions that precede it.
\*
\* We verify this by checking that the commit order respects read dependencies:
\* If transaction A reads a value written by B, then B must appear before A
\* in the commit order.
SerializableIsolation ==
    \A i, j \in 1..Len(commitOrder) :
        i < j =>
            LET tA == commitOrder[i]
                tB == commitOrder[j]
            IN \* tA committed before tB
               \* If tB read any key that tA wrote, that's a conflict that
               \* should have been detected (tB should have seen tA's writes
               \* or been aborted)
               \A k \in readSet[tB] :
                   writeBuffer[tA][k] # NoValue =>
                       \* Either tB saw tA's write (in its snapshot) or
                       \* tB wrote to k itself (overwriting tA)
                       \/ readSnapshot[tB][k] = writeBuffer[tA][k]
                       \/ writeBuffer[tB][k] # NoValue

\* Invariant 2: Conflict Detection
\* In FDB, conflicts are detected when:
\* - Transaction T1 reads key K from its snapshot
\* - Transaction T2 writes key K with a DIFFERENT value
\* - T2 commits BEFORE T1 tries to commit
\* - At T1's commit time, kvStore[K] != T1's snapshot[K] → T1 aborts
\*
\* The invariant: if both T1 (reader) and T2 (writer) committed, and T2
\* committed first with a different value, then either:
\* 1. T1's snapshot already reflected T2's write (different snapshots), OR
\* 2. T1 also wrote to K (overwriting T2's value), OR
\* 3. T1 committed before T2 (no conflict at T1's commit time), OR
\* 4. T2 wrote the same value as the snapshot (no visible change)
ConflictDetection ==
    \A tWriter, tReader \in Transactions :
        (tWriter # tReader /\
         txnState[tWriter] = COMMITTED /\ txnState[tReader] = COMMITTED) =>
            \A k \in Keys :
                \* If tWriter wrote k and tReader read k...
                (writeBuffer[tWriter][k] # NoValue /\ k \in readSet[tReader]) =>
                    \* ...either they had different snapshots, OR tReader also wrote k,
                    \* OR tReader committed first, OR tWriter wrote the same value
                    \/ readSnapshot[tWriter] # readSnapshot[tReader]
                    \/ writeBuffer[tReader][k] # NoValue
                    \/ writeBuffer[tWriter][k] = readSnapshot[tReader][k]  \* No change
                    \/ \E i, j \in 1..Len(commitOrder) :
                        /\ commitOrder[i] = tReader
                        /\ commitOrder[j] = tWriter
                        /\ i < j  \* tReader committed before tWriter

\* Invariant 3: Atomic Commit
\* A transaction's writes are either all visible or none are visible.
AtomicCommit ==
    \A t \in Transactions :
        txnState[t] = COMMITTED =>
            \* All writes from t are in kvStore
            \A k \in Keys :
                writeBuffer[t][k] # NoValue =>
                    \* Either our write is there, or someone wrote after us
                    \/ kvStore[k] = writeBuffer[t][k]
                    \/ \E t2 \in Transactions :
                        /\ t2 # t
                        /\ txnState[t2] = COMMITTED
                        /\ writeBuffer[t2][k] # NoValue

\* Invariant 4: Read Your Writes
\* A transaction always sees its own uncommitted writes.
\* (This is enforced by TxnRead, verified by construction)
ReadYourWrites ==
    \A t \in Transactions :
        txnState[t] = RUNNING =>
            \A k \in Keys :
                writeBuffer[t][k] # NoValue =>
                    TxnRead(t, k) = writeBuffer[t][k]

\* Combined safety invariant
Safety ==
    /\ TypeOK
    /\ SerializableIsolation
    /\ ConflictDetection
    /\ AtomicCommit
    /\ ReadYourWrites

--------------------------------------------------------------------------------
(* Liveness properties *)

\* Every started transaction eventually commits or aborts
EventualTermination ==
    \A t \in Transactions :
        txnState[t] = RUNNING ~> (txnState[t] = COMMITTED \/ txnState[t] = ABORTED)

\* A transaction with no conflicts eventually commits
\* (non-conflicting transactions make progress)
EventualCommit ==
    \A t \in Transactions :
        (txnState[t] = RUNNING /\ ~HasConflict(t)) ~> txnState[t] = COMMITTED

--------------------------------------------------------------------------------
(* State constraints for bounded model checking *)

\* Limit the number of committed transactions to keep state space tractable
StateConstraint ==
    Len(commitOrder) <= Cardinality(Transactions)

================================================================================
