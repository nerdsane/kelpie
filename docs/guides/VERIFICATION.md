# Verification-First Development Guide

## Core Principle

**Trust execution, not documentation. Verify before claiming complete.**

```
┌─────────────────────────────────────┐
│  Trusted Sources                    │
├─────────────────────────────────────┤
│  ✅ Test execution output           │
│  ✅ Command execution results       │
│  ✅ Actual code (after reading it)  │
└─────────────────────────────────────┘

┌─────────────────────────────────────┐
│  Untrusted Sources                  │
├─────────────────────────────────────┤
│  ❌ Documentation (might be stale)  │
│  ❌ Comments (might be outdated)    │
│  ❌ Plan checkboxes (might be lies) │
└─────────────────────────────────────┘
```

## Verification Pyramid

Kelpie uses a **verification pyramid** with increasing confidence levels.

### Quick Reference

```bash
# Level 1: Unit Tests (~1-5 seconds)
cargo test -p kelpie-core
cargo test -p kelpie-server --lib

# Level 2: DST - Deterministic Simulation (~5-30 seconds)
cargo test -p kelpie-dst --release
DST_SEED=12345 cargo test -p kelpie-dst  # Reproducible

# Level 3: Integration Tests (~30-60 seconds)
cargo test -p kelpie-server --test '*'

# Level 4: Stateright Model Checking (~60+ seconds)
cargo test stateright_* -- --ignored

# Level 5: Kani Bounded Proofs (when installed)
cargo kani --package kelpie-core --harness verify_single_activation

# Full Verification (before commit)
cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --check
```

### When to Use Each Level

| Level | Time | Use When |
|-------|------|----------|
| **Unit** | ~5s | After every change |
| **DST** | ~30s | After logic changes, before commit |
| **Integration** | ~60s | Before merging PRs |
| **Stateright** | ~60s+ | For distributed invariants |
| **Kani** | varies | For critical proofs |

### Hard Controls

- Pre-commit hook runs `cargo test` + `cargo clippy`
- Task completion requires verification evidence
- Index queries warn if code changed since last test

## Task Workflow

For any non-trivial task:
1. **Load constraints** - Read `.vision/CONSTRAINTS.md` (non-negotiable rules)
2. **Query indexes** - Use `index_symbols`, `index_modules` to understand scope
3. **Create plan** - Save to `.progress/NNN_YYYYMMDD_task-name.md`
4. **Execute phases** - Verify each by running tests, not reading docs
5. **Final verification** - `cargo test`, `cargo clippy`, `cargo fmt`

## Verification Workflow

When asked "Is X implemented?" or "Does Y work?":
1. **Find tests** - Search for relevant test files
2. **Run tests** - Execute and capture output
3. **Report results** - What you OBSERVED, not what docs claim

```bash
# Example: Verifying snapshot feature
cargo test snapshot  # Run relevant tests
# Report: "Ran 5 snapshot tests, 4 passed, 1 failed (restore_concurrent)"
```

## Exploration Workflow

Start broad, narrow down:
1. **Modules** - `cargo metadata` to see crate structure
2. **Dependencies** - `index_deps` to understand relationships
3. **Symbols** - `grep` for specific implementations
4. **Code reading** - Read the actual implementation
5. **Test verification** - Run tests to confirm understanding

## Handoff Protocol

When taking over from another agent:
1. **NEVER trust checkboxes** - Re-verify completed phases
2. **Run the tests** - See if claimed work actually passes
3. **Check for regressions** - Code may have changed since completion
4. **Document findings** - Update plan with actual verification status

## Slop Hunt

Periodic cleanup for:
- **Dead code** - Unused functions, dependencies
- **Orphaned code** - Old implementations not deleted
- **Duplicates** - Same logic in multiple places
- **Fake DST** - Tests claiming determinism but aren't
- **Incomplete code** - TODOs, stubs in production

```bash
# Detection
grep -r "TODO\|FIXME" crates/ --include="*.rs"
grep -r "unwrap()\|expect(" crates/ --include="*.rs"
cargo clippy --workspace -- -W dead_code
```

## Acceptance Criteria

### No Stubs Policy

Code must be functional, not placeholder:

```rust
// FORBIDDEN - stub implementation
fn execute_tool(&self, name: &str) -> String {
    "Tool execution not yet implemented".to_string()
}

// REQUIRED - real implementation or don't merge
fn execute_tool(&self, name: &str, input: &Value) -> String {
    match name {
        "shell" => {
            let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
            self.sandbox.exec("sh", &["-c", command]).await
        }
        _ => format!("Unknown tool: {}", name),
    }
}
```

### Verification Checklist

Before marking any feature complete:

| Check | How to Verify |
|-------|---------------|
| Code compiles | `cargo build` |
| Tests pass | `cargo test` |
| No warnings | `cargo clippy` |
| Actually works | Run the server, hit the endpoint, see real output |
| Edge cases handled | Test with empty input, large input, malformed input |
| Errors are meaningful | Trigger errors, verify messages are actionable |

### What "Done" Means

A feature is done when:

- [ ] Implementation is complete (no TODOs, no stubs)
- [ ] Unit tests exist and pass
- [ ] Integration test exists and passes
- [ ] You have personally run it and seen it work
- [ ] Error paths have been tested
- [ ] Documentation updated if needed

## Commit Policy: Only Working Software

**Never commit broken code.** Every commit must represent working software.

### Pre-Commit Verification

Before every commit, you MUST verify the code works:

```bash
# Required before EVERY commit
cargo test           # All tests must pass
cargo clippy         # No warnings allowed
cargo fmt --check    # Code must be formatted
```

### Why This Matters

- Every commit is a potential rollback point
- Broken commits make `git bisect` useless
- CI should never be the first place code is tested
- Other developers should be able to checkout any commit

### Commit Checklist

Before running `git commit`:

1. **Run `cargo test`** - All tests must pass
2. **Run `cargo clippy`** - Fix any warnings
3. **Run `cargo fmt`** - Code must be formatted
4. **Review changes** - `git diff` to verify what's being committed
5. **Write clear message** - Describe what and why, not how

### If Tests Fail

Do NOT commit. Instead:
1. Fix the failing tests
2. If the fix is complex, consider `git stash` to save work
3. Never use `--no-verify` to skip pre-commit hooks
4. Never commit with `// TODO: fix this test` comments
