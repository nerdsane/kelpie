# Kelpie TLA+ Specifications

This directory contains TLA+ specifications for formally verifying Kelpie's distributed actor system.

## Overview

TLA+ is a formal specification language for describing concurrent and distributed systems. These specs verify critical safety and liveness properties of Kelpie actors.

## Prerequisites

Download TLA+ tools:
```bash
curl -L -o ~/tla2tools.jar https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar
```

Requires Java 11+.

## Specifications

### KelpieLease.tla
Models the lease-based actor ownership protocol from ADR-004.

**Features (Issue #42):**
- TTL-based lease expiration with explicit time modeling
- Grace period before deactivation (prevents instant eviction)
- False suspicion handling (GC pause recovery)
- Clock skew tolerance between nodes
- Fencing tokens for stale write prevention
- Node crash/restart simulation

#### Safety Invariants
| Invariant | Description |
|-----------|-------------|
| `TypeOK` | Type correctness of all variables |
| `LeaseUniqueness` | At most one node believes it holds a valid lease per actor |
| `RenewalRequiresOwnership` | Only lease holder can renew |
| `LeaseValidityBounds` | Lease expiry within configured bounds |
| `FencingTokenMonotonic` | Fencing tokens never decrease |
| `ClockSkewSafety` | Node clocks within acceptable bounds |
| `GracePeriodRespected` | No instant deactivation during grace period |
| `FalseSuspicionSafety` | False suspicion doesn't cause immediate lease loss |

#### Liveness Properties
| Property | Description |
|----------|-------------|
| `EventualLeaseResolution` | Eventually a lease is granted or expires cleanly |
| `FalseSuspicionRecovery` | False suspicions eventually resolve |

#### Actions (12 total)
1. **AcquireLeaseSafe** - Atomic CAS lease acquisition with fencing token
2. **RenewLeaseSafe** - Lease renewal by current holder
3. **ReleaseLease** - Voluntary lease release
4. **SuspectNodeDead** - Mark node as suspected dead
5. **RecoverFromSuspicion** - Node proves liveness after false suspicion
6. **NodeCrash** - Node actually crashes
7. **NodeRestart** - Crashed node restarts
8. **ClockDrift** - Model bounded clock skew
9. **TickClock** - Advance simulated time
10. **WriteWithFencing** - Validate fencing token on write
11. **AcquireLeaseNoCheck** (buggy) - Race condition lease acquisition
12. **RenewLeaseBuggy** (buggy) - Renewal without proper checks

#### Verification Status
- **Safety invariants**: 130M+ states explored, no violations found
- **Liveness**: Uses strong fairness (SF_vars) for AcquireLeaseSafe to ensure progress despite suspicion/recovery cycles
- **Quick check**: Use `KelpieLease_Minimal.cfg` for rapid iteration
- **Safety only**: Use `KelpieLease_SafetyOnly.cfg` to skip expensive liveness checking

#### TLC Commands
```bash
# Full verification (may take extended time due to liveness checking)
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease.cfg KelpieLease.tla

# Safety only (faster)
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease_SafetyOnly.cfg KelpieLease.tla

# Minimal model (quick iteration)
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease_Minimal.cfg KelpieLease.tla

# Buggy version (should fail)
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease_Buggy.cfg KelpieLease.tla
```

### KelpieActorLifecycle.tla
Models the lifecycle of a Kelpie virtual actor (ADR-001 G1.3, G1.5).

#### Safety Invariants
- **IdleTimeoutRespected**: Actor idle beyond timeout must be deactivating/deactivated
- **GracefulDeactivation**: Pending messages drained before deactivate completes
- **NoResurrection**: Deactivated actor cannot process without re-activation
- **LifecycleOrdering**: States follow Inactive → Activating → Active → Deactivating → Inactive

#### Liveness Properties
- **EventualDeactivation**: Idle actors eventually deactivated
- **EventualActivation**: First invocation eventually activates actor
- **MessageProgress**: Pending messages eventually processed or rejected

#### Bug Variants
- **BUGGY_DEACTIVATE**: CompleteDeactivation_WithPending - violates GracefulDeactivation
- **BUGGY_INVOKE**: ProcessMessage_WhenDeactivating - violates LifecycleOrdering

- **TLC Results**: PASS (safe) / FAIL GracefulDeactivation (buggy)

### KelpieMigration.tla
Models Kelpie's 3-phase actor migration protocol.
- **TLC Results**: PASS (59 distinct states) / FAIL MigrationAtomicity (buggy)

### KelpieActorState.tla
Models actor state management and transaction semantics.
- **TLC Results**: PASS (60 distinct states) / FAIL RollbackCorrectness (buggy)

### KelpieFDBTransaction.tla
Models FoundationDB transaction semantics (ADR-002 G2.4, ADR-004 G4.1).

#### Safety Invariants
| Invariant | Description |
|-----------|-------------|
| `TypeOK` | Type correctness of all variables |
| `SerializableIsolation` | Committed transactions appear in some serial order |
| `ConflictDetection` | Concurrent writes to same key cause at least one abort |
| `AtomicCommit` | Transaction commit is all-or-nothing |
| `ReadYourWrites` | Transaction sees its own uncommitted writes |
| `SnapshotReads` | Reads see consistent snapshot from transaction start |

#### Liveness Properties
| Property | Description |
|----------|-------------|
| `EventualTermination` | Every started transaction eventually commits or aborts |
| `EventualCommit` | Non-conflicting transactions eventually commit |

#### Bug Variant
The buggy config sets `EnableConflictDetection = FALSE`, which skips conflict checks and violates `SerializableIsolation` and `ConflictDetection`.

- **TLC Results**: PASS (56,193 distinct states) / FAIL SerializableIsolation (buggy)

### KelpieTeleport.tla
Models teleport state consistency for VM snapshot operations.
- **TLC Results**: PASS (1,508 distinct states) / FAIL ArchitectureValidation (buggy)

### KelpieSingleActivation.tla
Models the single-activation guarantee with FDB transaction semantics.

**Features:**
- BUGGY mode: Skips OCC version check, allowing split-brain (multiple active nodes)
- Models TOCTOU race condition when version checking is disabled

**Bug Variant:**
- `BUGGY=TRUE`: Commits based on stale read-time state, ignoring concurrent modifications

- **TLC Results**: PASS (714 distinct states, depth 27) / FAIL SingleActivation (buggy)

### KelpieRegistry.tla
Models the actor registry with node lifecycle and cache coherence.
- **TLC Results**: PASS (6,174 distinct states, 22,845 generated) / FAIL PlacementConsistency (buggy*)

### KelpieWAL.tla
Models the Write-Ahead Log for operation durability and atomicity.
- **TLC Results**: PASS (70,713 states single client / 2,548,321 states concurrent) / FAIL Idempotency (buggy*)

*Note: Buggy configs created but require BUGGY constant to be added to .tla files for full testing.

### KelpieAgentActor.tla
Models the AgentActor state machine from ADR-013/014.
- **Invariants**: SingleActivation, CheckpointIntegrity, MessageProcessingOrder, TypeOK
- **Liveness**: EventualCompletion, EventualCrashRecovery
- **TLC Results**: PASS (860 distinct states) / FAIL CheckpointIntegrity (buggy)

#### Safety Invariants
| Invariant | Description | BUGGY Mode Violation |
|-----------|-------------|----------------------|
| `TypeOK` | Type correctness of all variables | N/A |
| `SingleActivation` | At most one node running agent | N/A |
| `CheckpointIntegrity` | FDB checkpoint ≤ current iteration | Skip FDB write |
| `MessageProcessingOrder` | Messages processed in order | N/A |

#### Actions (10 total)
1. **EnqueueMessage** - Add message to queue
2. **StartAgent(n)** - Node starts agent, reads FDB checkpoint
3. **CompleteStartup(n)** - Agent transitions to Running
4. **ExecuteIteration(n)** - Process message, write checkpoint (BUGGY: skip write)
5. **StopAgent(n)** - Initiate graceful shutdown
6. **CompleteStop(n)** - Finish shutdown
7. **NodeCrash(n)** - Node crashes, loses local state
8. **NodeRecover(n)** - Node recovers, ready to restart
9. **PauseAgent(n)** - Agent pauses (heartbeat pause)
10. **ResumeAgent(n)** - Agent resumes after pause

### KelpieClusterMembership.tla
Models cluster membership protocol with:
- Node states: Joining, Active, Leaving, Failed, Left
- Heartbeat-based failure detection
- Network partitions
- Primary election with terms (Raft-style)

#### Safety Invariants
| Invariant | Description |
|-----------|-------------|
| `TypeOK` | Type safety - all variables have expected types |
| `JoinAtomicity` | Node is either fully joined or not joined |
| `NoSplitBrain` | At most one node has valid primary claim |

#### TLC Results
- **Safe version**: PASS - All invariants hold
- **Buggy version**: FAIL - NoSplitBrain violated

### KelpieLinearizability.tla
Models linearization points for client-visible operations as defined in ADR-004.
- **TLC Results**: PASS (10,680 distinct states)

#### Linearization Points (per ADR-004)
| Operation | Linearization Point | Description |
|-----------|---------------------|-------------|
| `Claim` | FDB transaction commit | Actor ownership acquisition |
| `Release` | FDB transaction commit | Actor ownership release |
| `Read` | FDB snapshot read | Query current actor owner |
| `Dispatch` | Activation check | Message dispatch to active actor |

#### Safety Invariants
| Invariant | Description |
|-----------|-------------|
| `TypeOK` | Type correctness of all variables |
| `SequentialPerActor` | Operations on same actor are totally ordered |
| `ReadYourWrites` | Same client sees own successful claims (per-client) |
| `MonotonicReads` | Same client's reads don't regress (per-client) |
| `DispatchConsistency` | Dispatch succeeds iff actor is owned |
| `OwnershipConsistency` | owner_client and ownership are always in sync |

#### Authorization
- Only the client who claimed an actor can release it
- Release by unauthorized client fails with "fail" response

#### Liveness Properties
| Property | Description |
|----------|-------------|
| `EventualCompletion` | Every pending operation eventually completes |
| `EventualClaim` | Claims on free actors eventually succeed |

---

## Running TLC Model Checker

### Safe Configurations (should pass)
```bash
cd docs/tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease.cfg KelpieLease.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieActorLifecycle.cfg KelpieActorLifecycle.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieMigration.cfg KelpieMigration.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieActorState.cfg KelpieActorState.tla
java -XX:+UseParallelGC -Xmx4g -jar ~/tla2tools.jar -deadlock -config KelpieFDBTransaction.cfg KelpieFDBTransaction.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieTeleport.cfg KelpieTeleport.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieSingleActivation.cfg KelpieSingleActivation.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieRegistry.cfg KelpieRegistry.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieWAL.cfg KelpieWAL.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieClusterMembership.cfg KelpieClusterMembership.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieAgentActor.cfg KelpieAgentActor.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLinearizability.cfg KelpieLinearizability.tla
```

### Buggy Configurations (should fail)
```bash
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieLease_Buggy.cfg KelpieLease.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieActorLifecycle_Buggy.cfg KelpieActorLifecycle.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieMigration_Buggy.cfg KelpieMigration.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieActorState_Buggy.cfg KelpieActorState.tla
java -XX:+UseParallelGC -Xmx4g -jar ~/tla2tools.jar -deadlock -config KelpieFDBTransaction_Buggy.cfg KelpieFDBTransaction.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieTeleport_Buggy.cfg KelpieTeleport.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieClusterMembership_Buggy.cfg KelpieClusterMembership.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieAgentActor_Buggy.cfg KelpieAgentActor.tla
# New buggy configs (require BUGGY constant added to specs):
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieRegistry_Buggy.cfg KelpieRegistry.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieSingleActivation_Buggy.cfg KelpieSingleActivation.tla
java -XX:+UseParallelGC -jar ~/tla2tools.jar -deadlock -config KelpieWAL_Buggy.cfg KelpieWAL.tla
```

## Consistency Notes (DST Alignment)

### NULL Sentinel Styles
Different specs use different sentinel values for "no value":
- `KelpieRegistry`: `NULL`
- `KelpieSingleActivation`: `NONE`
- `KelpieLease`: `NoHolder`, `NONE`

**Recommendation**: Standardize on `NONE` (TLA+ convention).

### BUGGY Mode Patterns
Specs use different mechanisms for bug injection:
| Pattern | Specs Using It |
|---------|---------------|
| `CONSTANT BUGGY` | KelpieClusterMembership, KelpieAgentActor, KelpieSingleActivation |
| `SafeMode = FALSE` | KelpieActorState |
| `SkipTransfer = TRUE` | KelpieMigration |
| Config-level override | KelpieTeleport, KelpieFDBTransaction |
| **Needs BUGGY added** | KelpieRegistry, KelpieWAL |

### DST Fault Alignment
| TLA+ Spec | Crash Modeling | DST Alignment |
|-----------|---------------|---------------|
| KelpieWAL | Yes (Crash, Recovery) | Aligned |
| KelpieRegistry | Yes (NodeCrash) | Aligned |
| KelpieMigration | Yes (phase crashes) | Aligned |
| KelpieClusterMembership | Yes (partition, crash) | Aligned |
| KelpieAgentActor | Yes (NodeCrash, NodeRecover) | Aligned |
| KelpieFDBTransaction | No | Needs crash-during-commit |
| KelpieSingleActivation | Yes (BUGGY mode for TOCTOU) | Aligned |
| KelpieLease | Yes (NodeCrash, NodeRestart, FalseSuspicion) | Aligned |
| KelpieTeleport | No | Needs crash-during-snapshot |
| KelpieActorState | No | Needs crash-during-invocation |
| KelpieActorLifecycle | No | Needs crash-during-activation |


## Cross-Module Composition

Kelpie's distributed guarantees are verified across multiple TLA+ specifications. This section documents how specifications compose to provide global single activation.

### Composition Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      Global Single Activation                            │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────────┐   ┌──────────────────┐   ┌────────────────────┐   │
│  │ KelpieSingle     │   │ KelpieLease      │   │ KelpieLineariz-    │   │
│  │ Activation.tla   │   │ .tla             │   │ ability.tla        │   │
│  │                  │   │                  │   │                    │   │
│  │ FDB OCC for      │   │ Lease expiry     │   │ Client-visible     │   │
│  │ atomic claim     │   │ and renewal      │   │ ordering           │   │
│  └────────┬─────────┘   └────────┬─────────┘   └─────────┬──────────┘   │
│           │                      │                       │               │
│           └──────────────────────┼───────────────────────┘               │
│                                  │                                       │
│                    ┌─────────────▼─────────────┐                        │
│                    │ KelpieRegistry.tla        │                        │
│                    │                           │                        │
│                    │ Actor placement and       │                        │
│                    │ cache coherence           │                        │
│                    └─────────────┬─────────────┘                        │
│                                  │                                       │
│           ┌──────────────────────┼──────────────────────┐               │
│           │                      │                      │               │
│  ┌────────▼─────────┐   ┌────────▼─────────┐   ┌───────▼───────────┐   │
│  │ KelpieAgent      │   │ KelpieCluster    │   │ KelpieMigration   │   │
│  │ Actor.tla        │   │ Membership.tla   │   │ .tla              │   │
│  │                  │   │                  │   │                   │   │
│  │ Agent state      │   │ Split-brain      │   │ Migration         │   │
│  │ machine          │   │ prevention       │   │ atomicity         │   │
│  └──────────────────┘   └──────────────────┘   └───────────────────┘   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Why Per-Module Verification is Sufficient

Rather than a single unified specification, Kelpie uses modular verification because:

1. **State Space Tractability**: A unified spec combining all modules would have an enormous state space (product of all module state spaces). Per-module specs stay tractable for TLC.

2. **Clear Abstraction Boundaries**: Each module has well-defined inputs and outputs:
   - `KelpieSingleActivation` assumes FDB OCC semantics
   - `KelpieRegistry` assumes single activation from `KelpieSingleActivation`
   - `KelpieLinearizability` assumes registry correctness

3. **Shared Invariants**: Key invariants are verified across multiple specs:
   | Invariant | Verified In |
   |-----------|-------------|
   | At most one active instance | KelpieSingleActivation, KelpieAgentActor, KelpieClusterMembership |
   | Lease uniqueness | KelpieLease, KelpieRegistry |
   | No split-brain | KelpieClusterMembership |
   | Linearizable history | KelpieLinearizability |

4. **Assumption Chaining**: Each spec's postconditions become the next spec's preconditions:
   - FDB provides serializable transactions → SingleActivation guarantees uniqueness
   - SingleActivation guarantees uniqueness → Registry maintains consistent placement
   - Registry maintains placement → Linearizability holds for clients

### Verification Evidence

| Composition Layer | Verified By | Key Invariant |
|-------------------|-------------|---------------|
| FDB Transactions | KelpieFDBTransaction.tla | SerializableIsolation |
| Single Activation | KelpieSingleActivation.tla | SingleActivation, ConsistentHolder |
| Lease Ownership | KelpieLease.tla | LeaseUniqueness |
| Client Ordering | KelpieLinearizability.tla | ReadYourWrites, MonotonicReads |
| Cluster Membership | KelpieClusterMembership.tla | NoSplitBrain |
| Migration | KelpieMigration.tla | MigrationAtomicity |

---

## Spec-to-ADR Cross-References

| TLA+ Specification | Related ADR |
|--------------------|-------------|
| KelpieWAL.tla | [ADR-022: WAL Design](../adr/022-wal-design.md) |
| KelpieRegistry.tla | [ADR-023: Actor Registry Design](../adr/023-actor-registry-design.md) |
| KelpieMigration.tla | [ADR-024: Actor Migration Protocol](../adr/024-actor-migration-protocol.md) |
| KelpieClusterMembership.tla | [ADR-025: Cluster Membership Protocol](../adr/025-cluster-membership-protocol.md) |
| KelpieLease.tla | [ADR-004: Linearizability Guarantees](../adr/004-linearizability-guarantees.md) |
| KelpieSingleActivation.tla | [ADR-004: Linearizability Guarantees](../adr/004-linearizability-guarantees.md) |
| KelpieLinearizability.tla | [ADR-004: Linearizability Guarantees](../adr/004-linearizability-guarantees.md) |
| KelpieAgentActor.tla | [ADR-013: Actor-Based Agent Server](../adr/013-actor-based-agent-server.md) |

## References

- [ADR-001: Virtual Actor Model](../adr/001-virtual-actor-model.md)
- [ADR-002: FoundationDB Integration](../adr/002-foundationdb-integration.md)
- [ADR-004: Linearizability Guarantees](../adr/004-linearizability-guarantees.md)
- [ADR-008: Transaction API](../adr/008-transaction-api.md)
- [ADR-020: Consolidated VM Crate](../adr/020-consolidated-vm-crate.md)
- [ADR-022: WAL Design](../adr/022-wal-design.md)
- [ADR-023: Actor Registry Design](../adr/023-actor-registry-design.md)
- [ADR-024: Actor Migration Protocol](../adr/024-actor-migration-protocol.md)
- [ADR-025: Cluster Membership Protocol](../adr/025-cluster-membership-protocol.md)
- [ADR-026: MCP Tool Integration](../adr/026-mcp-tool-integration.md)
- [ADR-027: Sandbox Execution Design](../adr/027-sandbox-execution-design.md)
- [TLA+ Home](https://lamport.azurewebsites.net/tla/tla.html)
- [TLC Model Checker](https://lamport.azurewebsites.net/tla/tools.html)
