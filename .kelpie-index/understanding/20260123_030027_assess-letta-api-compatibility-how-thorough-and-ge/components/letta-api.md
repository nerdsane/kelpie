# letta-api

**Examined:** 2026-01-23T02:54:28.788009+00:00

## Summary

Letta API schema compatibility is 6/10. Core endpoints exist but response schemas have significant mismatches with Letta's expectations.

## Connections

- kelpie-server
- letta-tests

## Details

**Schema Compatibility Issues:**

1. **Blocking Issues**:
   - `tool_call` (singular) vs Letta's `tool_calls` (plural array) - breaks OpenAI spec
   - Memory structure is flat Vec<Block> vs Letta's hierarchical MemoryBank
   - Missing required fields: user_id, org_id, token_count

2. **Message Format Mismatch**:
   - message_type is String, Letta expects enum
   - Missing function_calls vs tool_use distinction
   - No assistant_id field

3. **Agent Response Gaps**:
   - Missing: user_id, org_id, llm_config, context_window, max_messages
   - tool_ids instead of full Tool objects
   - No memory_bank structure

4. **Hardcoded Defaults**:
   - Embedding model hardcoded to "openai/text-embedding-3-small"
   - Block limit defaults to 20000 (Letta likely uses 8000)
   - Agent capabilities list doesn't validate against actual tool registry

## Issues

### [HIGH] tool_call vs tool_calls plural mismatch - breaks OpenAI spec

**Evidence:** models.rs: pub tool_call: Option<ToolCall> (singular)

### [HIGH] Missing user_id/org_id in agent responses

**Evidence:** AgentState struct lacks user_id, org_id fields

### [MEDIUM] Memory structure flat vs hierarchical

**Evidence:** Vec<Block> in AgentState vs Letta's MemoryBank with CoreMemory/ArchivalMemory

### [MEDIUM] Hardcoded embedding model default

**Evidence:** default_embedding_model() returns hardcoded string
