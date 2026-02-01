# Plan 034: Write-Ahead Log (WAL) for Atomicity

## Problem Statement

The `create_agent()` and other service operations return success to the client before the transaction is durably persisted. When a crash happens during commit, data is lost but the client thinks the operation succeeded.

### Bugs Fixed

**BUG-001: Create Returns Ok But Get Fails (CRITICAL)**
- `create_agent()` returns `Ok(agent)`
- But `get_agent(agent.id)` fails afterward
- Violates linearizability - if create succeeded, the data should be readable

**BUG-002: Partial Initialization During Crash (CRITICAL)**
- AppState can be created in a partially initialized state
- The object exists but the underlying service doesn't work properly
- Operations succeed at actor layer but crash during commit

**Root Cause:** Both bugs have the same underlying cause - operations succeed in the actor layer but crash during the transaction commit, leaving data unpersisted.

## Solution: Write-Ahead Log Pattern

Record intent before execution, replay on recovery.

```
┌─────────────────────────────────────────────────────────────┐
│ BEFORE (Broken):                                            │
│   invoke() → actor sets state → commit() → return success   │
│                                    ↑                        │
│                              CRASH HERE = DATA LOSS         │
├─────────────────────────────────────────────────────────────┤
│ AFTER (WAL):                                                │
│   WAL.append(intent) → invoke() → commit() → WAL.complete() │
│         ↑                                          ↑        │
│   CRASH HERE = REPLAY ON RECOVERY                  │        │
│                              CRASH HERE = REPLAY ON RECOVERY│
└─────────────────────────────────────────────────────────────┘
```

## Design

### WAL Entry Structure

```rust
pub struct WalEntry {
    pub id: u64,                    // Monotonic ID
    pub operation: WalOperation,    // What to do
    pub actor_id: ActorId,          // Target actor
    pub payload: Bytes,             // Serialized request
    pub status: WalStatus,          // Pending/Complete/Failed
    pub created_at: u64,            // Timestamp (ms)
    pub completed_at: Option<u64>,  // When completed
}

pub enum WalOperation {
    CreateAgent,
    UpdateAgent,
    SendMessage,
    DeleteAgent,
    // ... other operations
}

pub enum WalStatus {
    Pending,
    Complete,
    Failed { error: String },
}
```

### WAL Trait

```rust
#[async_trait]
pub trait WriteAheadLog: Send + Sync {
    /// Durably append entry, returns entry ID
    async fn append(&self, op: WalOperation, actor_id: &ActorId, payload: Bytes) -> Result<u64>;

    /// Mark entry as successfully completed
    async fn complete(&self, entry_id: u64) -> Result<()>;

    /// Mark entry as failed (won't be replayed)
    async fn fail(&self, entry_id: u64, error: &str) -> Result<()>;

    /// Get all pending entries for replay
    async fn pending_entries(&self) -> Result<Vec<WalEntry>>;

    /// Cleanup old completed entries (retention policy)
    async fn cleanup(&self, older_than_ms: u64) -> Result<u64>;
}
```

### Implementation Options

| Backend | Durability | Performance | Complexity |
|---------|------------|-------------|------------|
| File-based | fsync per entry | Good | Low |
| KV-backed | Depends on KV | Varies | Low |
| Memory (DST) | Simulated | Fast | Low |

## Phases

### Phase 1: WAL Core (kelpie-storage) ✅ COMPLETE
- [x] 1.1 Define `WalEntry`, `WalOperation`, `WalStatus` types
- [x] 1.2 Define `WriteAheadLog` trait
- [x] 1.3 Implement `MemoryWal` for testing/DST

### Phase 2: KV-Backed WAL (kelpie-storage) ✅ COMPLETE
- [x] 2.1 Implement `KvWal` using existing KV trait
- [x] 2.2 Add atomic counter for entry IDs (uses transaction for atomicity)
- [x] 2.3 Add cleanup/compaction logic

