# Kelpie Git Hooks - Hard Controls

This directory contains git hooks that enforce hard controls at commit time.

## Overview

Kelpie uses a **layered control architecture**:

1. **Soft Controls** (Skills, prompts) - Guide agent behavior
2. **Hard Controls** (MCP tools, git hooks) - Enforce verification
3. **Hard Floor** (CI) - Final safety net

Git hooks are part of the **hard controls layer**. They cannot be bypassed by agents (except with `--no-verify`, which is discouraged).

## Available Hooks

### `pre-commit` - Verification Gate

Runs before every commit to ensure code quality.

**What it checks:**

1. **Constraint checks** (if available)
   - Loads `.kelpie-index/constraints/extracted.json`
   - Runs all "hard" enforcement checks
   - Example: `cargo clippy`, `grep -r "unwrap()"`, etc.

2. **Code formatting** (fast)
   ```bash
   cargo fmt --check
   ```
   Ensures code is formatted according to project style.

3. **Clippy linter** (medium speed)
   ```bash
   cargo clippy --all-targets --all-features -- -D warnings
   ```
   Treats warnings as errors. Catches common mistakes.

4. **Test suite** (slowest)
   ```bash
   cargo test --all
   ```
   Runs full test suite. Only runs if previous checks pass.

**Order matters:** Fast checks first, slow checks last. If formatting fails, we don't waste time running tests.

## Installation

### Fresh Clone

After cloning the repository:

```bash
./tools/install-hooks.sh
```

This copies the hooks from `tools/hooks/` to `.git/hooks/` and makes them executable.

### Manual Installation

```bash
cp tools/hooks/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
```

### Verify Installation

```bash
ls -lh .git/hooks/pre-commit
# Should show executable permissions
```

Test the hook:

```bash
# This should trigger the hook (even with no changes)
git commit --allow-empty -m "test hook"
```

## Usage

### Normal Workflow

The hook runs automatically on `git commit`:

```bash
# Make changes
vim src/some_file.rs

# Stage changes
git add src/some_file.rs

# Commit (hook runs automatically)
git commit -m "feat: Add new feature"

# Hook output:
# 🔒 Kelpie Pre-Commit Hook: Running hard controls...
# ▶️  Code formatting (cargo fmt)
# ✅ Code formatting passed
# ▶️  Clippy linter (cargo clippy)
# ✅ Clippy linter passed
# ▶️  Test suite (cargo test)
# ✅ Test suite passed
# ✅ All checks passed! Proceeding with commit.
```

### If Checks Fail

The hook will block the commit:

```bash
git commit -m "broken code"

# Hook output:
# 🔒 Kelpie Pre-Commit Hook: Running hard controls...
# ▶️  Code formatting (cargo fmt)
# ✅ Code formatting passed
# ▶️  Clippy linter (cargo clippy)
# ❌ Clippy linter FAILED
# Output:
# error: unused variable: `foo`
#  --> src/main.rs:10:9
#
# ❌ Pre-commit checks FAILED
# Fix the issues above before committing.
```

**Fix the issues**, then commit again:

```bash
# Fix the code
vim src/main.rs

# Try again
git add src/main.rs
git commit -m "fix: Remove unused variable"
# Now passes ✅
```

### Bypassing the Hook (NOT RECOMMENDED)

You can bypass the hook with `--no-verify`:

```bash
git commit --no-verify -m "bypass hook"
```

**DO NOT DO THIS** unless you have a very good reason (e.g., emergency hotfix, CI is down).

Why?
- **Every commit should be working code**
- Broken commits make `git bisect` useless
- Other developers may check out broken code
- CI will catch it anyway (waste of time)

**Rule:** If the hook fails, fix the code. Don't bypass the hook.

## How It Works

### TigerStyle: Safety > Performance > DX

The hook is designed with safety as the top priority:

1. **Fail fast** - Run fast checks first (formatting, clippy)
2. **Comprehensive** - Run full test suite before commit
3. **Clear output** - Show exactly what failed and how to fix it
4. **Cannot be bypassed by agents** - Runs at git level

### Hook Script Location

Hooks are **not tracked by git** in `.git/hooks/` directory (git limitation).

We work around this by:
1. Storing hooks in **tracked** `tools/hooks/` directory
2. Providing `install-hooks.sh` script to copy them to `.git/hooks/`
3. New contributors run install script after cloning

### Constraint Checks

If `.kelpie-index/constraints/extracted.json` exists, the hook will:

1. Parse the JSON file
2. Find constraints with `"enforcement": "hard"`
3. Run the `verification.command` for each
4. Block commit if any fail

Example constraint:

