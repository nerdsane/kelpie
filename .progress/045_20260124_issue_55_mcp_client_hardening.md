# Issue #55: MCP Client Production Hardening

**Date:** 2026-01-24
**Branch:** issue-55-mcp-hardening
**Status:** Complete

## Summary

Production hardening for the MCP client (`crates/kelpie-tools/src/mcp.rs`). Adding:
- SSE transport graceful shutdown
- Automatic reconnection with exponential backoff
- Health checks
- Pagination support
- Robust response parsing

## Implementation Plan

### Phase 1: SSE Transport Graceful Shutdown ✅
**Priority:** High

- [x] Fix `SseTransport::close()` to actually signal listener to stop
- [x] Add proper shutdown handling in SSE listener task
- [x] Store listener task handle for join-with-timeout
- [x] Add test: `test_sse_transport_graceful_shutdown`

### Phase 2: Automatic Reconnection ✅
**Priority:** High

- [x] Add `ReconnectConfig` struct with backoff settings
- [x] Add `McpClient::reconnect()` with exponential backoff
- [x] Add `with_reconnect_config()` builder method
- [x] Update registry's `execute_mcp()` to use reconnect
- [x] Add test: `test_reconnection_with_backoff`
- [x] Add test: `test_reconnection_max_attempts_exceeded`

### Phase 3: Health Checks ✅
**Priority:** Medium

- [x] Add `McpClient::health_check()` method
- [x] Add optional `start_health_monitor()` for background monitoring
- [x] Add test: `test_health_check`

### Phase 4: Pagination Support ✅
**Priority:** Medium

- [x] Modify `discover_tools()` to handle `next_cursor`
- [x] Handle servers that don't support pagination
- [x] Add test: `test_tool_discovery_pagination`

### Phase 5: Robust Response Parsing ✅
**Priority:** Medium

- [x] Add `extract_tool_output()` helper function
- [x] Handle all MCP content types (text, image, resource)
- [x] Handle empty responses meaningfully
- [x] Add test: `test_response_parsing_edge_cases`

## Files Modified

| File | Changes |
|------|---------|
| `crates/kelpie-tools/src/mcp.rs` | All phases |
| `crates/kelpie-tools/src/lib.rs` | Export new types |
| `crates/kelpie-server/src/tools/registry.rs` | Phase 2 integration |

## Verification

- [x] `cargo test -p kelpie-tools` passes (67 tests)
- [x] `cargo clippy -p kelpie-tools -p kelpie-server` has no warnings
- [x] `cargo fmt` applied

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 19:35 | Use JoinHandle + abort for SSE shutdown | Clean task termination | Need to store handle |
| 19:35 | Default 5 reconnect attempts | Balance reliability vs latency | May delay failure detection |
| 19:35 | Use tools/list for health check | MCP has no ping method | Slightly heavier than ping |
