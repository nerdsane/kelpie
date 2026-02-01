# ADR-024: Actor Migration Protocol

## Status

Accepted

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| 3-phase protocol | 📋 Designed | TLA+ spec |
| State transfer | 📋 Designed | TLA+ spec |
| Crash recovery | 📋 Designed | TLA+ spec |
| Migration coordinator | 📋 Designed | TLA+ spec |

## Context

Kelpie needs to move actors between nodes for:

1. **Load Balancing**: Redistribute actors when nodes are overloaded
2. **Node Shutdown**: Gracefully move actors off a node before shutdown
3. **Maintenance**: Move actors for planned maintenance windows
4. **Optimization**: Colocate actors that frequently communicate

Migration must maintain the single activation guarantee: at no point should an actor be active on both source and target nodes simultaneously.

## Decision

Implement a 3-phase migration protocol with coordinator-driven execution.

### Protocol Phases

```
┌─────────────────────────────────────────────────────────────────────┐
│                   Actor Migration Protocol                           │
│                                                                      │
│  Source Node                    Target Node                          │
│  ───────────                    ───────────                          │
│       │                              │                               │
│       │  ┌──────────────────────────┐│                               │
│       │  │     PHASE 1: PREPARE     ││                               │
│       │  └──────────────────────────┘│                               │
│       │                              │                               │
│  [Deactivate Actor]                  │                               │
│       │                              │                               │
│       ├──────── Prepare Request ────▶│                               │
│       │                              │                               │
│       │◀─────── Prepare Ack ─────────┤                               │
│       │                              │                               │
│       │  ┌──────────────────────────┐│                               │
│       │  │    PHASE 2: TRANSFER     ││                               │
│       │  └──────────────────────────┘│                               │
│       │                              │                               │
│       ├──────── State Data ─────────▶│                               │
│       │                              │                               │
│       │◀─────── Transfer Ack ────────┤                               │
│       │                              │                               │
│       │  ┌──────────────────────────┐│                               │
│       │  │    PHASE 3: COMPLETE     ││                               │
│       │  └──────────────────────────┘│                               │
│       │                              │                               │
│       │                         [Activate Actor]                     │
│       │                              │                               │
│       ├──────── Complete Request ───▶│                               │
│       │                              │                               │
│       │◀─────── Complete Ack ────────┤                               │
│       │                              │                               │
│  [Migration Done]              [Actor Active]                        │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Phase Details

**Phase 1: Prepare**
1. Coordinator initiates migration
2. Source node deactivates actor (stops processing messages)
3. Target node prepares to receive actor
4. Actor is now in `Migrating` state (inactive on both nodes)

**Phase 2: Transfer**
1. Source serializes actor state
2. State transferred to target node
3. Target stores state but doesn't activate yet
4. Critical: State must be fully transferred before proceeding

**Phase 3: Complete**
1. Target node activates actor
2. Registry updated with new placement
3. Source node releases resources
4. Migration complete

### Key Design Points

1. **Source Deactivation First**: Actor is deactivated on source BEFORE any transfer begins. This prevents dual activation.

2. **State Transfer Atomicity**: Complete state must be transferred. Partial state is never exposed.

3. **Registry Update**: Placement is atomically updated in FDB at completion.

4. **Crash Recovery**:
   - Crash during Phase 1: Actor stays on source
   - Crash during Phase 2: Actor recovers on source (state not committed on target)
   - Crash during Phase 3: Actor recovers on target (state committed)

5. **Cooldown**: After migration, a cooldown period prevents immediate re-migration.

### State Serialization

```rust
struct MigrationState {
    actor_id: ActorId,
    actor_state: Bytes,      // Serialized actor state
    pending_messages: Vec<Message>,  // Queued messages during migration
    generation: u64,         // For consistency check
}
```

## Formal Specification

**TLA+ Model**: [KelpieMigration.tla](../tla/KelpieMigration.tla)

### Safety Invariants

| Invariant | Description |
|-----------|-------------|
| `MigrationAtomicity` | If migration completes, full state was transferred to target |
| `NoStateLoss` | Actor state is never lost during migration |
| `SingleActivationDuringMigration` | At most one active instance during migration |
| `MigrationRollback` | Failed migration leaves actor recoverable |
| `TypeInvariant` | All variables have correct types |

### Liveness Properties

| Property | Description |
|----------|-------------|
| `EventualMigrationCompletion` | If nodes stay alive, migration eventually completes or fails |
| `EventualRecovery` | If recovery is pending and a node is alive, actor eventually recovers |

### Model Checking Results

- **Safe config**: PASS (59 distinct states)
- **Buggy config**: FAIL - `MigrationAtomicity` violated when SkipTransfer=TRUE skips state transfer

### DST Alignment

| Failure Mode | TLA+ | DST | Notes |
|--------------|------|-----|-------|
| CrashDuringPrepare | ✅ Phase crash | ✅ | Actor stays on source |
| CrashDuringTransfer | ✅ Phase crash | ✅ | Actor recovers on source |
| CrashDuringComplete | ✅ Phase crash | ✅ | Actor recovers on target |
| PartialStateTransfer | ✅ SkipTransfer | ✅ | Buggy mode test |

## Consequences

### Positive

- **Strong Single Activation**: Guaranteed at all times during migration
- **No State Loss**: Complete state transfer or rollback
- **Crash Safe**: Deterministic recovery in all crash scenarios
- **Transparent**: Clients unaware of migration (brief unavailability)

### Negative

- **Unavailability Window**: Actor unavailable during migration
- **Coordination Overhead**: Multi-phase protocol requires coordination
- **Network Dependency**: Requires stable network for transfer phase

### Neutral

- Migration can be triggered manually or automatically
- Transfer time depends on state size

## Alternatives Considered

### 2-Phase Protocol

- Combine Prepare and Transfer into one phase
- Faster migration

**Rejected because**: Harder to reason about crash recovery. 3-phase provides clearer state transitions.

### Stop-the-World Migration

- Pause all actors, migrate, resume
- Simpler coordination

**Rejected because**: Unacceptable downtime for uninvolved actors.

### Saga Pattern with Compensation

- Execute forward, compensate on failure
- Eventually consistent

**Rejected because**: Compensation is complex. State could be in inconsistent state during compensation.

### Live Migration (Copy-on-Write)

- Keep source active while copying state
- Switch at end

**Rejected because**: Violates single activation guarantee. Complex conflict resolution needed.

## References

- [KelpieMigration.tla](../tla/KelpieMigration.tla) - TLA+ specification
- [ADR-001: Virtual Actor Model](./001-virtual-actor-model.md) - Actor model
- [ADR-004: Linearizability Guarantees](./004-linearizability-guarantees.md) - Consistency
- [Orleans Actor Migration](https://learn.microsoft.com/en-us/dotnet/orleans/grains/grain-migration)
