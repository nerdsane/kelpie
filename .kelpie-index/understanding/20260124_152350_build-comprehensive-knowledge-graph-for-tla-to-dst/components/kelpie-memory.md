# kelpie-memory

**Examined:** 2026-01-24T15:23:23.827309+00:00

## Summary

Three-tier memory system: Core (~32KB), Working (~1MB), Archival (embeddings) with checkpoint/restore

## Connections

- kelpie-server
- kelpie-storage
- kelpie-core

## Details

**Memory Tiers:**
- Core Memory: Fixed capacity, explicit blocks with ordering
- Working Memory: KV with TTL, capacity-bounded
- Archival: Vector embeddings for semantic search (partial)

**Invariants:**
- Core: current_bytes ≤ max_bytes
- Working: current_bytes ≤ max_bytes
- Block: size_bytes ≤ 16KB
- block_order.len() == blocks.len()
- Checkpoint sequence strictly increasing

**Checkpoint:**
- Serialization snapshot (not WAL)
- No atomic checkpoint with state mutation

## Issues

### [HIGH] No thread safety - CoreMemory/WorkingMemory are Clone but not Arc<Mutex>

**Evidence:** Multiple concurrent add_block() calls would race

### [MEDIUM] Checkpoint not atomic with state mutations - crash during checkpoint loses state

**Evidence:** No WAL visible in checkpoint.rs

### [MEDIUM] Expired entries still count toward capacity until pruned

**Evidence:** working.rs expired entries remain in current_bytes
