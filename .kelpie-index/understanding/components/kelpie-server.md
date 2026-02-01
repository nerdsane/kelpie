# kelpie-server

**Examined:** 2026-01-22T21:31:39.761125+00:00

## Summary

Agent orchestration platform with HTTP API, MCP integration, multi-provider LLM support (Anthropic/OpenAI), tool registry, and sandboxed code execution

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-storage
- kelpie-agent
- kelpie-vm

## Details

79 files across: API (17), LLM (2), Agent impl (10), Tests (34). Components: Agent CRUD, MCP servers, Tool Registry (Builtin/MCP/Custom), Memory tools, Code execution sandbox. Architecture: HTTP API → Tool Registry → LLM completion loop → State persistence.

## Issues

### [CRITICAL] No authentication/authorization - all routes publicly accessible

**Evidence:** agents.rs, mcp_servers.rs, identities.rs

### [HIGH] No input sanitization for user-provided IDs - potential injection/path traversal

**Evidence:** agents.rs

### [HIGH] No command injection validation in MCPServerConfig::Stdio command/args

**Evidence:** mcp_servers.rs

### [HIGH] No rate limiting on LLM requests

**Evidence:** llm.rs

### [HIGH] Resource leak - ProcessSandbox not cleaned up, zombie processes

**Evidence:** code_execution.rs

### [MEDIUM] TOCTOU vulnerability in core_memory_replace

**Evidence:** memory.rs

### [MEDIUM] No tenant isolation in identities endpoint

**Evidence:** identities.rs
