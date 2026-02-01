# TLA+ to DST Test Mapping

This document maps TLA+ specifications and invariants to their corresponding Rust implementations and DST tests.

## Overview

Kelpie uses a verification pyramid:
1. **TLA+ Specs** - Prove algorithm correctness (exhaustive within bounds)
2. **DST Tests** - Test implementation correctness (sampled executions)
3. **Runtime Invariants** - Bridge specs to tests by checking the same properties

## Invariant Implementations

All invariants are implemented in `crates/kelpie-dst/src/invariants.rs`.

### Safety Invariants

| TLA+ Spec | Invariant | Rust Implementation | Status |
|-----------|-----------|---------------------|--------|
| KelpieSingleActivation.tla | SingleActivation | `SingleActivation` | ✅ Implemented |
| KelpieSingleActivation.tla | ConsistentHolder | `ConsistentHolder` | ✅ Implemented |
| KelpieRegistry.tla | PlacementConsistency | `PlacementConsistency` | ✅ Implemented |
| KelpieLease.tla | LeaseUniqueness | `LeaseUniqueness` | ✅ Implemented |
| KelpieLease.tla | FencingTokenMonotonic | `FencingTokenMonotonic` | ✅ Implemented |
| KelpieWAL.tla | Durability | `Durability` | ✅ Implemented |
| KelpieWAL.tla | AtomicVisibility | `AtomicVisibility` | ✅ Implemented |
| KelpieClusterMembership.tla | NoSplitBrain | `NoSplitBrain` | ✅ Implemented |
| KelpieFDBTransaction.tla | ReadYourWrites | `ReadYourWrites` | ✅ Implemented |
| KelpieTeleport.tla | SnapshotConsistency | `SnapshotConsistency` | ✅ Implemented |

### Not Yet Implemented

| TLA+ Spec | Invariant | Priority |
|-----------|-----------|----------|
| KelpieFDBTransaction.tla | SerializableIsolation | Medium |
| KelpieFDBTransaction.tla | ConflictDetection | Medium |
| KelpieActorLifecycle.tla | LifecycleOrdering | Low |
| KelpieActorLifecycle.tla | GracefulDeactivation | Low |
| KelpieMigration.tla | OwnershipConsistency | Medium |

## DST Test Coverage

### single_activation_dst.rs

Tests the SingleActivation and ConsistentHolder invariants.

```rust
// VERIFIES: KelpieSingleActivation.tla::SingleActivation
// VERIFIES: KelpieSingleActivation.tla::ConsistentHolder
#[test]
fn test_concurrent_activation_single_winner() {
    // ... test code ...

    // Invariant check
    let checker = InvariantChecker::new()
        .with_invariant(SingleActivation)
        .with_invariant(ConsistentHolder);
    checker.verify_all(&state).expect("Invariants must hold");
}
```

### lease_dst.rs

Tests the LeaseUniqueness and FencingTokenMonotonic invariants.

```rust
// VERIFIES: KelpieLease.tla::LeaseUniqueness
// VERIFIES: KelpieLease.tla::FencingTokenMonotonic
#[test]
fn test_dst_lease_uniqueness_invariant() {
    // ... test code ...
}
```

### cluster_membership_dst.rs

Tests the NoSplitBrain invariant.

```rust
// VERIFIES: KelpieClusterMembership.tla::NoSplitBrain
#[test]
fn test_membership_no_split_brain() {
    // ... test code ...
}
```

### partition_tolerance_dst.rs

Tests invariants under network partitions.

```rust
// VERIFIES: KelpieSingleActivation.tla::SingleActivation (under partition)
#[test]
fn test_partition_healing_no_split_brain() {
    // ... test code ...
}
```

## Using Invariants in Tests

### Basic Usage

```rust
use kelpie_dst::{
    InvariantChecker, SingleActivation, ConsistentHolder, SystemState, NodeInfo, NodeState
};

#[test]
fn test_with_invariants() {
    // Build system state
    let state = SystemState::new()
        .with_node(NodeInfo::new("node-1").with_actor_state("actor-1", NodeState::Active))
        .with_fdb_holder("actor-1", Some("node-1".to_string()));

    // Verify invariants
    let checker = InvariantChecker::new()
        .with_invariant(SingleActivation)
        .with_invariant(ConsistentHolder);

    checker.verify_all(&state).expect("Invariants violated!");
}
```

### Using InvariantCheckingSimulation

```rust
use kelpie_dst::InvariantCheckingSimulation;

#[test]
fn test_with_auto_checking() {
    let sim = InvariantCheckingSimulation::new()
        .with_standard_invariants()
        .with_cluster_invariants();

    // Run test, checking invariants at each step
    // ...
}
```

### Preset Invariant Groups

```rust
// Standard invariants (6)
sim.with_standard_invariants()

// Cluster membership
sim.with_cluster_invariants()  // NoSplitBrain

// Linearizability
sim.with_linearizability_invariants()  // ReadYourWrites

// Lease safety
sim.with_lease_invariants()  // LeaseUniqueness, FencingTokenMonotonic
```

## TLA+ Definitions

### SingleActivation (KelpieSingleActivation.tla)

```tla
SingleActivation ==
    Cardinality({n \in Nodes : node_state[n] = "Active"}) <= 1
```

At most one node can be Active for any given actor.

### NoSplitBrain (KelpieClusterMembership.tla)

```tla
NoSplitBrain ==
    \A n1, n2 \in Nodes :
        /\ HasValidPrimaryClaim(n1)
        /\ HasValidPrimaryClaim(n2)
        => n1 = n2
```

At most one valid primary (with quorum) exists.

### ReadYourWrites (KelpieFDBTransaction.tla)

```tla
ReadYourWrites ==
    \A t \in Transactions :
        txnState[t] = RUNNING =>
            \A k \in Keys :
                writeBuffer[t][k] # NoValue =>
                    TxnRead(t, k) = writeBuffer[t][k]
```

A running transaction sees its own writes.

### FencingTokenMonotonic (KelpieLease.tla)

```tla
FencingTokenMonotonic ==
    \A a \in Actors:
        fencingTokens[a] >= 0
```

Fencing tokens are non-negative and only increase.

## Verification Commands

```bash
# Run all invariant tests
cargo test -p kelpie-dst --lib invariants

# Run with specific TLA+ spec coverage
cargo test -p kelpie-dst single_activation  # SingleActivation
cargo test -p kelpie-dst lease              # LeaseUniqueness
cargo test -p kelpie-dst cluster_membership # NoSplitBrain

# Verify TLA+ specs with TLC
cd docs/tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock KelpieSingleActivation.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease_SafetyOnly.cfg KelpieLease.tla
```

## Adding New Invariants

1. Define the invariant in TLA+ spec (if not already)
2. Add Rust implementation in `invariants.rs`:
   ```rust
   pub struct MyInvariant;

   impl Invariant for MyInvariant {
       fn name(&self) -> &'static str { "MyInvariant" }
       fn tla_source(&self) -> &'static str { "docs/tla/MySpec.tla" }
       fn check(&self, state: &SystemState) -> Result<(), InvariantViolation> {
           // Implementation
       }
   }
   ```
3. Add tests for the invariant
4. Export from `lib.rs`
5. Update this mapping document
