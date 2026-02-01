# kelpie-server

**Examined:** 2026-01-24T15:16:36.429129+00:00

## Summary

Main server binary with actor-based AppState, AgentService, WAL-backed transactions, and comprehensive DST test coverage

## Connections

- kelpie-runtime
- kelpie-storage
- kelpie-registry
- kelpie-dst

## Details

**State Machines:**
- AppState lifecycle: Uninitialized → Initialized → ShuttingDown → Shutdown
- AgentService request lifecycle: Pending → Processing → Completed/Failed/CrashedDuringTransaction
- WAL recovery enables Crashed → Completed transition

**Invariants Found:**
1. AppState initialization must be all-or-nothing (no partial state)
2. No duplicate agents from concurrent requests with same name
3. All created agents must be retrievable immediately after creation (or via WAL recovery)
4. In-flight requests during shutdown must complete OR fail - never silently drop
5. New requests after shutdown must be rejected with shutdown error
6. Agent data must not be corrupted after retrieval (name, system, tool_ids match)
7. Search must never return results from other agents (BUG-002 pattern)

**Concurrency:**
- Arc<AppState<R>> shared across concurrent tasks
- Arc<FaultInjector> for deterministic fault injection
- Arc<UnifiedToolRegistry> for tool execution
- RwLock<Vec<String>> for execution logging

**TOCTOU Risks:**
- Concurrent creation race: between name existence check and write
- BUG-001 pattern: between create return and get call (mitigated by WAL)
- Shutdown race: between shutdown initiation and rejection taking effect

## Issues

### [MEDIUM] Shutdown race between initiation and rejection needs atomic state transition

**Evidence:** test_shutdown_with_inflight_requests tests this but fix unclear

### [LOW] BUG-001/BUG-002 patterns documented but should be verified with DST

**Evidence:** Tests exist but TLA+ invariants not formally defined
