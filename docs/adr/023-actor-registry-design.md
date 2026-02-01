# ADR-023: Actor Registry Design

## Status

Accepted

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| Node lifecycle | 📋 Designed | TLA+ spec |
| Actor placement | 📋 Designed | TLA+ spec |
| Cache coherence | 📋 Designed | TLA+ spec |
| Failure detection | 📋 Designed | TLA+ spec |

## Context

Kelpie's virtual actor model requires a registry that:

1. **Tracks Actor Placement**: Maps ActorId to the node hosting it
2. **Enforces Single Activation**: Ensures at most one instance per actor
3. **Detects Node Failures**: Identifies failed nodes and triggers recovery
4. **Provides Discovery**: Allows any node to find where an actor is hosted
5. **Supports Caching**: Enables fast lookups without hitting central storage

The registry is critical for maintaining the single activation guarantee that underpins Kelpie's linearizability.

## Decision

Implement a centralized registry backed by FoundationDB with distributed node-local caches.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Actor Registry                            │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐   │
│  │    Node A    │    │    Node B    │    │    Node C    │   │
│  │  ┌────────┐  │    │  ┌────────┐  │    │  ┌────────┐  │   │
│  │  │ Cache  │  │    │  │ Cache  │  │    │  │ Cache  │  │   │
│  │  └───┬────┘  │    │  └───┬────┘  │    │  └───┬────┘  │   │
│  └──────┼───────┘    └──────┼───────┘    └──────┼───────┘   │
│         │                   │                   │            │
│         └───────────────────┼───────────────────┘            │
│                             │                                │
│                    ┌────────▼────────┐                       │
│                    │   FoundationDB  │                       │
│                    │  (Authoritative)│                       │
│                    └─────────────────┘                       │
└─────────────────────────────────────────────────────────────┘
```

### Node State Machine

```
┌─────────┐    join     ┌─────────┐
│  Left   │────────────▶│ Joining │
└─────────┘             └────┬────┘
     ▲                       │ complete
     │                       ▼
     │               ┌─────────────┐
     │   leave       │   Active    │◀───────┐
     │  complete     └──────┬──────┘        │
     │                      │               │
     │                      │ leave         │ recover
     │               ┌──────▼──────┐        │
     │               │   Leaving   │        │
     │               └──────┬──────┘        │
     │                      │               │
     └──────────────────────┘        ┌──────┴──────┐
                                     │   Failed    │
                                     └─────────────┘
```

### Key Design Points

1. **Authoritative Storage**: FoundationDB stores the ground truth for all placements
2. **Node-Local Caches**: Each node maintains a cache of placements for fast lookups
3. **Eventually Consistent Caches**: Caches may be stale but converge to authoritative state
4. **Heartbeat-Based Failure Detection**: Nodes send periodic heartbeats; missed heartbeats trigger failure detection
5. **Single Activation via Placement**: An actor is placed on at most one node

### Placement Data Model

```rust
// Authoritative placement in FDB
struct Placement {
    actor_id: ActorId,
    node_id: NodeId,
    generation: u64,    // Increments on each placement change
}

// Node-local cache entry
struct CacheEntry {
    actor_id: ActorId,
    node_id: Option<NodeId>,  // None if not placed
}
```

### Failure Detection Protocol

1. **Heartbeat**: Active nodes send heartbeats at regular intervals
2. **Timeout**: If heartbeat not received within threshold, node marked Suspect
3. **Confirmation**: If still no heartbeat, node marked Failed
4. **Cleanup**: Failed node's placements are cleared

## Formal Specification

**TLA+ Model**: [KelpieRegistry.tla](../tla/KelpieRegistry.tla)

### Safety Invariants

| Invariant | Description |
|-----------|-------------|
| `SingleActivation` | An actor is placed on at most one node at any time |
| `PlacementConsistency` | Placed actors are not on Failed nodes |
| `TypeOK` | All variables have correct types |

### Liveness Properties

| Property | Description |
|----------|-------------|
| `EventualFailureDetection` | Dead nodes are eventually marked as Failed |
| `EventualCacheInvalidation` | Stale cache entries on alive nodes eventually get corrected |

### Model Checking Results

- **Safe config**: PASS (6,174 distinct states, 22,845 generated)
- **Buggy config**: FAIL - `PlacementConsistency` violated when BUGGY=TRUE allows claiming on Suspect nodes

### DST Alignment

| Failure Mode | TLA+ | DST | Notes |
|--------------|------|-----|-------|
| NodeCrash | ✅ isAlive flag | ✅ | Triggers failure detection |
| HeartbeatTimeout | ✅ HeartbeatTick | ✅ | Increments missed count |
| StaleCache | ✅ cache variable | ✅ | Eventually invalidated |
| PartialFailure | ✅ heartbeatCount | ✅ | Suspect state |

## Consequences

### Positive

- **Single Activation Guarantee**: Strongly enforced via FDB transactions
- **Fast Lookups**: Local cache provides sub-millisecond lookups
- **Fault Tolerance**: Automatic failure detection and recovery
- **Consistency**: FDB provides linearizable placement updates

### Negative

- **Central Dependency**: FDB must be available for placement changes
- **Cache Staleness**: Stale caches can cause misdirected invocations
- **Failure Detection Latency**: Takes time to detect and recover from failures

### Neutral

- Heartbeat interval and timeout are tunable parameters
- Cache invalidation strategy affects freshness vs. load trade-off

## Alternatives Considered

### Consistent Hashing

- Hash actor ID to determine placement node
- No central registry needed

**Rejected because**: Cannot guarantee single activation during node joins/leaves without coordination. Rebalancing is complex.

### Distributed Hash Table (DHT)

- Chord/Kademlia-style DHT for placement
- Decentralized coordination

**Rejected because**: DHT consistency is eventual, not linearizable. Single activation guarantee is weaker.

### Gossip-Based Discovery

- Nodes gossip placement information
- Eventually consistent views

**Rejected because**: Gossip convergence time is unpredictable. Single activation may be violated during convergence.

## References

- [KelpieRegistry.tla](../tla/KelpieRegistry.tla) - TLA+ specification
- [ADR-001: Virtual Actor Model](./001-virtual-actor-model.md) - Actor model overview
- [ADR-004: Linearizability Guarantees](./004-linearizability-guarantees.md) - Consistency guarantees
- [Orleans Cluster Management](https://learn.microsoft.com/en-us/dotnet/orleans/implementation/cluster-management)
