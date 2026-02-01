# Kelpie MCP Server

Single Python MCP server for Kelpie Repo OS, aligned with VDE (Verification-Driven Exploration) architecture.

## Architecture

This is a **single MCP server** combining:
- **RLM (Recursive Language Models)** - Context as variables, not tokens
- **AgentFS** - Persistent state across sessions (Turso AgentFS SDK)
- **Indexer** - Structural code analysis (tree-sitter)
- **Verification** - CLI execution tracking
- **DST** - Deterministic Simulation Testing coverage
- **Issues** - Persistent issue tracking

Based on QuickHouse VDE implementation (see `.progress/VDE.md`).

## Why Python?

1. **RLM requires Python** - Native REPL, dynamic execution, sub-LLM calls
2. **AgentFS SDK available** - Official Turso Python SDK
3. **State integration** - RLM + AgentFS in same process for tool tracking
4. **VDE proven** - QuickHouse uses Python MCP successfully

## Installation

```bash
cd kelpie-mcp

# Install with uv (recommended)
uv sync --prerelease=allow

# Or with pip
pip install -e ".[dev]"
```

## Usage

### Start MCP Server

```bash
# Set codebase path and run
KELPIE_CODEBASE_PATH=/path/to/kelpie uv run --prerelease=allow mcp-kelpie
```

### Run Tests

```bash
# All tests (92 tests)
uv run --prerelease=allow pytest tests/ -v

# Specific test file
uv run --prerelease=allow pytest tests/test_indexer.py -v
uv run --prerelease=allow pytest tests/test_rlm.py -v
uv run --prerelease=allow pytest tests/test_tools.py -v
```

## Tool Categories (33 Tools)

### REPL Tools (5 tools)
- `repl_load` - Load files into server variable by glob pattern
- `repl_exec` - Execute Python code on loaded variables
- `repl_query` - Evaluate expression and return result
- `repl_state` - Show current variable names/sizes
- `repl_clear` - Clear variables to free memory

### VFS/AgentFS Tools (11 tools)
- `vfs_init` - Initialize verification session
- `vfs_fact_add` - Record a verified fact with evidence
- `vfs_fact_check` - Check if a claim has been verified
- `vfs_fact_list` - List all verified facts
- `vfs_invariant_verify` - Mark an invariant as verified
- `vfs_invariant_status` - Check invariant verification status
- `vfs_status` - Get session status
- `vfs_tool_start` - Start tracking a tool call
- `vfs_tool_success` - Mark tool call as successful
- `vfs_tool_error` - Mark tool call as failed
- `vfs_tool_list` - List all tool calls with timing

### Index Tools (6 tools)
- `index_symbols` - Find symbols matching a pattern
- `index_tests` - Find tests by name pattern or crate
- `index_modules` - Get module hierarchy information
- `index_deps` - Get dependency graph information
- `index_status` - Get status of all indexes
- `index_refresh` - Rebuild indexes

### Verification Tools (4 tools)
- `verify_claim` - Verify a claim by executing a command
- `verify_all_tests` - Run all tests (cargo test --all)
- `verify_clippy` - Run Rust linter (cargo clippy)
- `verify_fmt` - Check code formatting (cargo fmt --check)

### DST Tools (3 tools)
- `dst_coverage_check` - Check DST coverage for critical paths
- `dst_gaps_report` - Generate report of DST coverage gaps
- `harness_check` - Check if DST harness supports required fault types

### Codebase Tools (4 tools)
- `codebase_grep` - Search for pattern in codebase files
- `codebase_peek` - Peek at first N lines of a file
- `codebase_read_section` - Read a section of a file by line range
- `codebase_list_files` - List files matching a glob pattern

## Architecture

```
mcp_kelpie/
├── server.py              # Main MCP server
├── rlm/                   # RLM implementation
│   ├── repl.py            # Python REPL with state
│   ├── executor.py        # Safe code execution
│   └── llm_query.py       # Recursive LLM calls
├── agentfs/               # AgentFS wrapper
│   ├── wrapper.py         # VerificationFS
│   └── session.py         # Session management
├── indexer/               # Structural indexing
│   ├── symbols.py         # Symbol extraction
│   ├── dependencies.py    # Dependency graph
│   ├── tests.py           # Test discovery
│   └── modules.py         # Module hierarchy
└── tools/                 # Tool implementations
    ├── verify.py          # Verification
    ├── dst.py             # DST coverage
    ├── codebase.py        # Codebase operations
    ├── issues.py          # Issue tracking
    └── constraints.py     # Constraint checking
```

## Development

### Running Tests

```bash
# All tests
pytest

# Specific test file
pytest tests/test_rlm.py

# With coverage
pytest --cov=mcp_kelpie
```

### Code Formatting

```bash
# Format code
black mcp_kelpie/

# Lint
ruff check mcp_kelpie/
```

## Migration Status

This replaces:
- ❌ `tools/mcp-kelpie/` (TypeScript) → Python
- ❌ `tools/kelpie-indexer/` (Rust) → Python tree-sitter
- ❌ `tools/rlm-env/` (Python) → Integrated into MCP

See `.progress/VDE_CONSOLIDATION_PLAN.md` for details.

## References

- VDE Paper: `.progress/VDE.md`
- RLM Paper: https://arxiv.org/html/2512.24601v1
- AgentFS: https://docs.turso.tech/agentfs/introduction
- MCP Protocol: https://modelcontextprotocol.io
