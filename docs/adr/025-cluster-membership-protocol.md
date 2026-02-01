# ADR-025: Cluster Membership Protocol

## Status

Accepted

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| Node state machine | 📋 Designed | TLA+ spec |
| Heartbeat protocol | 📋 Designed | TLA+ spec |
| Primary election | 📋 Designed | TLA+ spec |
| Partition handling | 📋 Designed | TLA+ spec |

## Context

Kelpie operates as a distributed cluster and needs:

1. **Node Discovery**: Know which nodes are part of the cluster
2. **Failure Detection**: Detect when nodes fail or become unreachable
3. **Membership Agreement**: All nodes agree on current membership
4. **Primary Election**: Elect a primary node for coordination tasks
5. **Partition Handling**: Handle network partitions safely

The membership protocol must prevent split-brain scenarios where multiple partitions operate independently with conflicting state.

## Decision

Implement a heartbeat-based membership protocol with Raft-style primary election.

### Node State Machine

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Node State Machine                              │
│                                                                      │
│     ┌────────┐                                                       │
│     │  Left  │◀──────────────────────────────────────┐               │
│     └───┬────┘                                       │               │
│         │ join                                       │ leave         │
│         ▼                                            │ complete      │
│     ┌────────┐     complete     ┌────────┐          │               │
│     │Joining │────────────────▶│ Active │──────────┼───────┐       │
│     └────────┘                  └───┬────┘          │       │       │
│                                     │               │       │       │
│                                     │ leave         │       │       │
│                                     ▼               │       │       │
│                               ┌─────────┐───────────┘       │       │
│                               │ Leaving │                   │       │
│                               └─────────┘                   │       │
│                                                             │       │
│                               ┌─────────┐                   │       │
│                               │ Failed  │◀──────────────────┘       │
│                               └────┬────┘   failure detected        │
│                                    │                                 │
│                                    │ recover                         │
│                                    ▼                                 │
│                               (back to Left)                         │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Heartbeat Protocol

1. **Interval**: Each active node sends heartbeat every `HEARTBEAT_INTERVAL_MS`
2. **Timeout**: If no heartbeat received for `MAX_HEARTBEAT_MISS * HEARTBEAT_INTERVAL_MS`, mark node as suspect
3. **Confirmation**: If still no heartbeat, mark node as failed
4. **Reset**: Receiving heartbeat resets the counter and clears suspect status

### Primary Election

Primary election follows Raft-style term-based approach:

1. **Terms**: Each primary claim has a monotonically increasing term number
2. **Quorum**: A node can only become primary if it can reach a majority of ALL nodes
3. **Step-Down**: A primary must step down if it loses quorum
4. **Conflict Resolution**: Higher term always wins

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Primary Election Rules                            │
│                                                                      │
│  To become primary, a node must:                                     │
│  1. Be in Active state                                               │
│  2. Reach majority of ALL nodes in cluster (not just its view)       │
│  3. No other node has a valid primary claim                          │
│                                                                      │
│  A primary claim is valid only if:                                   │
│  1. The primary can still reach a majority                           │
│  2. The primary has the highest term among all primaries             │
│                                                                      │
│  A primary must step down when:                                      │
│  - It can no longer reach a majority of ALL nodes                    │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Split-Brain Prevention

Split-brain is prevented by:

1. **Quorum Requirement**: Primary must maintain majority of ENTIRE cluster
2. **Step-Down on Partition**: Primary in minority partition steps down
3. **No Shrinking Quorum**: Quorum is always based on total cluster size, not view size
4. **Term-Based Ordering**: New primaries get higher terms, preventing conflicts after heal

### Partition Handling

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Partition Handling                                │
│                                                                      │
│  Scenario: 5-node cluster partitions into 3+2                        │
│                                                                      │
│  ┌─────────────┐         ┌─────────────┐                            │
│  │  Partition A │         │ Partition B │                            │
│  │  (3 nodes)   │         │  (2 nodes)  │                            │
│  │  ─────────   │         │  ─────────  │                            │
│  │  Has quorum  │         │  No quorum  │                            │
│  │  (3 > 5/2)   │    X    │  (2 <= 5/2) │                            │
│  │  Can elect   │         │  Cannot     │                            │
│  │  primary     │         │  elect      │                            │
│  └─────────────┘         └─────────────┘                            │
│                                                                      │
│  Result: Only Partition A can operate. B is unavailable.             │
│  When healed: B rejoins, any stale primary steps down.               │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Membership View Synchronization

Active nodes that can communicate synchronize their membership views:
- Higher view number takes precedence
- Merged view includes both communicating nodes
- View numbers increment on membership changes

## Formal Specification

**TLA+ Model**: [KelpieClusterMembership.tla](../tla/KelpieClusterMembership.tla)

### Safety Invariants

| Invariant | Description |
|-----------|-------------|
| `NoSplitBrain` | At most one node has a valid primary claim |
| `MembershipConsistency` | Active nodes with same view number have same membership view |
| `JoinAtomicity` | A node is either fully joined (Active with non-empty view) or not joined |
| `LeaveDetectionWeak` | Left nodes are not in any active node's membership view |
| `TypeOK` | All variables have correct types |

### Liveness Properties

| Property | Description |
|----------|-------------|
| `EventualMembershipConvergence` | If network heals and nodes are stable, all active nodes eventually have same view |

### Model Checking Results

- **Safe config**: PASS - All invariants hold
- **Buggy config**: FAIL - `NoSplitBrain` violated when BUGGY_MODE=TRUE allows election without quorum check

### DST Alignment

| Failure Mode | TLA+ | DST | Notes |
|--------------|------|-----|-------|
| NetworkPartition | ✅ partitioned set | ✅ | Bidirectional partitions |
| HeartbeatMiss | ✅ heartbeatReceived | ✅ | Triggers failure detection |
| NodeCrash | ✅ MarkNodeFailed | ✅ | Node marked Failed |
| PartitionHeal | ✅ HealPartition | ✅ | Resolves split-brain atomically |

## Consequences

### Positive

- **No Split-Brain**: Proven by TLA+ model checking
- **Clear Failure Detection**: Heartbeat-based with tunable thresholds
- **Automatic Recovery**: Nodes can rejoin after failure
- **CP Semantics**: Consistency over availability during partitions

### Negative

- **Unavailability During Partition**: Minority partition cannot operate
- **Election Latency**: Term-based election takes time
- **Heartbeat Overhead**: Regular heartbeat messages consume resources

### Neutral

- Heartbeat interval is configurable (trade-off: faster detection vs. more traffic)
- Quorum-based approach is well-understood from Raft/Paxos

## Alternatives Considered

### SWIM Protocol

- Gossip-based membership with infection-style dissemination
- More scalable for large clusters

**Rejected because**: SWIM provides weaker consistency guarantees. Split-brain prevention is harder to reason about.

### External Coordination (etcd/ZooKeeper)

- Delegate membership to external consensus system
- Proven reliability

**Rejected because**: Additional operational dependency. Kelpie already uses FDB which provides similar guarantees.

### Virtual Synchrony (Isis/JGroups)

- Atomic broadcast with view changes
- Strong ordering guarantees

**Rejected because**: Higher complexity and latency. Overkill for our membership needs.

## References

- [KelpieClusterMembership.tla](../tla/KelpieClusterMembership.tla) - TLA+ specification
- [ADR-004: Linearizability Guarantees](./004-linearizability-guarantees.md) - Consistency model
- [Raft Consensus](https://raft.github.io/) - Term-based election
- [SWIM Protocol](https://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/SWIM.pdf) - Alternative approach
