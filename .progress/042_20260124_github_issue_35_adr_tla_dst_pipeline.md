# Plan: Strengthen ADR→TLA+→DST Pipeline (GitHub Issue #35)

**Status**: COMPLETE
**GitHub Issue**: https://github.com/rita-aga/kelpie/issues/35
**Created**: 2026-01-24
**Branch**: issue-35-pipeline

## Goal

Complete the remaining items from issue #35 to strengthen the ADR→TLA+→DST verification pipeline.

## Current State Analysis

### Already Done (from previous work):
- ✅ Single activation DST tests exist (`single_activation_dst.rs` - 11 tests)
- ✅ Partition tolerance DST tests exist (`partition_tolerance_dst.rs` - 11 tests including `test_partition_healing_no_split_brain`)
- ✅ TLA+ specs exist (11 specs)
- ✅ TLA README has Spec-to-ADR mapping table
- ✅ Liveness DST tests exist (`liveness_dst.rs`)
- ✅ Cluster membership tests exist (`cluster_dst.rs` - 24 tests)

### Remaining Tasks:
1. **Create docs/VERIFICATION.md** - Document canonical ADR→TLA+→DST pipeline
2. **Update ADR-001** - Add TLA+ spec reference for single activation
3. **Add test_primary_election_convergence** - Cluster membership test from ADR-025
4. **Add test_single_activation_with_network_partition** - Explicit network partition test for single activation
5. **Update TLA+ spec headers** - Ensure each spec cites corresponding ADR

## Options & Decisions

### Decision 1: Where to Document Pipeline

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: docs/VERIFICATION.md | Dedicated file | Clear separation, easy to find | Another file to maintain |
| B: Extend CLAUDE.md | Add to existing guide | Single source of truth | File getting large |
| C: docs/adr/028-verification-pipeline.md | As an ADR | Formal decision record | ADRs are for decisions, not procedures |

**Decision**: Option A - Create `docs/VERIFICATION.md`. The canonical pipeline deserves its own document, and it's what the issue specifically suggests.

**Trade-offs**: One more file to maintain, but clearer organization.

### Decision 2: Test Naming Convention

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| A: Match issue exactly | Use `test_single_activation_with_network_partition` | Direct mapping to issue | May duplicate existing similar tests |
| B: Add alias tests | Create wrapper tests that call existing ones | Satisfies issue, avoids duplication | Indirection |
| C: Document mapping | Show how existing tests cover requirements | No code duplication | May not satisfy literal interpretation |

**Decision**: Option A - Create explicit tests with exact names from issue. The existing tests have different semantics (e.g., `test_concurrent_activation_with_network_delay` uses delay, not partition). New tests will use actual partition injection.

**Trade-offs**: Some overlap with existing tests, but clearer mapping to TLA+ spec invariants.

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-24 | Create docs/VERIFICATION.md | Issue specifies this location | Extra file |
| 2026-01-24 | Explicit test names matching issue | Clear mapping to requirements | Some overlap |
| 2026-01-24 | Add Formal Specification section to ADR-001 | Follow pattern from ADR-004 | None |

## Implementation Phases

### Phase 1: Create docs/VERIFICATION.md [COMPLETE]
- [x] Document ADR→TLA+→DST pipeline
- [x] Add template for new features
- [x] Add checklist for verification

### Phase 2: Update ADR-001 [COMPLETE]
- [x] Add Formal Specification section
- [x] Reference KelpieSingleActivation.tla
- [x] Add safety invariants list

### Phase 3: Add Missing DST Tests [COMPLETE]
- [x] `test_single_activation_with_network_partition`
- [x] `test_single_activation_with_crash_recovery`
- [x] `test_primary_election_convergence`

### Phase 4: Update TLA+ Specs Headers [COMPLETE]
- [x] Audit each .tla file for ADR citation
- [x] Add missing ADR references to KelpieSingleActivation.tla

### Phase 5: Verification [COMPLETE]
- [x] Run `cargo test -p kelpie-dst` - All relevant tests pass
- [x] Run `cargo clippy` - No warnings
- [x] Commit and push (commit 80140b6b)

## What to Try

### Works Now
| What | How to Try | Expected Result |
|------|------------|-----------------|
| Pipeline documentation | Read `docs/VERIFICATION.md` | Full ADR→TLA+→DST pipeline |
| ADR-001 Formal Spec section | Read `docs/adr/001-virtual-actor-model.md` | Has TLA+ references |
| Network partition test | `cargo test -p kelpie-dst test_single_activation_with_network_partition` | Pass |
| Crash recovery test | `cargo test -p kelpie-dst test_single_activation_with_crash_recovery` | Pass |
| Primary election test | `cargo test -p kelpie-dst test_primary_election_convergence` | Pass |
| All single activation tests | `cargo test -p kelpie-dst --test single_activation_dst` | 11 pass, 2 ignored |
| All cluster tests | `cargo test -p kelpie-dst --test cluster_dst` | 21 pass, 2 ignored |
| All partition tests | `cargo test -p kelpie-dst --test partition_tolerance_dst` | 9 pass, 1 ignored |

### Doesn't Work Yet
| What | Why | When Expected |
|------|-----|---------------|
| N/A - All tasks complete | - | - |

### Known Limitations
- Tests use simulated cluster, not production FDB
- Primary election test uses SimCluster quorum logic
- Pre-existing failing test in deterministic_scheduling_dst.rs (unrelated to this issue)
