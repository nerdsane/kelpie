# Kelpie TLA+ Specifications

This directory contains TLA+ formal specifications for verifying Kelpie's distributed protocols.

## Overview

TLA+ is a formal specification language for modeling concurrent and distributed systems. We use it to verify safety and liveness properties of Kelpie's core protocols before implementing them in Rust.

## Specifications

### KelpieMigration.tla

Models Kelpie's 3-phase actor migration protocol:

```
PREPARE → TRANSFER → COMPLETE
```

**Reference Implementation:** `crates/kelpie-cluster/src/handler.rs`

#### Safety Invariants

| Invariant | Description |
|-----------|-------------|
| `MigrationAtomicity` | Complete migration transfers full state |
| `NoStateLoss` | Actor state is never lost during migration |
| `SingleActivationDuringMigration` | At most one active instance during migration |
| `MigrationRollback` | Failed migration leaves actor recoverable |
| `TypeInvariant` | Variables have correct types |

#### Liveness Properties

| Property | Description |
|----------|-------------|
| `EventualMigrationCompletion` | Started migrations eventually complete or fail |
| `EventualRecovery` | Actors with pending recovery eventually recover |

#### Crash Fault Model

The specification models node crashes during any migration phase:
- Crashes can happen at any point in the protocol
- Recovery is possible when source or target node comes back
- Maximum crash count is bounded for tractable model checking

## Running TLC Model Checker

### Prerequisites

Download TLA+ tools:
```bash
curl -L -o tla2tools.jar \
  https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar
```

### Safe Configuration

Verifies the correct protocol passes all invariants:

```bash
cd docs/tla
java -XX:+UseParallelGC -jar tla2tools.jar -deadlock \
  -config KelpieMigration.cfg KelpieMigration.tla
```

**Expected Result:** No errors found

```
Model checking completed. No error has been found.
238 states generated, 59 distinct states found, 0 states left on queue.
The depth of the complete state graph search is 11.
```

### Buggy Configuration

Demonstrates the spec catches bugs (skipping state transfer):

```bash
java -XX:+UseParallelGC -jar tla2tools.jar -deadlock \
  -config KelpieMigration_Buggy.cfg KelpieMigration.tla
```

**Expected Result:** `MigrationAtomicity` invariant violation

```
Error: Invariant MigrationAtomicity is violated.
```

The counter-example trace shows:
1. Migration starts (PREPARE)
2. Target is prepared (TRANSFER)
3. **State transfer is skipped** (bug!)
4. Migration completes (COMPLETE)
5. `target_received_state = "none"` but `actor_state = "initial_state"` → violation

## Verification Results

| Configuration | States | Distinct | Result | Time |
|--------------|--------|----------|--------|------|
| Safe | 238 | 59 | ✅ Pass | <1s |
| Buggy | 50 | 18 | ❌ MigrationAtomicity violated | <1s |

## Model Parameters

The specifications use bounded model checking with small state spaces:

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `Nodes` | `{n1, n2}` | Two nodes for migration |
| `Actors` | `{a1}` | One actor to migrate |
| `MaxCrashes` | `2` | Bound crash exploration |

These parameters can be increased for deeper exploration at the cost of longer verification times.

## Architecture Decision References

- **ADR-004:** Linearizability Guarantees
  - G4.5: Failure detection and recovery
  - CP design: Consistency over Availability
  - Lease-based ownership protocol

## Development Workflow

1. **Design changes in TLA+ first** - Modify the specification
2. **Run TLC** - Verify invariants hold
3. **Implement in Rust** - Follow the verified protocol
4. **DST coverage** - Test implementation matches spec

## Adding New Specifications

1. Create `NewProtocol.tla` with the protocol model
2. Create `NewProtocol.cfg` for the safe configuration
3. Create `NewProtocol_Buggy.cfg` to demonstrate spec catches bugs
4. Add documentation to this README
5. Run TLC to verify both configurations

## Resources

- [TLA+ Documentation](https://lamport.azurewebsites.net/tla/tla.html)
- [TLA+ Video Course](https://lamport.azurewebsites.net/video/videos.html)
- [TLC Model Checker](https://github.com/tlaplus/tlaplus)
- [Practical TLA+](https://learntla.com/) - Online book
