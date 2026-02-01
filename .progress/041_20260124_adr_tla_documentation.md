# Plan: Create Missing ADRs for TLA+ Coverage

**Status**: Complete
**GitHub Issue**: https://github.com/rita-aga/kelpie/issues/11
**Created**: 2026-01-24

## Goal

Create 6 new ADRs and update 1 existing ADR to document the aspirational design for components that have TLA+ specs but lack ADR documentation.

## Phases

### Phase 1: Create New ADRs [Complete]
- [x] ADR-022: WAL Design (references KelpieWAL.tla)
- [x] ADR-023: Actor Registry Design (references KelpieRegistry.tla)
- [x] ADR-024: Actor Migration Protocol (references KelpieMigration.tla)
- [x] ADR-025: Cluster Membership Protocol (references KelpieClusterMembership.tla)
- [x] ADR-026: MCP Tool Integration (no TLA+ spec)
- [x] ADR-027: Sandbox Execution Design (no TLA+ spec)

### Phase 2: Update Existing ADR [Complete]
- [x] Update ADR-004 with Formal Specification section
- [x] Add split-brain prevention details
- [x] Add TLA+ cross-references

### Phase 3: Update TLA README [Complete]
- [x] Add cross-references to new ADRs

### Phase 4: Verification [Complete]
- [x] Run verification script from issue - ALL PASS
- [x] Create PR to rita-aga/kelpie

## Verification Results

```
=== ADR-022 === Has Formal Spec section, Has TLA+ reference
=== ADR-023 === Has Formal Spec section, Has TLA+ reference
=== ADR-024 === Has Formal Spec section, Has TLA+ reference
=== ADR-025 === Has Formal Spec section, Has TLA+ reference
=== ADR-026 === No Formal Spec section (N/A - no TLA+ spec)
=== ADR-027 === No Formal Spec section (N/A - no TLA+ spec)
=== ADR-004 === Has Split-Brain Prevention section, Has KelpieClusterMembership.tla reference
```

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 14:30 | Use enhanced template with Formal Spec section | Issue requirement | More detailed ADRs |
| 14:30 | Include model checking results from TLA README | Provides verification evidence | None |
| 14:35 | ADR-026/027 without Formal Spec | No TLA+ specs for these | Correct per issue |

## What to Try

**Works Now**: All ADRs created and cross-referenced
**Doesn't Work Yet**: N/A
**Known Limitations**: None
