# FdbRegistry Implementation Remediation Plan

**Status:** IN PROGRESS
**Created:** 2026-01-27
**Issue:** #77 - Critical gaps found in implementation review
**Spec:** specs/077-fdbregistry-multinode-cluster.md

## Problem Statement

Critical review revealed the spec was marked COMPLETE but has significant gaps:

1. **DST tests use simulation mocks, NOT production code** - violates DST-1
2. **Missing TLA+ actions** - SyncViews, DetectFailure, NodeRecover not implemented
3. **FR-7 Actor Migration not implemented** - MigrationQueue doesn't exist
4. **TigerStyle violations** - cluster.rs has <1 assertion per function

## Issues Summary

### 🔴 Critical (Must Fix)

| ID | Issue | Impact | Status |
|----|-------|--------|--------|
| C1 | DST tests use `SimClusterState` not `ClusterMembership` | Tests don't verify production code | ✅ FIXED (587e70d7) |
| C2 | `SyncViews` not implemented | Membership views won't converge after partition heal | ✅ FIXED |
| C3 | `MigrationQueue` missing | Actor migration on node failure broken | ✅ FIXED |
| C4 | `DetectFailure` not implemented | Heartbeat timeout won't trigger failure | ✅ FIXED |
| C5 | `NodeRecover` not implemented | Failed nodes can't rejoin | ✅ FIXED |

### 🟡 Medium (Should Fix)

| ID | Issue | Impact |
|----|-------|--------|
| M1 | cluster.rs: 0.97 assertions/fn (need 2+) | TigerStyle non-compliance |
| M2 | Production unwrap at cluster.rs:799 | Potential panic |
| M3 | Method naming doesn't match TLA+ | Confusion, maintenance burden |

### 🟢 Working Correctly

- `NodeState` enum matches TLA+ exactly
- State transition validation (`can_transition_to`)
- `MembershipView` with quorum logic
- `PrimaryInfo` with term-based election
- Module exports from lib.rs

## Remediation Plan

### Phase 1: Fix DST Tests to Use Production Code (HIGHEST PRIORITY)

**Goal:** Make DST tests verify actual `ClusterMembership`, not simulation

**Why First:** Without this, we can't verify any other fixes work.

#### Tasks

1.1. **Refactor `ClusterMembership` for testability**
   - Add constructor that accepts `TimeProvider`
   - Add constructor that accepts mock storage (or use SimStorage)
   - Ensure no direct FDB calls without abstraction

1.2. **Create `MockStorageBackend` for DST**
   ```rust
   // In kelpie-dst or kelpie-registry
   pub struct MockClusterStorage {
       nodes: RwLock<HashMap<NodeId, ClusterNodeInfo>>,
       membership_view: RwLock<MembershipView>,
       primary: RwLock<Option<PrimaryInfo>>,
       primary_term: RwLock<u64>,
   }

   impl ClusterStorageBackend for MockClusterStorage { ... }
   ```

1.3. **Rewrite DST tests to use production types**
   ```rust
   // BEFORE (current - BAD)
   let cluster = Arc::new(SimClusterState::new());

   // AFTER (correct)
   let storage = MockClusterStorage::new();
   let time = SimClock::new(0);
   let cluster = ClusterMembership::new_with_providers(storage, time);
   ```

1.4. **Verify invariants against real `ClusterMembership` state**

**Acceptance Criteria:**
- [ ] `SimClusterState` removed from test file
- [ ] `SimNodeInfo` removed from test file
- [ ] Tests instantiate `ClusterMembership` directly
- [ ] All 8 tests still pass
- [ ] Tests actually exercise production code paths

**Estimated Effort:** 2-3 days

---

### Phase 2: Implement Missing TLA+ Actions

**Goal:** Complete the TLA+ action coverage in production code

#### Tasks

2.1. **Implement `detect_failure` (TLA+ DetectFailure)**
   ```rust
   impl ClusterMembership {
       /// Detect failure based on heartbeat timeout
       /// TLA+ action: DetectFailure(detector, target)
       pub async fn detect_failure(&self, target: &NodeId) -> RegistryResult<bool> {
           assert!(self.local_node_id != *target, "cannot detect self as failed");

           let node = self.get_node(target).await?;
           let now = self.time_provider.now_ms();

           if node.is_heartbeat_timeout(now, HEARTBEAT_FAILURE_THRESHOLD * HEARTBEAT_INTERVAL_MS) {
               // Remove from membership view
               self.remove_from_view(target).await?;
               return Ok(true);
           }
           Ok(false)
       }
   }
   ```

