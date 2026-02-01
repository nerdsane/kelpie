# Kelpie Index Directory

Auto-generated indexes for the Kelpie codebase. These files enable fast lookup and semantic understanding without scanning the entire codebase.

## Directory Structure

```
.kelpie-index/
├── structural/           # Deterministic, tool-generated indexes
│   ├── symbols.json      # Functions, types, traits (tree-sitter/rust-analyzer)
│   ├── dependencies.json # Crate dependency graph (cargo metadata)
│   ├── tests.json        # All tests with categorization
│   └── modules.json      # Module hierarchy
├── semantic/             # LLM-generated, for navigation (not source of truth)
│   ├── summaries/        # Per-module summaries
│   └── embeddings/       # Vector embeddings (optional)
├── constraints/          # Extracted from .vision/CONSTRAINTS.md
│   └── extracted.json    # Structured constraints with verification commands
└── meta/
    ├── freshness.json    # Git SHA, file hashes for staleness detection
    └── build_log.json    # When indexes were last built
```

## Usage

### Building Indexes

```bash
# Full rebuild (Phase 7 - not yet implemented)
./tools/kelpie-indexer --full

# Incremental rebuild (Phase 7 - not yet implemented)
./tools/kelpie-indexer --incremental path/to/changed.rs
```

### Querying Indexes

Use the MCP server (Phase 4):
```bash
# Via MCP tools (not yet implemented)
mcp.index_symbols("ActorId")
mcp.index_tests("streaming")
mcp.index_constraints()
```

## Freshness

Indexes track the git SHA and file hashes at build time. Before returning results, the system checks if files have changed and can auto-rebuild or warn about staleness.

## Git Tracking

- **structural/** - Git-tracked (deterministic, useful for review)
- **semantic/** - Git-ignored (LLM-generated, may vary)
- **meta/** - Git-tracked (freshness tracking is important)

## Source of Truth

**IMPORTANT:** These indexes are derived data. The actual code in `crates/` is the source of truth. When in doubt, verify by execution (run tests, run clippy), not by reading these indexes.
