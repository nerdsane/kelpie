# kelpie-tools

**Examined:** 2026-01-24T15:23:40.144987+00:00

## Summary

Tool execution framework with UnifiedToolRegistry, builtin tools (filesystem, git, shell), MCP integration, and SimTool for DST

## Connections

- kelpie-server
- kelpie-sandbox
- kelpie-dst

## Details

**Components:**
- UnifiedToolRegistry: Central registry for builtin + MCP tools
- Builtin tools: filesystem, git, shell
- MCP integration: External tool servers
- SimTool: Deterministic simulation tools

**Key Traits:**
- Tool: async execute() -> ToolResult
- ToolRegistry: register, unregister, execute, list

**State:**
- Tools are stateless (registry manages tool references)
- MCP server connections managed externally
