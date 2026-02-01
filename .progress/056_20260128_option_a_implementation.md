# Plan: Fix Kelpie for Minimal Assistant (Option A)

**Status:** COMPLETE
**Created:** 2026-01-28
**Completed:** 2026-01-28
**Scope:** Address remaining gaps for minimal assistant functionality

---

## Executive Summary

All gaps fixed for minimal assistant functionality.

| Gap | Component | Work Needed | Status |
|-----|-----------|-------------|--------|
| 1 | MCP Stdio Transport | Fix race condition in routing | ✅ Complete |
| 2 | Call Chain Propagation | Pass call context in nested calls | ✅ Complete |
| 3 | CLI Interface | Full implementation | ✅ Complete |
| 4 | Telegram Interface | Full implementation | ✅ Complete |

---

## Gap 1: MCP Stdio Transport ✅

### Problem
The stdio transport in `crates/kelpie-tools/src/mcp.rs` had a race condition:
- Response router could receive before pending map was populated
- Extra indirection vs working SSE pattern

### Solution
Refactored `StdioTransport` to match the simpler SSE pattern:
- Insert into pending map BEFORE sending request
- Simplified to direct flow with shared pending map
- Added proper error propagation on connection close/timeout

### Files Modified
- [x] `crates/kelpie-tools/src/mcp.rs` - Refactored StdioTransport (~200 lines)

---

## Gap 2: Call Chain Propagation ✅

### Problem
When Agent A calls Agent B via `call_agent`, Agent B's `ToolExecutionContext` was created fresh,
ignoring the parent's call chain.

### Solution
Added `CallContextInfo` struct and propagated it through agent invocations:
- Added `call_context: Option<CallContextInfo>` to `HandleMessageFullRequest`
- Agent B now receives parent's call_depth and call_chain
- Properly detects A→B→C→A cycles

### Files Modified
- [x] `crates/kelpie-server/src/actor/agent_actor.rs` - Added CallContextInfo, updated handler
- [x] `crates/kelpie-server/src/actor/mod.rs` - Exported CallContextInfo
- [x] `crates/kelpie-server/src/tools/agent_call.rs` - Pass call context when invoking
- [x] `crates/kelpie-server/src/service/mod.rs` - Set call_context: None for API calls
- [x] `crates/kelpie-server/tests/full_lifecycle_dst.rs` - Updated test

---

## Gap 3: CLI Interface ✅

### Problem
`kelpie-cli` was a stub - all commands printed "Not yet implemented".

### Solution
Implemented full CLI with HTTP client, REPL, and streaming support:
- reqwest-based HTTP client for API calls
- rustyline-based interactive REPL with history
- SSE streaming support for real-time responses
- Colored output with terminal formatting

### Commands Implemented
- [x] `kelpie status` - Server health and version
- [x] `kelpie agents list` - List all agents
- [x] `kelpie agents get <id>` - Get agent details
- [x] `kelpie agents create <name>` - Create new agent
- [x] `kelpie agents delete <id>` - Delete agent
- [x] `kelpie chat <agent_id>` - Interactive REPL with streaming
- [x] `kelpie invoke <agent_id>` - Single message send
- [x] `kelpie doctor` - Full diagnostics

### Files Modified
- [x] `crates/kelpie-cli/Cargo.toml` - Added reqwest, rustyline, colored, dirs, etc.
- [x] `crates/kelpie-cli/src/main.rs` - Full command implementation
- [x] `crates/kelpie-cli/src/client.rs` - NEW - HTTP client with all API methods
- [x] `crates/kelpie-cli/src/repl.rs` - NEW - Interactive REPL with streaming

---

## Gap 4: Telegram Interface ✅

### Problem
No Telegram bot existed.

### Solution
Added feature-gated Telegram bot to kelpie-server:
- teloxide-based bot with configurable strategies
- User-to-agent mapping (one agent per user, or shared agent)
- Per-user rate limiting
- Long message splitting for Telegram's limits
- Commands: /start, /help, /reset

### Configuration
```bash
KELPIE_TELEGRAM_TOKEN=<bot_token>
KELPIE_TELEGRAM_AGENT_STRATEGY=user_agent  # or shared_agent
KELPIE_TELEGRAM_SHARED_AGENT_ID=<agent_id>  # for shared_agent mode
KELPIE_TELEGRAM_RATE_LIMIT=20  # messages per minute per user
```

### Files Modified
- [x] `Cargo.toml` (workspace) - Added teloxide dependency
- [x] `crates/kelpie-server/Cargo.toml` - Added telegram feature
- [x] `crates/kelpie-server/src/lib.rs` - Added interface module
- [x] `crates/kelpie-server/src/interface/mod.rs` - NEW - Module definition
- [x] `crates/kelpie-server/src/interface/telegram.rs` - NEW - Full bot implementation

---

## Verification

All tests pass:
```bash
# MCP tests pass (28 tests)
cargo test -p kelpie-tools mcp

# Multi-agent DST tests pass (8 tests)
cargo test -p kelpie-server --test multi_agent_dst --features dst

# Full workspace tests pass
cargo test --workspace

# No clippy warnings
cargo clippy --workspace
```

### Manual Verification Commands
```bash
# Start server
ANTHROPIC_API_KEY=... cargo run -p kelpie-server

# In another terminal - CLI commands
cargo run -p kelpie-cli -- status
cargo run -p kelpie-cli -- agents list
cargo run -p kelpie-cli -- agents create my-agent
cargo run -p kelpie-cli -- chat <agent_id>

# Telegram (requires bot token)
KELPIE_TELEGRAM_TOKEN=... cargo run -p kelpie-server --features telegram
```

---

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-28 10:00 | Start with MCP stdio fix | Foundation for other work | None |
| 2026-01-28 10:30 | Use shared pending map pattern | Simpler than channel routing | Slight coupling |
| 2026-01-28 11:00 | Add CallContextInfo to HandleMessageFullRequest | Backward compatible with serde default | Slight API change |
| 2026-01-28 12:00 | Use rustyline 13 | Newer versions require rustc 1.88 | Older version |
| 2026-01-28 13:00 | Feature-gate Telegram | Keeps default build smaller | Extra build flag needed |

