# kelpie-tools

**Examined:** 2026-01-22T21:37:41.673596+00:00

## Summary

Tool registry and execution framework for LLM operations - Builtin, MCP, and Custom tool categories

## Connections

- kelpie-core
- kelpie-server
- kelpie-sandbox

## Details

11 files, 121KB. Tool categories: Builtin (Shell, Filesystem, Git), MCP (stdio-based IPC to Python), Custom (trait-based). Async execution with timeout. No central input sanitization.

## Issues

### [CRITICAL] ShellTool - no visible input sanitization, command injection possible

**Evidence:** lib.rs

### [HIGH] MCP tool arguments passed directly without sanitization

**Evidence:** mcp_client.rs

### [HIGH] Path traversal in read_file tool - no validation

**Evidence:** mcp_client.rs

### [MEDIUM] No input validation framework - delegated to each tool inconsistently

**Evidence:** registry.rs
