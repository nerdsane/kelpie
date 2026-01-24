# Kelpie TLA+ Specifications

This directory contains TLA+ specifications for verifying Kelpie's distributed system properties.

## Files

| File | Description |
|------|-------------|
| `KelpieActorState.tla` | Actor state and transaction lifecycle specification |
| `KelpieActorState.cfg` | Safe mode configuration (correct implementation) |
| `KelpieActorState_Buggy.cfg` | Buggy mode configuration (intentionally incorrect) |

## KelpieActorState Specification

Models the actor state management and transaction semantics from ADR-008.

### Properties Verified

| Property | Description |
|----------|-------------|
| `TypeOK` | Type invariant for all variables |
| `RollbackCorrectness` | After rollback, memory equals pre-invocation snapshot |
| `BufferEmptyWhenIdle` | Transaction buffer empty when not running |
| `SafetyInvariant` | Combined safety properties |
| `EventualCommitOrRollback` | Liveness: every invocation eventually completes |

### RollbackCorrectness Invariant

This is the key invariant (G8.2 from ADR-008):

```tla
RollbackCorrectness ==
    invocationState = "Aborted" =>
        /\ memory = stateSnapshot
        /\ buffer = <<>>
```

After a rollback (abort), the actor's memory state must equal the snapshot taken when the invocation started, and the transaction buffer must be empty. This ensures no partial state changes are visible after rollback.

### Safe vs Buggy Mode

The specification uses a `SafeMode` constant to toggle between correct and buggy implementations:

**Safe Mode (`SafeMode = TRUE`):**
- Writes are buffered until commit (transaction isolation)
- On rollback, memory is restored to pre-invocation snapshot

**Buggy Mode (`SafeMode = FALSE`):**
- Writes are applied directly to memory (isolation violation)
- On rollback, memory is NOT restored (rollback bug)

The buggy mode demonstrates that our invariants catch real bugs.

## Running TLC Model Checker

### Prerequisites

1. Java 11+ installed
2. TLA+ tools (tla2tools.jar) - download from [TLA+ releases](https://github.com/tlaplus/tlaplus/releases)

### Commands

```bash
# Run Safe variant (should pass all invariants)
java -XX:+UseParallelGC -jar tla2tools.jar -deadlock \
  -config KelpieActorState.cfg KelpieActorState.tla

# Run Buggy variant (should FAIL RollbackCorrectness)
java -XX:+UseParallelGC -jar tla2tools.jar -deadlock \
  -config KelpieActorState_Buggy.cfg KelpieActorState.tla

# Run with liveness checking (requires longer run)
java -XX:+UseParallelGC -jar tla2tools.jar -deadlock -liveness \
  -config KelpieActorState.cfg KelpieActorState.tla
```

### Verification Results

| Configuration | Result | States | Time |
|---------------|--------|--------|------|
| Safe (`KelpieActorState.cfg`) | PASS | 136 generated, 60 distinct | <1s |
| Buggy (`KelpieActorState_Buggy.cfg`) | FAIL (RollbackCorrectness) | 12 generated | <1s |

### Buggy Mode Counterexample

TLC finds this counterexample in buggy mode:

```
State 1: Initial
  memory = "empty", snapshot = "empty", state = "Idle"

State 2: StartInvocation
  snapshot = "empty" (captured), state = "Running"

State 3: BufferWrite("v1") [BUGGY: applies directly to memory]
  memory = "v1", snapshot = "empty"

State 4: Rollback [BUGGY: doesn't restore memory]
  memory = "v1" (SHOULD BE "empty"), snapshot = "empty"

  => RollbackCorrectness VIOLATED: memory ≠ stateSnapshot
```

This proves the invariant correctly catches rollback bugs.

## Constants

| Constant | Description | Default |
|----------|-------------|---------|
| `Values` | Set of possible values | `{"v1", "v2", "empty"}` |
| `SafeMode` | TRUE for correct, FALSE for buggy | varies by config |
| `MaxBufferLen` | Max buffer size (bounds state space) | `2` |

## Design Notes

### Why Model Checking?

TLA+ model checking provides:
- Exhaustive exploration of state space
- Automatic counterexample generation
- Formal verification of invariants
- Reproducible verification

### Relation to ADR-008

This specification verifies the transaction semantics defined in ADR-008:
- Atomicity: All writes succeed (commit) or none (abort)
- Isolation: Buffered writes not visible until commit
- Rollback: Memory restored to pre-invocation state on abort

### State Space Bounding

The `MaxBufferLen` constant bounds the buffer size to keep the state space finite. With:
- 3 values (`v1`, `v2`, `empty`)
- 4 invocation states
- Max buffer length of 2

TLC explores 60 distinct states in <1 second.

## See Also

- [ADR-008: Transaction API](../adr/008-transaction-api.md)
- [kelpie-storage/src/transaction.rs](../../crates/kelpie-storage/src/transaction.rs)
- [TLA+ documentation](https://lamport.azurewebsites.net/tla/tla.html)
