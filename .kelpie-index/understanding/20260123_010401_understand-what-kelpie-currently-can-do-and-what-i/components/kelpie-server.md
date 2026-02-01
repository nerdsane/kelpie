# kelpie-server

**Examined:** 2026-01-23T01:03:14.885915+00:00

## Summary

Letta-compatible agent server with REST API, LLM integration, tools, actor-based architecture - extensive DST coverage

## Connections

- kelpie-runtime
- kelpie-storage
- kelpie-dst
- kelpie-vm

## Details

**WORKING (based on test index - 70+ DST tests):**
- REST API: agents, messages, blocks, tools, archival, teleport
- Agent types: MemGPT, LettaV1, React with capability-based tool filtering
- LLM integration via trait abstraction (Claude/OpenAI compatible)
- Memory tools: core_memory_append, archival_memory_insert/search
- Heartbeat/pause mechanism for agent control
- Code execution tool support (Python, JavaScript, TypeScript, R)
- Session storage with FDB backend
- Teleport service for agent migration
- SSE streaming for responses

**DST Test Coverage:**
- heartbeat_dst.rs: 18 tests (pause, clock skew, determinism)
- heartbeat_integration_dst.rs: 8 tests (fault injection)
- agent_types_dst.rs: 15 tests (capabilities, tool filtering)
- agent_loop_types_dst.rs: 11 tests (loop behavior under faults)
- heartbeat_real_dst.rs: 13 tests (real registry integration)

**Architecture:**
- Actor-based via AgentActor wrapping agent state
- AgentService layer between REST and Dispatcher
- FDB for hot-path, UMI for search (dual storage)
- SimLlmClient for deterministic testing

## Issues

### [MEDIUM] Requires external LLM API key for production - not testable without mock

**Evidence:** llm.rs config detection

### [LOW] FDB storage tests require external cluster

**Evidence:** storage/fdb.rs tests ignored