**Files created:**
- `crates/kelpie-storage/src/wal.rs` - WAL types, trait, and implementations
- Updated `crates/kelpie-storage/src/lib.rs` - exports WAL module
- Updated `crates/kelpie-storage/Cargo.toml` - added serde/serde_json deps

**Tests:** 6 new tests (all passing)
- `test_memory_wal_append_and_complete`
- `test_memory_wal_append_and_fail`
- `test_memory_wal_pending_entries_ordered`
- `test_memory_wal_cleanup`
- `test_kv_wal_basic`
- `test_pending_count`

### Phase 3: Service Integration (kelpie-server) ✅ COMPLETE
- [x] 3.1 Add WAL to `AgentService` (with IoContext for timestamps)
- [x] 3.2 Wrap `create_agent` with WAL
- [x] 3.3 Wrap `update_agent` with WAL
- [x] 3.4 Wrap `send_message` with WAL
- [x] 3.5 Wrap `delete_agent` with WAL
- [x] 3.6 Wrap `update_block_by_label` with WAL

**Implementation notes:**
- Added `new_without_wal()` constructor for testing (uses MemoryWal)
- Production constructor requires explicit WAL and IoContext
- All mutation operations follow: append → execute → complete/fail pattern

### Phase 4: Recovery (kelpie-server) ✅ COMPLETE
- [x] 4.1 Add `recover()` method to replay pending entries
- [x] 4.2 Implemented idempotency checks:
  - CreateAgent: Check if agent exists before replay
  - DeleteAgent: Check if agent already deleted
  - UpdateAgent/SendMessage/UpdateBlock: Replay (idempotent)
- [x] 4.3 Call `recover()` on service startup (main.rs)

**Implementation notes:**
- Added `recover_wal()` method to AppState that delegates to AgentService
- Called during server startup after custom tools loaded, before loading agents
- Logs count of recovered entries

### Phase 5: DST Tests ✅ COMPLETE
- [x] 5.1 CrashDuringTransaction fault works with WAL (recovery handles it)
- [ ] 5.2 Add WAL-specific faults (CrashDuringWalAppend, CrashDuringWalComplete) - Future enhancement
- [x] 5.3 Verify `test_deactivate_during_create_crash` passes
- [x] 5.4 Verify `appstate_integration_dst` tests pass (5 tests)

**Test fixes:**
- `test_deactivate_during_create_crash` now passes (was ignored)
- `test_first_invoke_after_creation` (BUG-001) now passes
- `test_appstate_init_crash` (BUG-002) now passes
- `test_concurrent_agent_creation_race` passes
- `test_shutdown_with_inflight_requests` passes
- All tests call `service.recover()` to simulate server restart
- Tests retry operations after recovery (crash can still affect reads)

### Phase 6: Cleanup
- [ ] 6.1 Add background cleanup task
- [ ] 6.2 Add metrics for WAL size/latency
- [ ] 6.3 Documentation

## Options & Decisions

### Decision 1: WAL Storage Location

| Option | Pros | Cons |
|--------|------|------|
| A. Separate file | Simple, fast fsync | Another storage system |
| B. Same KV as actors | Unified, simple | KV must support fsync |
| C. Dedicated WAL KV | Optimized for append | More complexity |

**Decision:** Option B - Same KV as actors
**Rationale:**
- Kelpie already has a KV abstraction
- FoundationDB guarantees durability on commit
- MemoryKV can be enhanced for DST
- Simpler architecture

### Decision 2: Entry ID Generation

| Option | Pros | Cons |
|--------|------|------|
| A. UUID | Unique, no coordination | Large, no ordering |
| B. Monotonic counter | Small, ordered | Need atomic increment |
| C. Timestamp + random | Unique, rough order | Clock skew issues |

**Decision:** Option B - Monotonic counter stored in KV
**Rationale:**
- Ordered IDs make cleanup easier
- Atomic increment is simple with transactions
- Matches TigerStyle (explicit, predictable)

