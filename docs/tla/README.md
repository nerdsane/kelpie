# Kelpie TLA+ Specifications

This directory contains TLA+ specifications for formally verifying Kelpie's distributed system properties.

## Specifications

### KelpieFDBTransaction.tla

Models FoundationDB transaction semantics that Kelpie relies on for correctness:

- **Serializable Isolation**: Concurrent transactions appear to execute in a serial order
- **Conflict Detection**: Read-write conflicts are detected and cause transaction abort
- **Atomic Commit**: Transaction writes are all-or-nothing
- **Read-Your-Writes**: Transactions see their own uncommitted writes

#### References

- [ADR-002](../adr/002-foundationdb-integration.md): FoundationDB Integration (G2.4 - conflict detection)
- [ADR-004](../adr/004-linearizability-guarantees.md): Linearizability Guarantees (G4.1 - atomic operations)
- [FDB Paper](https://www.foundationdb.org/files/fdb-paper.pdf): FoundationDB testing paper

## Running the Model Checker

### Prerequisites

1. Java 11+ installed
2. TLA+ tools (tla2tools.jar) - download from [TLA+ Tools](https://github.com/tlaplus/tlaplus/releases)

### Safe Configuration (Correct Implementation)

```bash
cd docs/tla
java -XX:+UseParallelGC -Xmx4g -jar tla2tools.jar -deadlock \
    -config KelpieFDBTransaction.cfg KelpieFDBTransaction.tla
```

Expected output: `Model checking completed. No error has been found.`

**Verification Results (2026-01-24):**
- States generated: 308,867
- Distinct states: 56,193
- Depth: 13
- Time: 14 seconds
- Result: **PASS** (all invariants hold)

### Buggy Configuration (Missing Conflict Detection)

```bash
cd docs/tla
java -XX:+UseParallelGC -Xmx4g -jar tla2tools.jar -deadlock \
    -config KelpieFDBTransaction_Buggy.cfg KelpieFDBTransaction.tla
```

Expected output: `Error: Invariant SerializableIsolation is violated.`

**Verification Results (2026-01-24):**
- States generated: 6,536 (before finding error)
- Distinct states: 2,237
- Depth: 7
- Time: 1 second
- Result: **FAIL** (SerializableIsolation violated)

The buggy configuration demonstrates what happens when conflict detection is disabled:
1. Transaction 1 reads key k1 (sees initial value v0)
2. Transaction 2 writes k1 = v1
3. Transaction 2 commits (k1 becomes v1)
4. Transaction 1 commits **without detecting the conflict**
5. Transaction 1 read a stale value, violating serializable isolation

## Model Design

### Constants

| Constant | Description | Default |
|----------|-------------|---------|
| `Transactions` | Set of transaction IDs | `{Txn1, Txn2}` |
| `Keys` | Set of keys | `{k1, k2}` |
| `Values` | Set of possible values | `{v0, v1, v2}` |
| `InitialValue` | Initial value for all keys | `v0` |
| `EnableConflictDetection` | Toggle for conflict detection | `TRUE`/`FALSE` |

### State Variables

- `kvStore`: Global key-value store (committed values)
- `txnState`: Transaction state (IDLE, RUNNING, COMMITTED, ABORTED)
- `readSet`: Keys read by each transaction
- `writeBuffer`: Buffered writes per transaction
- `readSnapshot`: Snapshot of kvStore at transaction start
- `commitOrder`: Sequence of committed transactions

### Operations

- `Begin(t)`: Start transaction, take snapshot
- `Read(t, k)`: Add key to read set
- `Write(t, k, v)`: Buffer write
- `Commit(t)`: Check conflicts, apply writes atomically
- `Abort(t)`: Discard buffered writes

### Invariants

1. **TypeOK**: Type correctness of all state variables
2. **SerializableIsolation**: Committed transactions respect serial order
3. **ConflictDetection**: Read-write conflicts properly detected
4. **AtomicCommit**: Writes are all-or-nothing
5. **ReadYourWrites**: Transactions see their own uncommitted writes

### Liveness Properties

- **EventualTermination**: Every running transaction eventually commits or aborts

## Extending the Specifications

To add new properties:

1. Add invariants in the `(* Safety invariants *)` section
2. Add liveness properties in the `(* Liveness properties *)` section
3. Include them in the `.cfg` file under `INVARIANT` or `PROPERTY`

To modify the model:

1. Adjust constants in the `.cfg` file for different state space sizes
2. Add new operations in the `(* Transaction operations *)` section
3. Update `Next` to include new operations

## TigerStyle Notes

Following TigerBeetle's TigerStyle principles:

- Explicit state variables with clear types
- Bounded model for tractable verification
- Separate Safe/Buggy configurations for validation
- Invariants document expected properties