2.2. **Implement `node_recover` (TLA+ NodeRecover)**
   ```rust
   /// Failed node recovers and can rejoin
   /// TLA+ action: NodeRecover(n)
   pub async fn node_recover(&self, node_id: &NodeId) -> RegistryResult<()> {
       let mut node = self.get_node_mut(node_id).await?;
       assert!(node.state == NodeState::Failed, "can only recover from Failed state");

       node.state = NodeState::Left;
       node.primary_term = 0;
       self.save_node(&node).await?;

       Ok(())
   }
   ```

2.3. **Implement `sync_views` (TLA+ SyncViews)**
   ```rust
   /// Synchronize membership views between two nodes
   /// TLA+ action: SyncViews(n1, n2)
   pub async fn sync_views(&self, other_view: &MembershipView) -> RegistryResult<MembershipView> {
       let my_view = self.get_membership_view().await?;

       if my_view.view_number == other_view.view_number {
           // Same view number - must have same content
           assert_eq!(my_view.active_nodes, other_view.active_nodes,
               "MembershipConsistency violation: same view number, different nodes");
           return Ok(my_view);
       }

       // Merge views
       let now = self.time_provider.now_ms();
       let merged = my_view.merge(other_view, now);
       self.save_membership_view(&merged).await?;

       Ok(merged)
   }
   ```

2.4. **Implement `send_heartbeat` (TLA+ SendHeartbeat)**
   ```rust
   /// Send heartbeat to update last_heartbeat_ms
   /// TLA+ action: SendHeartbeat(n)
   pub async fn send_heartbeat(&self) -> RegistryResult<()> {
       let now = self.time_provider.now_ms();
       let mut node = self.get_local_node_mut().await?;

       assert!(node.state == NodeState::Active, "only active nodes send heartbeats");

       node.last_heartbeat_ms = now;
       self.save_node(&node).await?;

       Ok(())
   }
   ```

**Acceptance Criteria:**
- [ ] All 4 missing TLA+ actions implemented
- [ ] Each action has 2+ assertions (TigerStyle)
- [ ] DST tests added for each action
- [ ] Method names reference TLA+ spec in docs

**Estimated Effort:** 2 days

---

### Phase 3: Implement Actor Migration (FR-7)

**Goal:** Complete actor migration when node fails

#### Tasks

3.1. **Add `MigrationQueue` to membership.rs**
   ```rust
   /// Queue of actors pending migration after node failure
   #[derive(Debug, Clone, Default)]
   pub struct MigrationQueue {
       /// Actors to migrate: (actor_id, from_node, reason)
       pending: Vec<(ActorId, NodeId, MigrationReason)>,
   }

   #[derive(Debug, Clone)]
   pub enum MigrationReason {
       NodeFailed,
       NodeLeaving,
       LoadBalancing,
   }
   ```

3.2. **Add `queue_actors_for_migration` to ClusterMembership**
   ```rust
   /// Queue all actors on a failed node for migration
   pub async fn queue_actors_for_migration(&self, failed_node: &NodeId) -> RegistryResult<usize> {
       assert!(self.get_node(failed_node).await?.state == NodeState::Failed);

       let actors = self.get_actors_on_node(failed_node).await?;
       let count = actors.len();

       for actor_id in actors {
           self.migration_queue.push((actor_id, failed_node.clone(), MigrationReason::NodeFailed));
       }

       assert!(self.migration_queue.len() >= count);
       Ok(count)
   }
   ```

3.3. **Add `process_migration_queue` method**
   ```rust
   /// Process pending migrations, returning actors that were migrated
   pub async fn process_migration_queue(&self) -> RegistryResult<Vec<(ActorId, NodeId)>> {
       let mut migrated = vec![];

       while let Some((actor_id, from_node, reason)) = self.migration_queue.pop() {
           // Find new node using placement strategy
           let new_node = self.select_node_for_actor(&actor_id).await?;

           // Update actor placement atomically
           self.migrate_actor(&actor_id, &from_node, &new_node).await?;

           migrated.push((actor_id, new_node));
       }

       Ok(migrated)
   }
   ```

3.4. **Integrate with `handle_node_failure`**
   ```rust
   /// Handle node failure: mark failed, queue migrations, update views
   pub async fn handle_node_failure(&self, failed_node: &NodeId) -> RegistryResult<()> {
       // 1. Mark node as failed
       self.mark_node_failed(failed_node).await?;

       // 2. Remove from membership view
       self.remove_from_view(failed_node).await?;

       // 3. If failed node was primary, trigger re-election
       if self.is_primary(failed_node).await? {
           self.clear_primary().await?;
       }

       // 4. Queue actors for migration
       let count = self.queue_actors_for_migration(failed_node).await?;
       tracing::info!(node = %failed_node, actors = count, "queued actors for migration");

       Ok(())
   }
   ```

