# untracked-files

**Examined:** 2026-01-24T21:07:55.836560+00:00

## Summary

Many untracked files but most are gitignored, some should be tracked

## Connections

- precommit-hooks

## Details

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

## Issues

### [LOW] Many untracked files should be committed

**Evidence:** git status shows 17+ untracked files/directories including source code (fdb.rs, invariants.rs, wal.rs), tests, docs, and infrastructure
