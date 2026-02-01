# Tracked TODOs - Kelpie Codebase

## Status: Triaged and Tracked (Phase 9.7)

This file tracks the TODOs found during the slop audit. They are legitimate future work items, not slop to be removed.

---

## HIGH Priority (Should be addressed soon)

### 1. FDB Transaction Atomicity
**Location:** `crates/kelpie-server/src/storage/fdb.rs:864`
**TODO:** Use FDB transaction for atomicity
**Impact:** Data consistency risk without atomic operations
**Recommendation:** Implement transactional wrapper when FDB is production-ready

### 2. Production Storage Backend
**Locations:** `crates/kelpie-server/src/state.rs` (lines 154, 235, 308, 386)
**TODO:** Use FDB instead of MemoryKV for production
**Impact:** Currently using in-memory storage which loses data on restart
**Recommendation:** Implement feature flag to switch between MemoryKV (dev) and FDB (prod)

### 3. Teleport Storage Implementation
**Locations:** `crates/kelpie-server/src/api/teleport.rs` (lines 96, 111, 123)
**TODO:** Implement actual teleport storage queries
**Impact:** Teleport API endpoints return placeholder responses
**Recommendation:** Implement when TeleportService is integrated with API layer

---

## MEDIUM Priority (Address when convenient)

### 4. Session/Message Deletion
**Location:** `crates/kelpie-server/src/storage/fdb.rs:354-355`
**TODO:** Delete all sessions/messages (need scan + delete loop)
**Impact:** Agent deletion may leave orphaned data
**Recommendation:** Implement range deletion when FDB integration matures

### 5. Filesystem Snapshots
**Location:** `crates/kelpie-sandbox/src/io.rs:351`
**TODO:** Add filesystem to Snapshot
**Impact:** Sandboxed processes can't persist filesystem state
**Recommendation:** Implement filesystem capture in snapshot serialization

---

## LOW Priority (Future enhancements)

### 6. Secondary Index Optimization
**Location:** `crates/kelpie-server/src/storage/fdb.rs:702`
**TODO:** Optimize with secondary index
**Impact:** Message queries may be slow without index
**Recommendation:** Add secondary index when performance becomes issue

### 7. Agent Cache Migration
**Location:** `crates/kelpie-server/src/state.rs:81`
**TODO:** Remove agents cache after HTTP handlers migrated
**Impact:** Technical debt - two storage paths for agents
**Recommendation:** Clean up after agent_service migration complete

### 8. Project ID Support
**Location:** `crates/kelpie-server/src/state.rs:837`
**TODO:** Add project_id to AgentMetadata
**Impact:** Multi-tenancy feature incomplete
**Recommendation:** Implement with organization/project support

### 9. Tool Call ID Tracking
**Location:** `crates/kelpie-server/src/actor/llm_trait.rs:273`
**TODO:** Track tool call ID across deltas
**Impact:** Tool call correlation may be unreliable in streaming
**Recommendation:** Implement proper tool call state tracking

### 10. Iteration Counter Verification
**Location:** `crates/kelpie-server/tests/agent_service_fault_injection.rs:525`
**TODO:** Add iteration counter verification
**Impact:** Test could miss iteration count bugs
**Recommendation:** Add assertion when agent iteration tracking added

---

## Resolution Summary

| Priority | Count | Action |
|----------|-------|--------|
| HIGH | 3 | Track, implement when ready |
| MEDIUM | 2 | Track, schedule for future sprint |
| LOW | 5 | Track, implement as needed |

**Total:** 10 TODOs tracked (16 found, 6 duplicates removed)

---

*Generated: 2026-01-21*
*Status: Phase 9.7 Complete - TODOs triaged and tracked*