```json
{
  "constraint": "No unwrap() in production code",
  "enforcement": "hard",
  "verification": {
    "command": "! grep -r 'unwrap()' --include='*.rs' crates/ | grep -v test"
  }
}
```

This ensures constraints are enforced at commit time, not just in code review.

## Integration with Other Hard Controls

Git hooks are the **last line of defense** before code enters the repository.

### Control Layers

```
┌─────────────────────────────────────────────────────┐
│  RLM Skills (Soft Controls)                         │
│  • Guide agents to verify before completion         │
│  • Can be ignored                                   │
├─────────────────────────────────────────────────────┤
│  MCP Tool Gates (Hard Controls)                     │
│  • state_task_complete() requires proof             │
│  • index_query() warns if indexes stale             │
│  • Cannot be bypassed by agents                     │
├─────────────────────────────────────────────────────┤
│  Git Pre-Commit Hook (Hard Floor)                   │
│  • Blocks commits if tests fail                     │
│  • Enforces constraints                             │
│  • Runs regardless of agent behavior                │
├─────────────────────────────────────────────────────┤
│  CI (Final Safety Net)                              │
│  • Runs on pull requests                            │
│  • Catches what hooks miss                          │
│  • Blocks merge if tests fail                       │
└─────────────────────────────────────────────────────┘
```

Even if:
- Agent ignores skills (soft controls)
- Agent bypasses MCP tools (shouldn't be possible)
- Agent uses `--no-verify` (discouraged)

**CI will still catch it.**

### Example: Full Verification Flow

```
1. Agent starts task
   └─> RLM skill suggests: "Call mcp.verify_by_tests() before completion"

2. Agent completes task
   └─> MCP tool requires: "state_task_complete() needs proof parameter"

3. Agent commits changes
   └─> Git hook enforces: "cargo test must pass"

4. Agent pushes to GitHub
   └─> CI enforces: "All checks must pass before merge"
```

## Debugging Hook Issues

### Hook Not Running

```bash
# Check if hook exists
ls -lh .git/hooks/pre-commit

# Check if executable
# Should show: -rwxr-xr-x (x = executable)

# If not executable:
chmod +x .git/hooks/pre-commit
```

### Hook Failing Unexpectedly

```bash
# Run hook manually to see full output
.git/hooks/pre-commit

# Check specific command
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

### Constraint Checks Failing

```bash
# Check if constraints file exists
ls -lh .kelpie-index/constraints/extracted.json

# View constraints
cat .kelpie-index/constraints/extracted.json | jq '.'

# Test constraint command manually
# (Copy command from JSON, run it)
```

## Maintenance

### Updating Hooks

Hooks are tracked in `tools/hooks/`. To update:

1. Edit `tools/hooks/pre-commit`
2. Re-run `./tools/install-hooks.sh`
3. Commit changes to `tools/hooks/pre-commit`

All team members will get the updated hook when they pull and re-run install script.

### Adding New Constraints

1. Add constraint to `.kelpie-index/constraints/extracted.json`
2. Set `"enforcement": "hard"` for git hook enforcement
3. Provide `verification.command` that exits with code 0 if passes, non-zero if fails
4. Test: `git commit --allow-empty -m "test"`

Example:

```json
{
  "constraint": "All TODOs must have issue numbers",
  "enforcement": "hard",
  "verification": {
    "command": "! grep -r 'TODO' --include='*.rs' crates/ | grep -v 'TODO(#[0-9]*)'"
  }
}
```

This blocks commits with TODOs that don't reference GitHub issues.

## Philosophy

### Why Hard Controls?

Soft controls (prompts, skills) are **guidance**. Agents might:
- Misunderstand instructions
- Skip steps to save tokens
- Trust documentation instead of running tests
- Mark tasks complete without verification

Hard controls (hooks, MCP gates) **enforce** behavior:
- Agent **must** provide proof to mark task complete
- Agent **cannot** commit code that fails tests
- Agent **cannot** bypass verification (without `--no-verify`)

### Trust Model

```
✅ TRUSTED:
- Code that passes all checks
- Commits that pass pre-commit hook
- Tests that actually run

❌ UNTRUSTED:
- Agent claims without proof
- Documentation without verification
- Checkboxes in plan files
```

**Verification-first development:** If it didn't run, it didn't happen.

## Summary

- **Git hooks enforce hard controls** at commit time
- **Pre-commit hook runs** tests, clippy, formatting checks
- **Cannot be bypassed by agents** (except with --no-verify)
- **Install with** `./tools/install-hooks.sh`
- **Part of layered control architecture** (skills → MCP → hooks → CI)
- **Philosophy:** Trust execution, not claims

Every commit is working code. No exceptions.
