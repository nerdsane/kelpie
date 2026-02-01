# Sandboxed Tools Remediation

**Created**: 2026-01-29
**Status**: In Progress
**Branch**: feature/sandboxed-agents

## Goal

Fix all issues identified in code review:
1. Functional gaps (SandboxPool not initialized, dead code)
2. DST violations (direct time access, no fault injection)
3. Missing tests (no integration tests, no DST tests)

## Issues to Fix

### Functional Issues
- [x] 1.1 Wire up SandboxPool in RikaiOS main.rs
- [x] 1.2 Remove dead ToolExecutor (deleted executor.rs)
- [ ] 1.3 Add integration test for tool execution flow

### DST Violations
- [x] 2.1 Add TimeProvider to WasmRuntime
- [x] 2.2 Remove ToolExecutor (was dead code)
- [x] 2.3 Replace Instant::now() in WasmRuntime (3 calls)
- [x] 2.4 Replace Instant::now() in kelpie-tools/registry.rs (1 call)
- [ ] 2.5 Add #[cfg(feature = "dst")] fault injection points
- [ ] 2.6 Add SimHttp for HTTP tool DST testing

### DST Tests
- [ ] 3.1 Test sandbox timeout handling
- [ ] 3.2 Test WASM cache eviction under load
- [ ] 3.3 Test concurrent tool execution
- [ ] 3.4 Test pool exhaustion

### Integration Tests
- [x] 3.5 Add custom tool integration test file
  - Tests tool registration, execution, error handling
  - Most tests require writable filesystem (marked #[ignore])
  - test_unsupported_runtime_error runs in CI

## Findings

### Phase 1: Functional Fixes (Complete)
- Deleted executor.rs (663 lines of dead code)
- Added `set_sandbox_pool()` method to UnifiedToolRegistry (with RwLock for interior mutability)
- RikaiOS main.rs now initializes SandboxPool with proper config:
  - min_size: 2, max_size: 10
  - memory: 512MB, exec_timeout: 30s

### Phase 2: DST Compliance (Partial)
- WasmRuntime now accepts `TimeProvider` in constructor
- CachedModule.last_used changed from `Instant` to `u64` (ms from TimeProvider)
- kelpie-tools/registry.rs uses `time_provider.monotonic_ms()` instead of `Instant::now()`

### Phase 3: Integration Tests (Complete)
- Added custom_tool_integration.rs with 6 tests
- Tests cover Python, JavaScript, Shell execution
- Tests cover sandbox pool usage and concurrent execution
- Sandbox tests require writable filesystem (marked as ignored for CI)

## Decision Log

| Time | Decision | Rationale |
|------|----------|-----------|
| | | |