### Decision 3: Idempotency Strategy

| Option | Pros | Cons |
|--------|------|------|
| A. Check before replay | Simple | Race conditions |
| B. Idempotency keys | Robust | More storage |
| C. Actor ID as key | Natural for creates | Doesn't work for all ops |

**Decision:** Option C for creates, Option B for others
**Rationale:**
- Creates: Check if actor exists before replay
- Updates/Messages: Use WAL entry ID as idempotency key in actor state
- SendMessage: Use message_id as idempotency key (generated UUID or client-provided)

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| Start | WAL in kelpie-storage | Reuse KV abstraction | Coupled to storage layer |
| Start | Entry ID = monotonic | Ordered, simple cleanup | Need atomic counter |
| Phase 5.7 | Add idempotency_key to WalEntry | Prevent duplicate WAL entries from client retries | More storage per entry |
| Phase 5.7 | Retry counter increment 5x | Handle transaction conflicts | Slightly slower on conflict |
| Phase 5.7 | Cleanup at startup | Automatic old entry removal | Adds startup time |

## What to Try

### Works Now (After Phase 1-5.7)
- `MemoryWal` can append, complete, fail, and cleanup entries
- `KvWal` works with any `ActorKV` backend (MemoryKV tested)
- `cargo test -p kelpie-storage` passes (19 tests + 8 FDB ignored)
- All AgentService mutations wrapped with WAL
- Recovery method implemented with idempotency checks
- `cargo test -p kelpie-server --lib` passes (181 tests)
- WAL recovery called on server startup in main.rs
- WAL cleanup called on server startup (removes entries older than 24h)
- All DST tests pass (7 tests across 2 test files)
- Idempotency keys supported for SendMessage to prevent duplicate WAL entries
- KvWal has retry logic (5x) for counter increment to handle transaction conflicts

### Bugs Fixed
- **BUG-001**: Create returns Ok but Get fails → Fixed via WAL recovery
- **BUG-002**: Partial initialization during crash → Fixed via WAL recovery
- **Issue #2**: No automatic WAL cleanup → Fixed, cleanup at startup (24h retention)
- **Issue #3**: KvWal::next_id() transaction conflicts → Fixed, retry logic added
- **Issue #4**: SendMessage duplicate WAL entries → Fixed, idempotency_key support added

### Known Limitations
- SendMessage recovery may still cause duplicate message processing if crash happens
  after message is processed but before WAL.complete(). This requires agent-side
  idempotency tracking (checking message_id in agent state) for full deduplication.
  The idempotency_key prevents duplicate WAL entries (client retries), not duplicate
  message processing during crash recovery.

### Next Steps (Future Enhancement)
- Add WAL-specific DST faults (CrashDuringWalAppend, CrashDuringWalComplete)
- Add agent-side message_id tracking for true SendMessage idempotency
- Add metrics for WAL size/latency
- Add background cleanup task (periodic, not just at startup)

## Instance Log

| Instance | Phase | Status | Notes |
|----------|-------|--------|-------|
| Claude-1 | 1-5 | Complete | Core WAL + recovery + all DST tests pass |
| Claude-2 | 5.7 | Complete | Idempotency keys, retry logic, cleanup |

## Verification Checklist

- [x] `cargo test -p kelpie-storage` passes (19/27 - 8 FDB ignored)
- [x] `cargo test -p kelpie-server --lib` passes (181 tests)
- [x] `cargo test -p kelpie-server` passes (all integration tests)
- [x] `cargo clippy -p kelpie-storage -p kelpie-server` passes (no errors)
- [x] `test_deactivate_during_create_crash` passes (BUG-001 fixed)
- [x] `appstate_integration_dst` tests pass (5 tests with --features dst)
- [x] WAL idempotency tests pass (4 new tests for MemoryWal and KvWal)
- [ ] Manual test: kill server during create, restart, agent exists
