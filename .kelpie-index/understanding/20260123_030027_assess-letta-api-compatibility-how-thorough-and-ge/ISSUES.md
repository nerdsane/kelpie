# Issues Found During Examination

**Task:** Assess Letta API compatibility: how thorough and genuine is the implementation vs hacks/workarounds to pass tests
**Generated:** 2026-01-23T03:00:27.493835+00:00
**Total Issues:** 16

---

## CRITICAL (1)

### [letta-tests] Import test passes when it should fail - fault injection disconnected

**Evidence:** test_dst_import_with_message_write_fault: 100% write fault + OK assertion

*Found: 2026-01-23T02:54:29.226801+00:00*

---

## HIGH (8)

### [kelpie-server] Silent failure in import - returns OK when message writes fail

**Evidence:** import_export.rs: tracing::warn then continue on add_message failure

*Found: 2026-01-23T02:54:28.317586+00:00*

---

### [kelpie-server] Cron scheduling completely non-functional

**Evidence:** models.rs: ScheduleType::Cron => None (returns None for all cron jobs)

*Found: 2026-01-23T02:54:28.317593+00:00*

---

### [kelpie-server] Tool system hardcoded to only 'shell' tool

**Evidence:** streaming.rs: match name { 'shell' => ... _ => 'Unknown tool' }

*Found: 2026-01-23T02:54:28.317596+00:00*

---

### [letta-api] tool_call vs tool_calls plural mismatch - breaks OpenAI spec

**Evidence:** models.rs: pub tool_call: Option<ToolCall> (singular)

*Found: 2026-01-23T02:54:28.788020+00:00*

---

### [letta-api] Missing user_id/org_id in agent responses

**Evidence:** AgentState struct lacks user_id, org_id fields

*Found: 2026-01-23T02:54:28.788023+00:00*

---

### [letta-tests] 73% of tests are smoke tests only checking status codes

**Evidence:** 8 of 11 tests only assert HTTP status, not data

*Found: 2026-01-23T02:54:29.226804+00:00*

---

### [letta-tests] No persistence verification tests

**Evidence:** No test creates data then reads it back to verify

*Found: 2026-01-23T02:54:29.226806+00:00*

---

### [ci-config] CI selectively skips failing tests instead of fixing them

**Evidence:** Comment: 'We only run the tests we know pass' - excludes search/groups/identities

*Found: 2026-01-23T02:54:29.450674+00:00*

---

## MEDIUM (7)

### [kelpie-server] Persistence errors silently discarded in streaming

**Evidence:** streaming.rs: let _ = state.add_message()

*Found: 2026-01-23T02:54:28.317599+00:00*

---

### [kelpie-server] Tests don't verify data persistence

**Evidence:** All tests use SimStorage, no create->read verification

*Found: 2026-01-23T02:54:28.317601+00:00*

---

### [letta-api] Memory structure flat vs hierarchical

**Evidence:** Vec<Block> in AgentState vs Letta's MemoryBank with CoreMemory/ArchivalMemory

*Found: 2026-01-23T02:54:28.788024+00:00*

---

### [letta-api] Hardcoded embedding model default

**Evidence:** default_embedding_model() returns hardcoded string

*Found: 2026-01-23T02:54:28.788027+00:00*

---

### [letta-tests] MockLlmClient used everywhere - no real LLM testing

**Evidence:** StubHttpClient returns hardcoded responses

*Found: 2026-01-23T02:54:29.226808+00:00*

---

### [ci-config] Dummy API key prevents real LLM testing in CI

**Evidence:** ANTHROPIC_API_KEY: 'sk-dummy-key'

*Found: 2026-01-23T02:54:29.450676+00:00*

---

### [ci-config] Unknown what percentage of Letta's full test suite passes

**Evidence:** Only 4 test files selected from full Letta test suite

*Found: 2026-01-23T02:54:29.450678+00:00*

---
