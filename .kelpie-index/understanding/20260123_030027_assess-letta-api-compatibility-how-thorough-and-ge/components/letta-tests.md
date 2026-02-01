# letta-tests

**Examined:** 2026-01-23T02:54:29.226791+00:00

## Summary

Test coverage is weak: 27% honest tests, 73% smoke tests. Critical persistence verification missing. One test has contradictory assertion (passes when should fail).

## Connections

- kelpie-server
- letta-api

## Details

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

## Issues

### [CRITICAL] Import test passes when it should fail - fault injection disconnected

**Evidence:** test_dst_import_with_message_write_fault: 100% write fault + OK assertion

### [HIGH] 73% of tests are smoke tests only checking status codes

**Evidence:** 8 of 11 tests only assert HTTP status, not data

### [HIGH] No persistence verification tests

**Evidence:** No test creates data then reads it back to verify

### [MEDIUM] MockLlmClient used everywhere - no real LLM testing

**Evidence:** StubHttpClient returns hardcoded responses
