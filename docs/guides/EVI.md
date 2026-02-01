# Exploration & Verification Infrastructure (EVI)

Kelpie includes an **Exploration & Verification Infrastructure (EVI)** for AI agent-driven development. This provides structural indexes, MCP tools, and verification-first workflows.

## Quick Reference

```bash
# Build all indexes (Python indexer with tree-sitter)
cd kelpie-mcp && uv run --prerelease=allow python3 -c "
from mcp_kelpie.indexer import build_indexes
build_indexes('/path/to/kelpie', '.kelpie-index/structural')
"

# Run indexer tests
cd kelpie-mcp && uv run --prerelease=allow pytest tests/test_indexer.py -v

# Run MCP server tests (102 tests)
cd kelpie-mcp && uv run --prerelease=allow pytest tests/ -v
```

## Structural Indexes

Located in `.kelpie-index/structural/`:

| Index | Description | Query Example |
|-------|-------------|---------------|
| `symbols.json` | Functions, structs, traits, impls | Find all `pub async fn` |
| `dependencies.json` | Crate dependency graph | Which crates depend on kelpie-core? |
| `tests.json` | All tests with topics and commands | Find tests for "storage" |
| `modules.json` | Module hierarchy per crate | What modules exist in kelpie-server? |

## MCP Server (VDE-Aligned Python)

The MCP server (`kelpie-mcp/`) provides 37 tools for AI agent development, built with VDE (Verification-Driven Exploration) architecture.

**Architecture:**
- **Single Python server** - All tools in one MCP server
- **tree-sitter indexing** - Fast, accurate Rust parsing for structural indexes
- **AgentFS integration** - Persistent state via `agentfs-sdk`
- **Sandboxed execution** - RLM REPL with RestrictedPython

**Tool categories (37 tools):**
- **REPL (7)** - `repl_load`, `repl_exec`, `repl_query`, `repl_state`, `repl_clear`, `repl_sub_llm`, `repl_map_reduce`
- **VFS/AgentFS (18)** - `vfs_init`, `vfs_fact_*`, `vfs_invariant_*`, `vfs_tool_*`, `vfs_spec_*`, `vfs_cache_*`, `vfs_export`
- **Index (6)** - `index_symbols`, `index_tests`, `index_modules`, `index_deps`, `index_status`, `index_refresh`
- **Examination (6)** - `exam_start`, `exam_record`, `exam_status`, `exam_complete`, `exam_export`, `issue_list`

**Running the server:**
```bash
cd kelpie-mcp
KELPIE_CODEBASE_PATH=/path/to/kelpie uv run --prerelease=allow mcp-kelpie
```

## Hard Controls

The infrastructure enforces verification-first development:

1. **Pre-commit hooks** - Tests, clippy, and formatting must pass
2. **Index freshness gates** - Stale indexes trigger warnings
3. **Completion verification** - `state_task_complete` requires proof
4. **Audit trail** - All MCP operations logged to `.agentfs/agent.db`

## AgentFS Storage

State is stored using [Turso AgentFS](https://github.com/tursodatabase/agentfs) Python SDK:

```bash
# Namespaced keys (VFS pattern)
session:{id}     # Verification session
fact:{id}        # Verified facts with evidence
invariant:{name} # Invariant verification status
tool:{id}        # Tool call tracking

# Storage location
.agentfs/agentfs-{session_id}.db  # SQLite database per session
```

The `agentfs-sdk` Python package handles all persistence. State survives across MCP restarts.

## Verification-First Development Principles

**Core Principle**: Trust execution, not documentation. Verify before claiming complete.

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

### Task Workflow
For any non-trivial task:
1. **Load constraints** - Read `.vision/CONSTRAINTS.md` (non-negotiable rules)
2. **Query indexes** - Use `index_symbols`, `index_modules` to understand scope
3. **Create plan** - Save to `.progress/NNN_YYYYMMDD_task-name.md`
4. **Execute phases** - Verify each by running tests, not reading docs
5. **Final verification** - `cargo test`, `cargo clippy`, `cargo fmt`

### Verification Workflow
When asked "Is X implemented?" or "Does Y work?":
1. **Find tests** - Search for relevant test files
2. **Run tests** - Execute and capture output
3. **Report results** - What you OBSERVED, not what docs claim

```bash
# Example: Verifying snapshot feature
cargo test snapshot  # Run relevant tests
# Report: "Ran 5 snapshot tests, 4 passed, 1 failed (restore_concurrent)"
```

### Exploration Workflow
Start broad, narrow down:
1. **Modules** - `cargo metadata` to see crate structure
2. **Dependencies** - `index_deps` to understand relationships
3. **Symbols** - `grep` for specific implementations
4. **Code reading** - Read the actual implementation
5. **Test verification** - Run tests to confirm understanding

### Handoff Protocol
When taking over from another agent:
1. **NEVER trust checkboxes** - Re-verify completed phases
2. **Run the tests** - See if claimed work actually passes
3. **Check for regressions** - Code may have changed since completion
4. **Document findings** - Update plan with actual verification status

### Slop Hunt
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

## Test Coverage

| Component | Tests | Command |
|-----------|-------|---------|
| AgentFS | 13 | `cd kelpie-mcp && uv run --prerelease=allow pytest tests/test_agentfs.py -v` |
| Indexer (Python) | 21 | `cd kelpie-mcp && uv run --prerelease=allow pytest tests/test_indexer.py -v` |
| RLM Environment | 36 | `cd kelpie-mcp && uv run --prerelease=allow pytest tests/test_rlm.py -v` |
| MCP Tools | 32 | `cd kelpie-mcp && uv run --prerelease=allow pytest tests/test_tools.py -v` |
| **Total** | **102** | `cd kelpie-mcp && uv run --prerelease=allow pytest tests/ -v` |

## Thorough Examination System

The examination tools enforce thoroughness before answering questions about the codebase. Use them for:
- **Full codebase mapping** - Build complete understanding of all components
- **Scoped thorough answers** - Examine all relevant components before answering

**Workflow:**
```
1. exam_start(task, scope)     # Define what to examine (["all"] or specific components)
2. exam_record(component, ...) # Record findings for EACH component
3. exam_status()               # Check progress (examined vs remaining)
4. exam_complete()             # GATE: returns can_answer=true only if ALL examined
5. exam_export()               # Generate MAP.md and ISSUES.md
6. issue_list(filter)          # Query issues by component or severity
```

**The Key Rule:** Do NOT answer questions until `exam_complete()` returns `can_answer: true`.

**Output Files (after exam_export):**
- `.kelpie-index/understanding/MAP.md` - Codebase map with all components
- `.kelpie-index/understanding/ISSUES.md` - All issues found by severity
- `.kelpie-index/understanding/components/*.md` - Per-component details

## Skills (`.claude/skills/`)

Project-specific skills that extend Claude's capabilities:

| Skill | Trigger | Purpose |
|-------|---------|---------|
| `codebase-map` | "map the codebase", "understand the project" | Full codebase examination workflow |
| `thorough-answer` | "how does X work?", complex questions | Scoped examination before answering |

**To use a skill:** Reference it by name or trigger phrase. The skill provides step-by-step guidance.

**Example - Using codebase-map:**
```
User: "I need to understand this codebase"
Claude: [Uses codebase-map skill]
1. exam_start(task="Build codebase map", scope=["all"])
2. For each component: read code, exam_record(...)
3. exam_complete() -> can_answer: true
4. exam_export() -> generates MAP.md, ISSUES.md
5. Present summary to user
```
