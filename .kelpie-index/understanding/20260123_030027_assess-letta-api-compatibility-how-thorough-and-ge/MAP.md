# Codebase Map

**Task:** Assess Letta API compatibility: how thorough and genuine is the implementation vs hacks/workarounds to pass tests
**Generated:** 2026-01-23T03:00:27.493537+00:00
**Components:** 4
**Issues Found:** 16

---

## Components Overview

### ci-config

**Summary:** CI workflow claims to run Letta SDK tests but selectively runs only tests that pass, excluding known failures. Test isolation questionable.

**Connects to:** letta-tests, kelpie-server

**Details:**

**CI Analysis (.github/workflows/letta-compatibility.yml):**

1. **Selective Test Execution**:
   - Comment: "We only run the tests we know pass/are relevant"
   - Excludes: search/groups/identities tests "which we know fail/skip"
   - Runs: agents_test.py, blocks_test.py, tools_test.py, mcp_servers_test.py

2. **Fake API Key**:
   - Uses `ANTHROPIC_API_KEY: "sk-dummy-key"`
   - Tests shouldn't hit real LLM APIs - but this limits what can be verified

3. **Test Source**:
   - Clones actual Letta repo: `git clone https://github.com/letta-ai/letta.git`
   - Runs Letta's own test suite against Kelpie server
   - This is legitimate validation, but selective execution hides failures

4. **What's Actually Tested**:
   - Agent CRUD operations
   - Block management
   - Tool registration
   - MCP server endpoints

5. **What's Skipped (by design)**:
   - Search functionality
   - Groups
   - Identities
   - Any test requiring real LLM responses

**Issues (3):**
- [HIGH] CI selectively skips failing tests instead of fixing them
- [MEDIUM] Dummy API key prevents real LLM testing in CI
- [MEDIUM] Unknown what percentage of Letta's full test suite passes

---

### kelpie-server

**Summary:** Agent server API implementation with ~17 API modules totaling 6039 lines. Core CRUD operations are real but persistence is delegated to opaque AppState trait.

**Connects to:** letta-api, letta-tests

**Details:**

**Implementation Quality: 6.5/10, ~70% honest**

The API layer has real HTTP routing, validation, and request handling. However:

1. **Black Box Persistence**: All state operations (add_message, get_agent, persist_agent) delegate to AppState<R> - cannot verify data actually persists without examining the state implementation

2. **Silent Failure Patterns**: 
   - import_export.rs: Returns empty vec[] on message fetch failures, masking data loss
   - streaming.rs: `let _ = state.add_message()` discards persistence errors
   - import returns OK even when 100% of messages fail to import

3. **Incomplete Features**:
   - Cron scheduling returns None for all jobs (placeholder)
   - Tool system only implements "shell" tool - hardcoded dispatch
   - Phase 7.9 streaming marked as "simplified/incomplete"

4. **Test Quality**: Tests use MockLlmClient and SimStorage (in-memory). No tests verify data persists across requests.

**Issues (5):**
- [HIGH] Silent failure in import - returns OK when message writes fail
- [HIGH] Cron scheduling completely non-functional
- [HIGH] Tool system hardcoded to only 'shell' tool
- [MEDIUM] Persistence errors silently discarded in streaming
- [MEDIUM] Tests don't verify data persistence

---

### letta-api

**Summary:** Letta API schema compatibility is 6/10. Core endpoints exist but response schemas have significant mismatches with Letta's expectations.

**Connects to:** kelpie-server, letta-tests

**Details:**

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

**Issues (4):**
- [HIGH] tool_call vs tool_calls plural mismatch - breaks OpenAI spec
- [HIGH] Missing user_id/org_id in agent responses
- [MEDIUM] Memory structure flat vs hierarchical
- [MEDIUM] Hardcoded embedding model default

---

### letta-tests

**Summary:** Test coverage is weak: 27% honest tests, 73% smoke tests. Critical persistence verification missing. One test has contradictory assertion (passes when should fail).

**Connects to:** kelpie-server, letta-api

**Details:**

**Test Quality Analysis:**

1. **Real Tests (27%)** - Only 2 of 11 tests verify actual data:
   - test_dst_conversation_search_date_with_faults: Verifies content filtering
   - test_dst_export_with_message_read_fault: Checks export structure

2. **Smoke Tests (73%)** - Only check HTTP status codes:
   - test_dst_summarization_with_llm_faults: 200 OR 500 both pass
   - test_dst_scheduling_job_write_fault: Any of BAD_REQUEST|INTERNAL_SERVER_ERROR|NOT_FOUND passes
   - Most tests: "assert status code is one of several" with no data validation

3. **Contradictory Test**:
   - test_dst_import_with_message_write_fault: Injects 100% StorageWriteFail on message_write
   - Assertion: StatusCode::OK
   - This SHOULD FAIL but PASSES - indicates fault injection not connected to import logic

4. **Missing Test Categories**:
   - No create→read→verify round-trip tests
   - No concurrent operation tests
   - No actual LLM integration (all use MockLlmClient)
   - No tool execution end-to-end tests

**Issues (4):**
- [CRITICAL] Import test passes when it should fail - fault injection disconnected
- [HIGH] 73% of tests are smoke tests only checking status codes
- [HIGH] No persistence verification tests
- [MEDIUM] MockLlmClient used everywhere - no real LLM testing

---

## Component Connections

```
ci-config -> letta-tests, kelpie-server
kelpie-server -> letta-api, letta-tests
letta-api -> kelpie-server, letta-tests
letta-tests -> kelpie-server, letta-api
```
