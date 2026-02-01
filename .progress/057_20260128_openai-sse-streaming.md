# Issue #76: OpenAI SSE Streaming Implementation

## Overview
Add Server-Sent Events (SSE) streaming support for OpenAI API, matching the existing Anthropic streaming.

## Research Findings (RLM Analysis)

### Current Architecture
1. **LlmClient** (`crates/kelpie-server/src/llm.rs`):
   - `stream_complete_with_tools()` - dispatches to provider-specific streaming
   - Currently only supports Anthropic: `self.stream_anthropic(messages, tools).await`
   - Returns error for OpenAI: `"Streaming only supported for Anthropic API"`

2. **Anthropic Streaming**:
   - `stream_anthropic()` - builds request with `"stream": true`
   - `parse_sse_stream()` - parses SSE events
   - Events: `content_block_delta` → `delta.text`, `message_stop` → done

3. **Streaming Endpoint** (`crates/kelpie-server/src/api/streaming.rs`):
   - `POST /v1/agents/{agent_id}/messages/stream`
   - Calls `llm.stream_complete_with_tools()`
   - Converts `StreamDelta` to Letta-compatible SSE events

### OpenAI Streaming Format (vs Anthropic)
| Aspect | Anthropic | OpenAI |
|--------|-----------|--------|
| Content delta | `{"type":"content_block_delta","delta":{"text":"..."}}` | `{"choices":[{"delta":{"content":"..."}}]}` |
| Completion | `{"type":"message_stop"}` | `{"choices":[{"finish_reason":"stop"}]}` then `data: [DONE]` |
| Auth header | `x-api-key` | `Authorization: Bearer` |
| Endpoint | `/messages` | `/chat/completions` |

## Implementation Plan

### Phase 1: Add `stream_openai()` method
1. Create `stream_openai()` in `llm.rs` that:
   - Builds request with `"stream": true`
   - Uses existing `send_streaming()` HTTP method
   - Returns byte stream for parsing

### Phase 2: Add `parse_openai_sse_stream()` function
1. Similar to `parse_sse_stream()` but handles:
   - `choices[0].delta.content` for text
   - `choices[0].finish_reason == "stop"` for completion
   - `data: [DONE]` marker (special case - not JSON)

### Phase 3: Update `stream_complete_with_tools()`
1. Change dispatch logic:
   ```rust
   if self.config.is_anthropic() {
       self.stream_anthropic(messages, tools).await
   } else {
       self.stream_openai(messages, tools).await  // NEW
   }
   ```

### Phase 4: Handle Tool Calls (Stretch)
- OpenAI uses `delta.tool_calls` array for streaming tool calls
- May defer to future PR if complex

## Acceptance Criteria
- [ ] `POST /agents/:id/messages/stream` works with OpenAI models
- [ ] Token streaming returns content incrementally
- [ ] `[DONE]` marker properly detected
- [ ] Parity with Anthropic streaming behavior

## Status
- [x] Research complete
- [x] Implementation
- [x] Testing (5 unit tests added)
- [x] PR created: https://github.com/rita-aga/kelpie/pull/82

## Implementation Summary

### Files Changed
- `crates/kelpie-server/src/llm.rs`

### Changes Made
1. **Updated `stream_complete_with_tools()`** - Now dispatches to `stream_openai()` for non-Anthropic providers
2. **Added `stream_openai()` method** - Builds streaming request to OpenAI `/chat/completions` endpoint
3. **Added `parse_openai_sse_stream()` function** - Parses OpenAI SSE format:
   - `choices[0].delta.content` for text content
   - `data: [DONE]` marker for stream completion
   - Ignores empty content deltas
4. **Added 4 unit tests**:
   - `test_is_anthropic` - Provider detection
   - `test_parse_openai_sse_stream_content` - Full content parsing
   - `test_parse_openai_sse_stream_handles_done_marker` - [DONE] handling
   - `test_parse_openai_sse_stream_ignores_empty_content` - Empty delta handling

### Known Limitations
- Tool calling during streaming deferred to future PR (OpenAI uses different delta format for tool calls)
- Tested via unit tests; integration test with real OpenAI API requires API key