**Acceptance Criteria:**
- [ ] `MigrationQueue` struct exists
- [ ] `queue_actors_for_migration` implemented
- [ ] `process_migration_queue` implemented
- [ ] `handle_node_failure` integrates all steps
- [ ] DST test verifies actor migration on failure
- [ ] Single activation maintained during migration

**Estimated Effort:** 2 days

---

### Phase 4: TigerStyle Compliance

**Goal:** Meet 2+ assertions per function requirement

#### Tasks

4.1. **Add precondition assertions to cluster.rs functions**
   - Every `pub fn` and `pub async fn` needs input validation
   - Add postcondition assertions where state changes

4.2. **Fix production unwrap at line 799**
   ```rust
   // BEFORE (BAD)
   let term = u64::from_be_bytes(data.as_ref().try_into().unwrap());

   // AFTER (GOOD)
   let bytes: [u8; 8] = data.as_ref()
       .try_into()
       .map_err(|_| RegistryError::InvalidData {
           reason: "primary_term must be 8 bytes".into()
       })?;
   let term = u64::from_be_bytes(bytes);
   ```

4.3. **Add assertions checklist**
   Review each function and add:
   - Precondition: validate inputs
   - Postcondition: validate state after mutation

**Acceptance Criteria:**
- [ ] cluster.rs has 2+ assertions per function (currently 0.97)
- [ ] No `.unwrap()` in production code paths
- [ ] `cargo clippy` clean

**Estimated Effort:** 1 day

---

### Phase 5: Verification & Documentation

**Goal:** Ensure all fixes work together and are verified

#### Tasks

5.1. **Run full DST suite**
   ```bash
   DST_SEED=12345 cargo test -p kelpie-dst --test cluster_membership_production_dst
   ```

5.2. **Verify determinism**
   ```bash
   DST_SEED=12345 cargo test -p kelpie-dst cluster_membership > run1.txt
   DST_SEED=12345 cargo test -p kelpie-dst cluster_membership > run2.txt
   diff run1.txt run2.txt  # Must be empty
   ```

5.3. **Update spec status**
   - Unmark spec as COMPLETE until all phases done
   - Re-verify each acceptance criterion
   - Mark COMPLETE only when all pass

5.4. **Document TLA+ to Rust mapping**
   Add table to cluster.rs:
   ```rust
   //! ## TLA+ Action Mapping
   //!
   //! | TLA+ Action | Rust Method |
   //! |-------------|-------------|
   //! | NodeJoin | `join_cluster()` |
   //! | NodeJoinComplete | `complete_join()` |
   //! | ... | ... |
   ```

**Acceptance Criteria:**
- [ ] All DST tests pass
- [ ] Determinism verified (same seed = same result)
- [ ] Spec marked COMPLETE only when truly complete
- [ ] TLA+ mapping documented

**Estimated Effort:** 0.5 days

---

## Execution Order

```
Phase 1 (DST Tests)     ████████████████████  2-3 days
         ↓
Phase 2 (TLA+ Actions)  ██████████████        2 days
         ↓
Phase 3 (Migration)     ██████████████        2 days
         ↓
Phase 4 (TigerStyle)    ███████               1 day
         ↓
Phase 5 (Verification)  ████                  0.5 days
                        ─────────────────────
                        Total: 7.5-8.5 days
```

## Dependencies

- Phase 2, 3, 4 can run in parallel after Phase 1 completes
- Phase 5 requires all other phases complete

## Quick Decision Log

| Time | Decision | Rationale |
|------|----------|-----------|
| Start | Phase 1 first | Can't verify anything without production code tests |
| Start | Unmark spec as COMPLETE | Prevents false confidence |
| Start | Keep SimClusterState tests temporarily | Reference for expected behavior |

## Success Criteria

- [ ] Zero `SimClusterState` usage in production DST tests
- [ ] All TLA+ actions have corresponding Rust methods
- [ ] `MigrationQueue` implemented and tested
- [ ] TigerStyle: 2+ assertions per function in cluster.rs
- [ ] All DST tests pass with production code
- [ ] Determinism verified

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| FDB abstraction complex | Medium | Start with MockClusterStorage, iterate |
| Migration breaks single-activation | Medium | Add explicit invariant check in test |
| Phase 1 takes longer | High | Time-box to 3 days, simplify if needed |

## References

- [Spec 077](../specs/077-fdbregistry-multinode-cluster.md)
- [TLA+ Spec](../docs/tla/KelpieClusterMembership.tla)
- [Progress 048 - True DST Architecture](./048_20260125_true_dst_simulation_architecture.md)
