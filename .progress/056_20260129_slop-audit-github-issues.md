# Slop Audit - GitHub Issue Creation

**Date:** 2026-01-29
**Status:** In Progress

## Workstream Analysis

Based on 65 issues from the slop audit, organized into 8 workstreams:

### Workstream 1: TLA+ Spec Completion (11 issues, CRITICAL)
Complete truncated TLA+ specifications so they can be model-checked with TLC.

**Issues:**
- KelpieTeleport.tla - Missing TypeOK, SwitchArch truncated
- KelpieClusterMembership.tla - NodeJoinComplete truncated
- KelpieAgentActor.tla - ExecuteIteration truncated
- KelpieSingleActivation.tla - Missing Next/Spec
- KelpieMigration.tla - CrashNode truncated, missing Init/Next/Spec
- KelpieMultiAgentInvocation.tla - InitiateCall truncated
- KelpieLinearizability.tla - InvokeRead truncated
- KelpieWAL.tla - StartRecovery truncated
- KelpieRegistry.tla - Truncated at "Liveness Pr"
- KelpieLease.tla - RenewLeaseSafe truncated
- KelpieActorLifecycle.tla - Missing Spec definition

### Workstream 2: TLA+ INVARIANT Declarations (2 issues, HIGH)
Add INVARIANT declarations so TLC can verify safety properties.

**Issues:**
- KelpieFDBTransaction.tla - Complete but no INVARIANT declarations
- KelpieActorState.tla - Complete but no INVARIANT declarations

### Workstream 3: DST Runtime Migration (9 issues, CRITICAL)
Migrate tests from real tokio runtime to madsim for determinism.

**Issues:**
- real_adapter_http_dst.rs - Uses #[tokio::test]
- memory_tools_dst.rs - Missing simulated time
- llm_token_streaming_dst.rs - Faults not applied
- real_adapter_dst.rs - Placeholder tests
- letta_full_compat_dst.rs - Uses chrono::Utc::now()
- agent_streaming_dst.rs - Uses runtime.spawn()
- mcp_servers_dst.rs - Real tokio runtime
- agent_message_handling_dst.rs - Real async runtime

### Workstream 4: DST Fault Injection Enhancement (15 issues, HIGH)
Add comprehensive fault injection to tests with simulation harness.

**Issues:**
- agent_types_dst.rs
- agent_service_send_message_full_dst.rs
- firecracker_snapshot_metadata_dst.rs
- fdb_storage_dst.rs
- umi_integration_dst.rs
- agent_loop_types_dst.rs
- heartbeat_dst.rs
- tla_bug_patterns_dst.rs
- heartbeat_integration_dst.rs
- agent_actor_dst.rs
- registry_actor_dst.rs (docstring claims fault injection)
- And more...

### Workstream 5: SimStorage FDB Semantics (1 issue, CRITICAL)
Fix SimStorage to properly model FDB transaction semantics.

**Issues:**
- sim.rs - No transaction isolation, no conflict detection

### Workstream 6: Verification Chain Gaps (3 issues, HIGH)
Create missing TLA+ specs for ADRs and vice versa.

**Issues:**
- ADR-010 Heartbeat/Pause - No TLA+ spec
- ADR-029 Federated Peer Architecture - No TLA+ spec
- KelpieLease.tla - No corresponding ADR

### Workstream 7: Spec-Implementation Gaps (1 issue, CRITICAL)
Align DST tests with TLA+ spec invariants.

**Issues:**
- registry_actor_dst.rs - Doesn't verify SingleActivation/PlacementConsistency

### Workstream 8: Code Quality (8 issues, LOW-MEDIUM)
Dead code, duplicate code, readability improvements.

**Issues:**
- state.rs - 10 unused functions
- registry.rs - Duplicate unregister functions
- agent_call.rs - Unused create_nested_context
- messages.rs - Magic numbers, undocumented structs
- DST LLM client divergence
- Duplicate test setup patterns

## GitHub Issues to Create

1. **[EPIC] Complete TLA+ Specifications for Model Checking**
2. **[EPIC] Migrate DST Tests to madsim Runtime**
3. **[EPIC] Enhance DST Fault Injection Coverage**
4. **Fix SimStorage Transaction Semantics**
5. **Add TLA+ Specs for Heartbeat and Federation ADRs**
6. **Align Registry DST with TLA+ Invariants**
7. **Clean Up Dead Code and Improve Readability**

## Progress

- [x] Create epic issues
- [x] Create individual issues linked to epics
- [ ] Update slop database with issue links

## Created Issues

### Epic Issues
| # | Title | Priority | Labels |
|---|-------|----------|--------|
| #85 | [EPIC] Complete TLA+ Specifications for Model Checking | P0 | tla+, verification, epic |
| #86 | [EPIC] Migrate DST Tests to madsim Runtime | P0 | dst, epic |
| #88 | [EPIC] Enhance DST Fault Injection Coverage | P1 | dst, fault-injection, epic |

### Critical Issues (P0)
| # | Title | Workstream |
|---|-------|------------|
| #87 | Fix SimStorage Transaction Semantics to Match FDB | Storage |
| #90 | Align Registry DST Tests with TLA+ Invariants | Spec-Impl Gap |
| #93 | Complete KelpieRegistry.tla specification | TLA+ |
| #94 | Complete KelpieSingleActivation.tla specification | TLA+ |
| #95 | Complete KelpieAgentActor.tla specification | TLA+ |
| #96 | Migrate letta_full_compat_dst.rs to madsim | DST Runtime |
| #97 | Migrate agent_streaming_dst.rs to madsim | DST Runtime |
| #101 | Migrate mcp_servers_dst.rs to madsim | DST Runtime |
| #102 | Migrate agent_message_handling_dst.rs to madsim | DST Runtime |

### High Priority Issues (P1)
| # | Title | Workstream |
|---|-------|------------|
| #89 | Add TLA+ Specs for Heartbeat and Federation ADRs | Verification Chain |
| #91 | Add INVARIANT Declarations to Complete TLA+ Specs | TLA+ |
| #98 | Add fault injection to registry_actor_dst.rs | Fault Injection |
| #99 | Complete KelpieWAL.tla specification | TLA+ |
| #100 | Complete KelpieLease.tla specification | TLA+ |

### Medium Priority Issues (P2)
| # | Title | Workstream |
|---|-------|------------|
| #92 | Clean Up Dead Code and Improve Documentation | Code Quality |

## Summary

**Total Issues Created:** 18
- 3 Epic issues
- 9 Critical (P0) issues
- 5 High (P1) issues
- 1 Medium (P2) issue

**Workstream Breakdown:**
- TLA+ Completion: 8 issues
- DST Runtime Migration: 5 issues
- Fault Injection: 2 issues
- Storage Semantics: 1 issue
- Spec-Impl Alignment: 1 issue
- Code Quality: 1 issue
