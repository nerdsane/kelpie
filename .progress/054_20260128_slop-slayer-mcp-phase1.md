# slop-slayer-mcp: Phase 1 Implementation

**Status**: COMPLETE
**Date**: 2026-01-28
**Location**: `/Users/seshendranalla/Development/slop-slayer-mcp/`

## Summary

Implemented Phase 1 of slop-slayer-mcp - a universal MCP server for codebase health assessment and enforcement. This phase establishes the core infrastructure with RLM, indexing, and SQLite-based issue registry.

## What Was Done

### 1. Project Structure Created

```
slop-slayer-mcp/
├── src/slop_slayer/
│   ├── __init__.py           # Package init
│   ├── server.py             # MCP server with 10 tools
│   ├── cli.py                # CLI entry point
│   │
│   ├── registry/             # SQLite issue tracking
│   │   ├── __init__.py
│   │   ├── db.py             # Database operations (CRUD)
│   │   ├── models.py         # Issue, Scan, ChainComponent models
│   │   └── recurrence.py     # Recurrence detection logic
│   │
│   ├── rlm/                  # Ported from kelpie-mcp
│   │   ├── __init__.py
│   │   ├── repl.py           # RestrictedPython sandbox
│   │   ├── llm.py            # SubLLM client (Anthropic)
│   │   ├── context.py        # Codebase context
│   │   └── types.py          # Data types
│   │
│   ├── indexer/              # Ported from kelpie-mcp
│   │   ├── __init__.py
│   │   ├── base.py           # Indexer class
│   │   ├── rust.py           # Rust parser (tree-sitter)
│   │   └── types.py          # Index data types
│   │
│   ├── detectors/            # Framework only
│   │   ├── __init__.py
│   │   └── base.py           # Detector interface
│   │
│   ├── chain/                # Stub
│   ├── provenance/           # Stub
│   └── tools/                # Stub
│
├── tests/
│   ├── test_registry.py      # 10 tests
│   ├── test_rlm.py           # 17 tests
│   ├── test_indexer.py       # 18 tests
│   └── test_detectors.py     # 5 tests
│
├── pyproject.toml
└── README.md
```

### 2. Components Implemented

| Component | Status | Tests | Description |
|-----------|--------|-------|-------------|
| RLM | ✅ Complete | 17 | Ported from kelpie-mcp, uses SLOP_* env vars |
| Indexer | ✅ Complete | 18 | Ported from kelpie-mcp, builds symbols/tests/modules |
| Registry | ✅ Complete | 10 | New SQLite-based issue tracking |
| Recurrence | ✅ Complete | 0 | Logic for detecting recurring issues |
| Detector Base | ✅ Complete | 5 | Interface for detection plugins |
| MCP Server | ✅ Complete | 0 | 10 tools exposed |

### 3. MCP Tools Implemented (10 of 42)

| Tool | Category | Description |
|------|----------|-------------|
| `repl_load` | RLM | Load files into server variable |
| `repl_exec` | RLM | Execute code with sub_llm() |
| `repl_sub_llm` | RLM | Analyze variable with sub-LLM |
| `repl_state` | RLM | Show loaded variables |
| `repl_clear` | RLM | Clear variables |
| `index_symbols` | Indexing | Query symbol index |
| `index_tests` | Indexing | Query test index |
| `index_modules` | Indexing | Query module index |
| `index_refresh` | Indexing | Refresh indexes |
| `slop_status` | Monitoring | Quick health metrics |

### 4. SQLite Schema

```sql
-- Issues table with full lifecycle tracking
CREATE TABLE issues (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    severity TEXT NOT NULL,
    status TEXT NOT NULL,
    location TEXT NOT NULL,      -- JSON array
    evidence TEXT NOT NULL,
    first_detected INTEGER,
    last_seen INTEGER,
    recurrence_count INTEGER,
    fix_pr TEXT,
    fix_author TEXT,
    verification_evidence TEXT,
    root_cause TEXT,
    cluster_id TEXT,
    introduced_by TEXT,
    introduced_pr TEXT,
    metadata TEXT                -- JSON
);

-- Chain components for verification tracking
CREATE TABLE chain_components (...);

-- Scan history
CREATE TABLE scans (...);

-- Watch list for recurrence detection
CREATE TABLE watch_list (...);
```

## What to Try

### Works Now

1. **Run tests**:
   ```bash
   cd /Users/seshendranalla/Development/slop-slayer-mcp
   uv run pytest tests/ -v
   ```

2. **Build indexes on a Rust codebase**:
   ```python
   from slop_slayer.indexer import build_indexes
   result = build_indexes("/path/to/rust/codebase", "/path/to/output")
   ```

3. **Use REPL environment**:
   ```python
   from slop_slayer.rlm import CodebaseContext, REPLEnvironment, SubLLM

   context = CodebaseContext("/path/to/codebase")
   sub_llm = SubLLM()  # Requires ANTHROPIC_API_KEY
   repl = REPLEnvironment(context, sub_llm)

   repl.load("**/*.rs", "code")
   result = repl.execute("result = len(code)")
   ```

4. **Use issue registry**:
   ```python
   from slop_slayer.registry import Database, IssueSeverity

   db = Database("/path/to/.slop/issues.db")
   issue = db.create_issue(
       type="dead_code",
       severity=IssueSeverity.HIGH,
       location=["src/foo.rs"],
       evidence="Function never called",
       timestamp=int(time.time()),
   )
   ```

### Doesn't Work Yet

1. **Detection plugins** - Only base interface exists, no actual detectors
2. **Scanning tools** - `slop_scan`, `slop_triage`, etc. not implemented
3. **Chain tracking** - `slop_chain_status` not implemented
4. **Git provenance** - No git integration yet

## Decisions Made

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Persistence | Raw SQLite | Full schema control, no external deps |
| Sub-LLM Model | Haiku (default) | Fast/cheap, `SLOP_SUB_LLM_MODEL` env var |
| Index Storage | `.slop-index/` | Separate from issue DB in `.slop/` |
| Project Location | Separate repo | Independent versioning, publishable |

## Next Steps (Phase 2)

1. **Implement first detectors**:
   - `dead_code` - Index + RLM analysis
   - `fake_dst` - For Kelpie DST tests

2. **Core scanning tools**:
   - `slop_scan` - Full detection
   - `slop_diff_scan` - Incremental

3. **Triage tools**:
   - `slop_triage` - Prioritized list
   - `slop_hunt` - Deep dive investigation

4. **Test on Kelpie codebase**

## Verification

```
$ uv run pytest tests/ -v
50 passed in 0.23s
```

All 50 tests pass:
- Registry: 10 tests
- RLM: 17 tests
- Indexer: 18 tests
- Detectors: 5 tests
