# Codebase Map

**Task:** Find all reasons precommit hook is failing and identify cleanup needed
**Generated:** 2026-01-24T21:08:06.375503+00:00
**Components:** 5
**Issues Found:** 6

---

## Components Overview

### clippy-warnings

**Summary:** clippy fails due to same compilation errors as test failures

**Connects to:** test-failures, kelpie-server

**Details:**

Clippy cannot run because the workspace fails to compile. Same E0599 errors as test suite:

- No function or associated item named `new_without_wal` 
- No method named `recover`

Clippy will only work once the test compilation errors are fixed.

**Issues (1):**
- [HIGH] clippy blocked by compilation errors

---

### formatting-issues

**Summary:** cargo fmt --check fails with 7 formatting violations in 2 files

**Connects to:** kelpie-server

**Details:**

**Files with formatting issues:**

1. **crates/kelpie-server/tests/common/invariants.rs** (6 issues):
   - Line 242: Long write!() macro call needs multi-line formatting
   - Line 390: Function signature too long (verify_capacity_bounds)
   - Line 573: Long if-let statement (verify_lease_validity)
   - Line 596: Long if-let statement (verify_lease_exclusivity)

2. **crates/kelpie-server/tests/tla_bug_patterns_dst.rs** (3 issues):
   - Line 20: Long use statement with 5 imported functions
   - Line 94: Long panic!() call
   - Line 362: println! call needs multi-line formatting

**Fix:**
Run `cargo fmt` to auto-fix all formatting issues.

**Issues (1):**
- [MEDIUM] cargo fmt --check fails with 7 formatting violations

---

### precommit-hooks

**Summary:** Pre-commit hook enforces 3 checks in sequence: fmt, clippy, test

**Connects to:** formatting-issues, clippy-warnings, test-failures

**Details:**

The hook at `hooks/pre-commit` runs these checks:

1. **cargo fmt --check** - FAILS (7 formatting issues)
2. **cargo clippy --workspace --all-targets** - FAILS (compilation errors)
3. **cargo test --all** - Skipped if previous checks fail, would FAIL (11 test compilation errors)

**Hook behavior:**
- Uses `set -e` (exits on first error)
- Each check uses `run_check()` function that captures output
- If any check fails, FAILED=1 and hook exits with code 1
- Also checks for `.kelpie-index/constraints/extracted.json` for additional hard constraints (file doesn't exist, shows warning)

**Current state:**
The hook will fail at step 1 (cargo fmt --check), never reaching clippy or tests.

**Issues (1):**
- [HIGH] Pre-commit hook will fail on first check (cargo fmt)

---

### test-failures

**Summary:** 11 test files fail to compile due to deleted AgentService methods

**Connects to:** kelpie-server

**Details:**

The AgentService struct no longer has `new_without_wal()` and `recover()` methods, but 11 test files still reference them:

**Compilation Errors:**
1. `AgentService::new_without_wal(handle)` - method removed but used in 10 files
2. `service.recover().await` - method removed, used in agent_deactivation_timing.rs

**Affected test files:**
- runtime_pilot_test.rs (new_without_wal at line 77)
- delete_atomicity_test.rs (new_without_wal at line 370)
- agent_deactivation_timing.rs (both new_without_wal at line 494, and recover() at line 85)
- real_llm_integration.rs (new_without_wal at line 85)
- agent_service_fault_injection.rs (new_without_wal at line 681)
- appstate_integration_dst.rs
- agent_service_dst.rs
- agent_message_handling_dst.rs
- agent_streaming_dst.rs
- llm_token_streaming_dst.rs
- agent_service_send_message_full_dst.rs

**Current implementation:**
AgentService only has `new(dispatcher: DispatcherHandle<R>)` constructor.

**Fix needed:**
Replace all `AgentService::new_without_wal(handle)` with `AgentService::new(handle)`.
Remove or replace all `service.recover()` calls.

**Issues (2):**
- [CRITICAL] 11 test files fail to compile due to deleted AgentService::new_without_wal() method
- [HIGH] agent_deactivation_timing.rs calls deleted recover() method

---

### untracked-files

**Summary:** Many untracked files but most are gitignored, some should be tracked

**Connects to:** precommit-hooks

**Details:**

**Untracked files analysis:**

**Correctly ignored (no action needed):**
- `.agentfs/` - 36 database files (already in .gitignore)
- `.kelpie-index/understanding/` - Generated docs (semantic/ is ignored)
- Cargo.lock - Modified (workspace dependency)

**Files that SHOULD be tracked:**
1. `.claude/` - Claude Code configuration
2. `.env.example` - Example environment file
3. `.mcp.json` - MCP server configuration
4. `.progress/*.md` - 9 progress/plan files (031-041)
5. `crates/kelpie-registry/src/fdb.rs` - New source file
6. `crates/kelpie-server/src/invariants.rs` - New source file
7. `crates/kelpie-server/tests/common/` - New test infrastructure
8. `crates/kelpie-server/tests/tla_bug_patterns_dst.rs` - New test
9. `crates/kelpie-storage/src/wal.rs` - New source file
10. `docs/adr/021-snapshot-type-system.md` - New ADR
11. `docs/papers/` - Documentation
12. `docs/tla/*.cfg` - TLA+ configs (3 files)
13. `hooks/` - Git hooks directory
14. `install-hooks.sh` - Hook installer
15. `kelpie-mcp/` - MCP server implementation
16. `launch_tla_agents*.sh` and `.scpt` - Helper scripts
17. `.vision/EVI*.md` - Vision documents

**Pre-commit impact:**
These untracked files won't cause the hook to fail, but they represent uncommitted work.

**Issues (1):**
- [LOW] Many untracked files should be committed

---

## Component Connections

```
clippy-warnings -> test-failures, kelpie-server
formatting-issues -> kelpie-server
precommit-hooks -> formatting-issues, clippy-warnings, test-failures
test-failures -> kelpie-server
untracked-files -> precommit-hooks
```
