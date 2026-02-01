# kelpie-server

**Examined:** 2026-01-24T23:29:46.924931+00:00

## Summary

PARTIAL - REST API models complete, LLM integration working, but MCP is stub and archival search unimplemented

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-storage
- kelpie-dst

## Details

Working:
- REST API models (Letta-compatible with serde aliases)
- LLM integration: OpenAI + Anthropic with streaming
- Tool definitions with JSON schema
- Agent/Block/Message storage (in-memory HashMap)
- Job scheduling (cron/interval/once)
- SSE streaming parser
- AgentActor + AgentService setup code exists

Phase 5/6 Migration in Progress:
- Agent_service and dispatcher instantiated
- But legacy HashMap storage still used for HTTP handlers
- TODO comments indicate migration incomplete

Not Implemented:
- MCP client (data models only, no execution)
- Archival memory search (no vector embeddings)
- Message windowing for context management
- Tool execution pipeline (registration exists, executor unclear)

## Issues

### [HIGH] MCP integration is stub only - data models exist but no client implementation

**Evidence:** mcp_servers HashMap stored but never used, no discovery/execution code

### [HIGH] Archival memory search not operational - no vector embeddings or retrieval

**Evidence:** ArchivalEntry model exists but no search implementation

### [MEDIUM] Phase 5/6 actor migration incomplete - still using legacy HashMap storage

**Evidence:** TODO comments: 'Remove after HTTP handlers migrated to agent_service'
