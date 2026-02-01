# docs

**Examined:** 2026-01-24T23:24:11.238916+00:00

## Summary

High-level planning docs including VISION.md (status tracker), PLAN.md (duplicate), VERIFICATION.md (pipeline spec), and MAP.md

## Connections

- docs/adr
- kelpie-dst

## Details

VISION.md contains the definitive status tracker with phases 0-7:
- Phase 0 Bootstrap: COMPLETE
- Phase 1 Actor Runtime: COMPLETE  
- Phase 2 Memory Hierarchy: COMPLETE (gap: no vector search)
- Phase 3 Sandbox: PARTIAL (ProcessSandbox yes, Firecracker no)
- Phase 4 Tools: PARTIAL (Tool trait yes, MCP client stub)
- Phase 5 Cluster: PARTIAL (protocols yes, TCP transport no)
- Phase 6 Adapters: PARTIAL (REST API yes, Letta backend no)
- Phase 7 Production: NOT STARTED

VERIFICATION.md defines ADR→TLA+→DST→Code pipeline with current coverage table.

## Issues

### [HIGH] VISION.md claims performance targets (1M agents, <1ms invocation) that are unverified

**Evidence:** VISION.md line 259-266: All metrics marked 'Unverified'

### [MEDIUM] Duplicate content between VISION.md and docs/PLAN.md - maintenance burden

**Evidence:** Both files contain identical phase status tracking
