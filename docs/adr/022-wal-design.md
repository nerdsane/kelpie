# ADR-022: WAL Design

## Status

Accepted

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| WAL entry types | 📋 Designed | TLA+ spec |
| Entry lifecycle | 📋 Designed | TLA+ spec |
| Recovery protocol | 📋 Designed | TLA+ spec |
| Cleanup/GC | 📋 Designed | TLA+ spec |

## Context

Kelpie needs a mechanism to ensure operation durability and atomicity for agent operations. When an agent performs operations (create, update, delete, send message), these operations must be:

1. **Durable**: Survive crashes and restarts
2. **Atomic**: Either fully complete or have no effect
3. **Idempotent**: Safe to replay without side effects
4. **Recoverable**: Pending operations resume after crash

Direct writes to FoundationDB don't provide these guarantees for multi-step operations. A Write-Ahead Log (WAL) provides a proven pattern for durability and atomicity.

## Decision

Implement a Write-Ahead Log (WAL) with the following design:

### Entry Lifecycle

```
┌─────────────────────────────────────────────────────────────┐
│                    WAL Entry Lifecycle                       │
│                                                              │
│    ┌─────────┐     ┌───────────┐     ┌───────────┐          │
│    │ Pending │────▶│ Completed │     │  Failed   │          │
│    └─────────┘     └───────────┘     └───────────┘          │
│         │                                   ▲                │
│         └──────────(on error)───────────────┘                │
│                                                              │
│    On crash recovery: replay all Pending entries             │
└─────────────────────────────────────────────────────────────┘
```

### WAL Entry Structure

```rust
struct WalEntry {
    id: u64,                    // Unique entry ID
    client: ClientId,           // Client that initiated operation
    operation: Operation,       // Create, Update, Delete, SendMessage
    idempotency_key: u64,       // For duplicate detection
    status: Status,             // Pending, Completed, Failed
    data: Bytes,                // Operation payload
}
```

### Key Design Points

1. **Append-Only Write**: Operations are logged to WAL before execution
2. **Status Transitions**: Pending → Completed (success) or Pending → Failed (error)
3. **Idempotency**: Each client+key combination produces at most one entry
4. **Recovery Replay**: On startup, all Pending entries are replayed to completion
5. **Cleanup**: Completed/Failed entries are garbage collected after retention period

### Protocol

1. **Append**: Client submits operation with idempotency key
   - If key exists: No-op (idempotent)
   - If key new: Create Pending entry

2. **Execute**: Process the operation against storage
   - On success: Mark entry Completed, apply to storage
   - On failure: Mark entry Failed

3. **Recovery**: On system startup
   - Scan for Pending entries
   - Replay each to completion
   - Resume normal operation

4. **Cleanup**: Periodically remove old Completed/Failed entries

## Formal Specification

**TLA+ Model**: [KelpieWAL.tla](../tla/KelpieWAL.tla)

### Safety Invariants

| Invariant | Description |
|-----------|-------------|
| `Durability` | Completed entries remain completed; storage reflects the data |
| `Idempotency` | No duplicate entries for same client+idempotency key |
| `AtomicVisibility` | Completed entries are fully applied to storage |
| `TypeOK` | All variables have correct types and bounds |

### Liveness Properties

| Property | Description |
|----------|-------------|
| `EventualRecovery` | After crash, system eventually recovers and processes all pending entries |
| `EventualCompletion` | Pending entries eventually become non-pending (completed or failed) |
| `NoStarvation` | Every client's pending operations eventually complete |
| `ProgressUnderCrash` | Crashes don't permanently block the system |

### Model Checking Results

- **Safe config**: PASS (70,713 states single client / 2,548,321 states concurrent)
- **Buggy config**: FAIL - `Idempotency` invariant violated when BUGGY=TRUE skips idempotency check

### DST Alignment

| Failure Mode | TLA+ | DST | Notes |
|--------------|------|-----|-------|
| CrashBeforeWrite | ✅ Crash action | ✅ | Entry stays Pending |
| CrashAfterWrite | ✅ Crash action | ✅ | Recovery replays |
| ConcurrentClients | ✅ Multiple clients | ✅ | Idempotency prevents duplicates |

## Consequences

### Positive

- **Guaranteed Durability**: Operations survive crashes
- **Atomic Operations**: All-or-nothing semantics
- **Idempotent Replay**: Safe crash recovery
- **Audit Trail**: WAL provides operation history

### Negative

- **Write Amplification**: Two writes per operation (WAL + storage)
- **Storage Overhead**: WAL entries consume space until cleanup
- **Latency**: Extra round-trip for WAL append

### Neutral

- WAL is a well-understood pattern with proven reliability
- Requires periodic cleanup to manage storage

## Alternatives Considered

### Direct FDB Writes

- Write directly to FoundationDB without WAL
- Rely on FDB transactions for atomicity

**Rejected because**: FDB transactions are limited to 5 seconds; complex multi-step operations may timeout. WAL allows resumable operations.

### Event Sourcing

- Store events as source of truth
- Rebuild state from event replay

**Rejected because**: Higher complexity for the same durability guarantees. WAL is simpler for our use case.

### Saga Pattern

- Compensating transactions for multi-step operations
- Rollback on failure

**Rejected because**: Compensating transactions are complex to implement correctly. WAL with replay is simpler.

## References

- [KelpieWAL.tla](../tla/KelpieWAL.tla) - TLA+ specification
- [ADR-008: Transaction API](./008-transaction-api.md) - Transaction semantics
- [Write-Ahead Logging (Wikipedia)](https://en.wikipedia.org/wiki/Write-ahead_logging)
- [FoundationDB Transactions](https://apple.github.io/foundationdb/transaction-basics.html)
