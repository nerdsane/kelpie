# ci-config

**Examined:** 2026-01-23T02:54:29.450664+00:00

## Summary

CI workflow claims to run Letta SDK tests but selectively runs only tests that pass, excluding known failures. Test isolation questionable.

## Connections

- letta-tests
- kelpie-server

## Details

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

## Issues

### [HIGH] CI selectively skips failing tests instead of fixing them

**Evidence:** Comment: 'We only run the tests we know pass' - excludes search/groups/identities

### [MEDIUM] Dummy API key prevents real LLM testing in CI

**Evidence:** ANTHROPIC_API_KEY: 'sk-dummy-key'

### [MEDIUM] Unknown what percentage of Letta's full test suite passes

**Evidence:** Only 4 test files selected from full Letta test suite
