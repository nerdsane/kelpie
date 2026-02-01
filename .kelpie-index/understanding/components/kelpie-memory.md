# kelpie-memory

**Examined:** 2026-01-22T21:37:41.524641+00:00

## Summary

Hierarchical memory system with Core/Working/Archival tiers for LLM context management

## Connections

- kelpie-core
- kelpie-server
- kelpie-agent

## Details

9 files, 96KB. Core memory (fixed capacity, always-loaded), Working memory (session-scoped), Archival (minimally implemented). Checkpoint-based persistence to KV storage. No concurrency protection.

## Issues

### [HIGH] Unbounded metadata/tag/checkpoint growth with no cleanup

**Evidence:** core.rs, checkpoint.rs

### [MEDIUM] Race condition in get_first_by_type_mut() - TOCTOU vulnerability

**Evidence:** core.rs

### [MEDIUM] No thread safety - assumes single-threaded use

**Evidence:** core.rs

### [MEDIUM] 2x memory spike during checkpoint serialization

**Evidence:** checkpoint.rs

### [LOW] No checkpoint validation or integrity checks

**Evidence:** checkpoint.rs
