# Tool Execution + Sandbox Integration (MVP)

**Created**: 2026-01-29
**Status**: Complete
**Branch**: feature/sandboxed-agents

## Goal

Make approved proposals actually execute tools, with sandboxed execution for untrusted code.

## Phases

### Phase 0: Setup ✅
- [x] Create feature branch in kelpie
- [x] Working directly in kelpie main repo

### Phase 1: Tool Execution Foundation (Kelpie) ✅
- [x] 1.1 HTTP Tool Definitions (`kelpie-tools/src/http_tool.rs`)
  - Created HttpToolDefinition with URL templates and JSONPath extraction
  - HttpTool implements Tool trait
  - Supports GET, POST, PUT, PATCH, DELETE
- [x] 1.2 WASM Runtime (`kelpie-wasm/src/runtime.rs`)
  - Real wasmtime integration (replacing stub)
  - Module caching with LRU eviction
  - Fuel-based execution limits
  - WASI support via stdin/stdout
- [x] 1.3 Custom Tool Executor (`kelpie-server/src/tools/executor.rs`)
  - Unified executor for Python, JavaScript, Shell, WASM
  - Sandbox pool integration
  - Language-specific wrapper scripts
  - Execution statistics

### Phase 2: Proposal Execution (RikaiOS) ✅
- [x] 2.1 Connect apply_proposal() to Tool Registry
- [x] 2.2 Pass Dependencies to apply_proposal()
  - Updated BotState to hold app_state
  - apply_proposal now registers tools via tool_registry.register_custom_tool()
- [ ] 2.3 Persist Proposals to FDB (deferred - MVP uses in-memory)

### Phase 3: Tool-Level Sandbox Integration (Kelpie) ✅
- [x] 3.1 Architecture (tool-level sandboxing)
  - Custom tools run in ProcessSandbox (already existed)
  - Added optional sandbox pool for performance
- [x] 3.2 Tool-Level Sandbox Integration in registry
  - Added with_sandbox_pool() method to UnifiedToolRegistry
  - Enhanced execute_custom() to support Python, JavaScript, and Shell
  - Pool sandboxes are reused; one-off sandboxes created when no pool
- [x] 3.3 Initialize Sandbox Pool in RikaiOS
  - Pool initialization is optional - works without pool too

### Phase 4: VM Backend Selection (Post-MVP)
- [ ] 4.1 VM Factory Configuration (deferred)

## Current Task

All MVP tasks complete!

## Findings

**Phase 1:**
- wasmtime-wasi v16 has different API from newer versions (no preview1 module)
- Used wasi-cap-std-sync + wasi-common for WASI context building
- ProcessSandbox already in registry.rs but executor.rs provides unified interface

**Phase 2:**
- BotState needed app_state for tool_registry access
- apply_proposal() changed from sync to async
- Tools now actually register on approval (not just logged)

**Phase 3:**
- Registry already had sandbox support for Python
- Extended to support JavaScript and Shell
- Added optional sandbox pool for performance optimization

## Summary of Changes

### Kelpie (kelpie repo)

| File | Change |
|------|--------|
| `kelpie-tools/src/http_tool.rs` | NEW - HTTP tool definitions with URL templates |
| `kelpie-tools/src/lib.rs` | Export http_tool module |
| `kelpie-wasm/src/lib.rs` | Replaced stub with real module exports |
| `kelpie-wasm/src/runtime.rs` | NEW - Real wasmtime integration |
| `kelpie-wasm/Cargo.toml` | Added serde_json, wasi-common, wasi-cap-std-sync |
| `kelpie-server/src/tools/executor.rs` | NEW - Unified tool executor |
| `kelpie-server/src/tools/mod.rs` | Export executor module |
| `kelpie-server/src/tools/registry.rs` | Added sandbox_pool, multi-language support |

### RikaiOS (rikaios repo)

| File | Change |
|------|--------|
| `src/telegram.rs` | Added app_state to BotState, async apply_proposal |
| `src/main.rs` | Pass app_state to run_telegram_bot |

## Verification

```bash
# All tests pass
cargo test -p kelpie-wasm      # 6 passed
cargo test -p kelpie-tools     # 76 passed
cargo test -p kelpie-server    # 216 passed
```
