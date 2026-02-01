# ADR-001: Virtual Actor Model

## Status

Accepted

## Date

2025-01-10

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| ActorId | ✅ Complete | `kelpie-core/src/lib.rs` |
| Actor trait | ✅ Complete | `kelpie-runtime/src/actor.rs` |
| ActorContext | ✅ Complete | `kelpie-runtime/src/context.rs` |
| Dispatcher | ✅ Complete | `kelpie-runtime/src/dispatcher.rs` |
| Single activation | ✅ Complete | `kelpie-runtime/src/dispatcher.rs` |
| Lifecycle hooks | ✅ Complete | `on_activate`, `on_deactivate` |
| State persistence | ✅ Complete | Via ActorKV trait |

## Context

Kelpie needs a programming model for distributed stateful computation that:
- Provides location transparency for actors
- Guarantees single-threaded execution per actor
- Supports on-demand activation and automatic lifecycle management
- Scales horizontally across a cluster
- Handles failures gracefully with automatic recovery

Traditional actor models (Erlang, Akka) require explicit actor lifecycle management and don't guarantee single-threaded execution in distributed settings.

## Decision

We adopt the **Virtual Actor** model, as pioneered by Microsoft Orleans and implemented in Go by NOLA.

### Key Properties

1. **Virtual Existence**: Actors always exist conceptually. They are activated on-demand when invoked and deactivated when idle.

2. **Single Activation Guarantee**: At most one instance of an actor exists in the cluster at any time. This is enforced by the registry.

3. **Location Transparency**: Callers invoke actors by ID without knowing their physical location. The runtime handles routing.

4. **Single-Threaded Execution**: Each actor processes one message at a time. No locks needed inside actors.

5. **Automatic Lifecycle**: Actors are automatically activated on first invocation and deactivated after idle timeout.

### Actor Identity

```rust
pub struct ActorId {
    namespace: String,  // e.g., "orders", "users"
    id: String,         // e.g., "order-12345", "user-abc"
}
```

### Actor Trait

```rust
#[async_trait]
pub trait Actor: Send + Sync + 'static {
    type State: Serialize + DeserializeOwned + Default + Send + Sync;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes>;

    async fn on_activate(&self, ctx: &mut ActorContext<Self::State>) -> Result<()>;
    async fn on_deactivate(&self, ctx: &mut ActorContext<Self::State>) -> Result<()>;
}
```

## Consequences

### Positive

- **Simplified Programming Model**: Developers think about actors as always existing. No explicit create/destroy.
- **Natural Scaling**: Actors distribute automatically across cluster nodes.
- **Built-in Fault Tolerance**: Failed actors are transparently reactivated on healthy nodes.
- **Single-Threaded Safety**: No race conditions inside actors.
- **Memory Efficiency**: Idle actors are deactivated, freeing memory.

### Negative

- **Registry Overhead**: Single activation guarantee requires distributed coordination.
- **Activation Latency**: First invocation to a deactivated actor incurs activation cost.
- **Limited Concurrency**: Single-threaded actors can become bottlenecks.
- **State Serialization**: Actor state must be serializable for persistence.

### Neutral

- Different from traditional actor models (no explicit spawn/kill)
- Requires understanding virtual actor semantics

## Alternatives Considered

### Traditional Actor Model (Akka/Erlang style)

- Explicit actor lifecycle management
- Multiple instances possible (no single activation)
- More flexible but harder to reason about in distributed settings

**Rejected because**: Single activation guarantee is essential for linearizability.

### Service Mesh with Stateless Services

- Stateless services with external state stores
- Horizontal scaling without coordination

**Rejected because**: Requires explicit state management, loses actor model benefits.

### Event Sourcing without Actors

- Events as the source of truth
- Aggregate roots rebuilt from events

**Rejected because**: Higher complexity, still need something to process events.

## Formal Specification

The single activation guarantee is formalized in [KelpieSingleActivation.tla](../tla/KelpieSingleActivation.tla).

### Safety Invariants

| Invariant | Description | TLA+ Definition |
|-----------|-------------|-----------------|
| SingleActivation | At most one active instance per actor ID in the cluster | `SingleActivation` |
| PlacementConsistency | Registry placement matches actual activation location | `PlacementConsistency` |
| LeaseValidityIfActive | Active actors have valid leases | `LeaseValidityIfActive` |

### Bug Patterns

The TLA+ spec includes buggy configurations that demonstrate how invariants can be violated:

| Bug Pattern | Violation | DST Test |
|-------------|-----------|----------|
| TryClaimActor_Racy | TOCTOU race allows dual activation | `test_single_activation_with_network_partition` |
| LeaseExpires_Racy | Zombie actor reclaim race | `test_single_activation_with_crash_recovery` |

### TLC Verification

- **Safe config**: All invariants hold (714 states, depth 27)
- **Buggy config**: SingleActivation violated with racy claims

### DST Coverage

Tests verifying these invariants are in `crates/kelpie-dst/tests/single_activation_dst.rs`:
- `test_concurrent_activation_single_winner` - Concurrent activations, exactly one wins
- `test_single_activation_with_network_partition` - Invariant holds under network partition
- `test_single_activation_with_crash_recovery` - Invariant holds after crash/recovery

## References

- [Orleans: Distributed Virtual Actors](https://www.microsoft.com/en-us/research/publication/orleans-distributed-virtual-actors-for-programmability-and-scalability/)
- [NOLA: Go Virtual Actors](https://github.com/richardartoul/nola)
- [Virtual Actors Paper](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/Orleans-MSR-TR-2014-41.pdf)
- [KelpieSingleActivation.tla](../tla/KelpieSingleActivation.tla)
