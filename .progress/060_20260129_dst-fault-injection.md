# DST Fault Injection for WASM Runtime and Custom Tools

**Status**: ✅ Complete
**Date**: 2026-01-29
**Commit**: bd355e9a

## Summary

Added full DST (Deterministic Simulation Testing) fault injection support for WASM runtime and custom tool execution, following the established FaultInjector pattern.

## Changes Made

### 1. New Fault Types (kelpie-dst/src/fault.rs)

Added 8 new fault types:

**WASM Runtime Faults:**
- `WasmCompileFail` - WASM module compilation fails
- `WasmInstantiateFail` - WASM module instantiation fails
- `WasmExecFail` - WASM execution fails
- `WasmExecTimeout { timeout_ms }` - WASM execution times out (fuel exhausted)
- `WasmCacheEvict` - Force cache eviction for testing cache behavior

**Custom Tool Faults:**
- `CustomToolExecFail` - Custom tool execution fails
- `CustomToolExecTimeout { timeout_ms }` - Custom tool execution times out
- `CustomToolSandboxAcquireFail` - Sandbox acquisition fails (pool exhausted)

### 2. FaultInjectorBuilder Methods

Added two new builder methods:
- `with_wasm_faults(probability)` - Adds all 5 WASM faults
- `with_custom_tool_faults(probability)` - Adds all 3 custom tool faults

### 3. WASM Runtime DST Support (kelpie-wasm/src/runtime.rs)

- Added `dst` feature to kelpie-wasm/Cargo.toml
- Added optional `FaultInjector` field (behind `#[cfg(feature = "dst")]`)
- Added `with_fault_injection()` constructor
- Added `check_fault()` method for DST mode
- Inject faults at:
  - `wasm_cache_lookup` - Before cache lookup (WasmCacheEvict)
  - `wasm_compile` - Before compilation (WasmCompileFail)
  - `wasm_execute` - Before execution (WasmExecFail, WasmExecTimeout, WasmInstantiateFail)

### 4. UnifiedToolRegistry DST Support (kelpie-server/src/tools/registry.rs)

- Added optional `FaultInjector` field (behind `#[cfg(feature = "dst")]`)
- Added `set_fault_injector()` method
- Added `check_fault()` method for DST mode
- Inject faults at `custom_tool_execute` before sandbox acquisition

### 5. DST Tests (kelpie-dst/tests/wasm_custom_tool_dst.rs)

Created 13 new tests:
- `test_wasm_fault_type_names` - Verify fault type names
- `test_custom_tool_fault_type_names` - Verify fault type names
- `test_fault_injector_builder_wasm_faults` - Verify builder adds 5 faults
- `test_fault_injector_builder_custom_tool_faults` - Verify builder adds 3 faults
- `test_wasm_fault_injection_determinism` - Same seed produces same results
- `test_custom_tool_fault_injection_determinism` - Same seed produces same results
- `test_wasm_fault_with_operation_filter` - Operation filters work correctly
- `test_custom_tool_fault_with_max_triggers` - max_triggers limit works
- `test_combined_wasm_and_custom_tool_faults` - Both fault sets combine correctly
- `test_fault_injection_stats_tracking` - Stats are tracked correctly
- `test_dst_wasm_fault_injection_in_simulation` - WASM faults in simulation context
- `test_dst_custom_tool_fault_injection_in_simulation` - Custom tool faults in simulation context
- `test_dst_high_load_fault_injection` - Stress test with 2000 operations

## Verification

- All 13 new DST tests pass
- Full kelpie-dst test suite passes
- kelpie-wasm and kelpie-server compile with `dst` feature
- Clippy passes with no warnings
- Code is formatted

## Phase 2: SimHttpClient Implementation (DONE)

### HTTP Fault Types Added (kelpie-dst/src/fault.rs)

Added 5 HTTP-related fault types:
- `HttpConnectionFail` - HTTP connection fails
- `HttpTimeout { timeout_ms }` - HTTP request times out
- `HttpServerError { status }` - Server returns error status (5xx)
- `HttpResponseTooLarge { max_bytes }` - Response body exceeds limit
- `HttpRateLimited { retry_after_ms }` - Server returns 429 rate limit

Added builder method:
- `with_http_faults(probability)` - Adds all 5 HTTP faults

### HttpClient Trait Refactoring

To avoid cyclic dependency between kelpie-dst and kelpie-tools:
- Moved `HttpClient` trait and types to `kelpie-core/src/http.rs`
- `kelpie-tools/src/http_client.rs` re-exports from kelpie-core and adds `ReqwestHttpClient`
- `kelpie-dst/src/http.rs` imports from kelpie-core

### SimHttpClient (kelpie-dst/src/http.rs)

Created simulated HTTP client for DST with:
- Fault injection at request time
- Configurable mock responses by URL pattern (prefix match)
- Request recording for verification
- Deterministic behavior with seeded RNG

API:
- `new(rng, fault_injector)` - Create client with DST components
- `mock_url(pattern, response)` - Set mock response for URL pattern
- `set_default_response(response)` - Set fallback response
- `get_requests()` - Get recorded requests for verification
- `request_count()` - Get total request count
- `clear_requests()` - Clear recorded requests

### SimHttp Tests (8 tests)

- `test_sim_http_basic_request` - Basic GET returns default response
- `test_sim_http_mock_response` - URL pattern matching works
- `test_sim_http_recorded_requests` - Requests are recorded
- `test_sim_http_with_connection_fault` - Connection failures
- `test_sim_http_with_timeout_fault` - Timeout simulation
- `test_sim_http_with_server_error_fault` - Server error injection
- `test_sim_http_determinism` - Same seed = same results
- `test_sim_http_rate_limited` - Rate limiting simulation

## Verification

- All 21 DST tests pass (13 WASM/custom + 8 HTTP)
- Clippy passes with no warnings
- No cyclic dependencies
- Code is formatted

## Summary

DST infrastructure is now complete for:
- ✅ WASM runtime fault injection
- ✅ Custom tool fault injection
- ✅ HTTP client fault injection (SimHttpClient)
