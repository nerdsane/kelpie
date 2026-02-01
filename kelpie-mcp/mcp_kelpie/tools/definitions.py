"""
MCP Tool definitions for Kelpie Repo OS.

All tools are defined here with their schemas.
Handlers are implemented in handlers.py.
"""

from typing import Any

# Tool schema type
Tool = dict[str, Any]


# ==================== RLM Tools ====================

REPL_TOOLS: list[Tool] = [
    {
        "name": "repl_load",
        "description": "Load files into server variable by glob pattern. Files become data in variables, not tokens in context.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern (e.g., '**/*.rs')"},
                "var_name": {"type": "string", "description": "Variable name to store files"},
            },
            "required": ["pattern", "var_name"],
        },
    },
    {
        "name": "repl_exec",
        "description": "Execute Python code on loaded variables. Use 'result = ...' to return values.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "code": {"type": "string", "description": "Python code to execute"},
            },
            "required": ["code"],
        },
    },
    {
        "name": "repl_query",
        "description": "Evaluate a Python expression and return the result.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "expression": {"type": "string", "description": "Python expression to evaluate"},
            },
            "required": ["expression"],
        },
    },
    {
        "name": "repl_state",
        "description": "Get current REPL state (loaded variables, memory usage).",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "repl_clear",
        "description": "Clear loaded variables to free memory.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "var_name": {"type": "string", "description": "Variable to clear (optional, clears all if not specified)"},
            },
        },
    },
    {
        "name": "repl_sub_llm",
        "description": "Have a sub-LLM analyze a loaded variable. The sub-model analyzes server-side without using primary model's context. Model configurable via KELPIE_SUB_LLM_MODEL env var.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "var_name": {"type": "string", "description": "Name of the loaded variable to analyze"},
                "query": {"type": "string", "description": "Question to ask about the variable content"},
                "selector": {"type": "string", "description": "Python expression to filter/transform context before sending to sub-LLM (e.g., 'var[\"limits\"]' or '{k:v for k,v in var.items() if \"MAX\" in v}')"},
                "model": {"type": "string", "description": "Model override (optional, defaults to KELPIE_SUB_LLM_MODEL or claude-haiku-4-5-20250514)"},
            },
            "required": ["var_name", "query"],
        },
    },
    {
        "name": "repl_map_reduce",
        "description": "Map-reduce pattern for partitioned codebase analysis. Load partitions, apply query to each, aggregate results.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Question to ask for each partition"},
                "partitions_var": {"type": "string", "description": "Variable name containing partitions (list of ModuleContext or dict)"},
            },
            "required": ["query", "partitions_var"],
        },
    },
]


# ==================== AgentFS/VFS Tools ====================

AGENTFS_TOOLS: list[Tool] = [
    {
        "name": "vfs_init",
        "description": "Initialize or resume a verification session.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Task description"},
                "session_id": {"type": "string", "description": "Existing session ID (optional)"},
            },
            "required": ["task"],
        },
    },
    {
        "name": "vfs_fact_add",
        "description": "Record a verified fact with evidence.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "claim": {"type": "string", "description": "The claim being verified"},
                "evidence": {"type": "string", "description": "Evidence supporting the claim"},
                "source": {"type": "string", "description": "Source of verification (e.g., 'dst', 'code_review')"},
                "command": {"type": "string", "description": "Command used (optional)"},
            },
            "required": ["claim", "evidence", "source"],
        },
    },
    {
        "name": "vfs_fact_check",
        "description": "Check if a claim has been verified.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "claim_pattern": {"type": "string", "description": "Pattern to search for in claims"},
            },
            "required": ["claim_pattern"],
        },
    },
    {
        "name": "vfs_fact_list",
        "description": "List all verified facts in chronological order.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "vfs_invariant_verify",
        "description": "Mark an invariant as verified.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Invariant name"},
                "component": {"type": "string", "description": "Component name"},
                "method": {"type": "string", "description": "Verification method (dst, stateright, kani, manual)"},
                "evidence": {"type": "string", "description": "Evidence of verification"},
            },
            "required": ["name", "component", "method"],
        },
    },
    {
        "name": "vfs_invariant_status",
        "description": "Check invariant verification status for a component.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "component": {"type": "string", "description": "Component name"},
            },
            "required": ["component"],
        },
    },
    {
        "name": "vfs_status",
        "description": "Get current verification session status.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "vfs_tool_start",
        "description": "Start tracking a tool call.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Tool name"},
                "args": {"type": "object", "description": "Tool arguments"},
            },
            "required": ["name", "args"],
        },
    },
    {
        "name": "vfs_tool_success",
        "description": "Mark tool call as successful.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "call_id": {"type": "integer", "description": "Call ID from vfs_tool_start"},
                "result": {"description": "Tool result"},
            },
            "required": ["call_id", "result"],
        },
    },
    {
        "name": "vfs_tool_error",
        "description": "Mark tool call as failed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "call_id": {"type": "integer", "description": "Call ID from vfs_tool_start"},
                "error": {"type": "string", "description": "Error message"},
            },
            "required": ["call_id", "error"],
        },
    },
    {
        "name": "vfs_tool_list",
        "description": "List all tool calls with timing.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    # ========== Spec Tracking ==========
    {
        "name": "vfs_spec_read",
        "description": "Record that a TLA+ spec was read. Tracks what specs you've studied.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Spec name (e.g., 'CompactionProtocol')"},
                "path": {"type": "string", "description": "Path to spec file"},
                "description": {"type": "string", "description": "Brief description (optional)"},
                "invariants": {"type": "string", "description": "Comma-separated list of invariant names (optional)"},
            },
            "required": ["name", "path"],
        },
    },
    {
        "name": "vfs_specs_list",
        "description": "List TLA+ specs read in session.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    # ========== Exploration Logging ==========
    {
        "name": "vfs_exploration_log",
        "description": "Log exploration action for audit trail.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type": "string", "description": "Action type (read, execute, query, analyze)"},
                "target": {"type": "string", "description": "Target of action (file path, query, etc.)"},
                "result": {"type": "string", "description": "Result summary (optional)"},
            },
            "required": ["action", "target"],
        },
    },
    # ========== Cache with TTL ==========
    {
        "name": "vfs_cache_get",
        "description": "Get cached value (respects TTL). Useful for expensive queries.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Cache key"},
            },
            "required": ["key"],
        },
    },
    {
        "name": "vfs_cache_set",
        "description": "Cache value with TTL. Useful for expensive query results.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Cache key"},
                "value": {"type": "string", "description": "Value to cache (JSON string)"},
                "ttl_minutes": {"type": "integer", "description": "Time to live in minutes (default 30)"},
            },
            "required": ["key", "value"],
        },
    },
    # ========== Export ==========
    {
        "name": "vfs_export",
        "description": "Export full session data as JSON for replay/analysis.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "vfs_explorations_list",
        "description": "List all exploration logs in chronological order. Complete audit trail of all exploration actions.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
]


# ==================== Index Tools ====================

INDEX_TOOLS: list[Tool] = [
    {
        "name": "index_symbols",
        "description": "Find symbols matching a pattern across the codebase.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Pattern to match (regex or simple string)"},
                "kind": {"type": "string", "description": "Symbol kind filter (function, struct, enum, trait, etc.)"},
            },
            "required": ["pattern"],
        },
    },
    {
        "name": "index_tests",
        "description": "Find tests by name pattern or crate.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Test name pattern (optional)"},
                "crate": {"type": "string", "description": "Crate filter (optional)"},
            },
        },
    },
    {
        "name": "index_modules",
        "description": "Get module hierarchy information.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "crate": {"type": "string", "description": "Crate filter (optional)"},
            },
        },
    },
    {
        "name": "index_deps",
        "description": "Get dependency graph information.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "crate": {"type": "string", "description": "Get dependencies for specific crate (optional)"},
            },
        },
    },
    {
        "name": "index_status",
        "description": "Get status of all indexes (freshness, file counts).",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "index_refresh",
        "description": "Rebuild indexes using Python indexer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "description": "Which index to rebuild (symbols, tests, modules, dependencies, all)",
                    "enum": ["symbols", "tests", "modules", "dependencies", "all"],
                },
            },
        },
    },
]


# ==================== Verification Tools ====================
# NOTE: Verification tools were removed - redundant with Claude's Bash tool.
# Use Bash directly:
#   cargo test --all
#   cargo clippy --all-targets
#   cargo fmt --check

VERIFICATION_TOOLS: list[Tool] = []


# ==================== DST Tools ====================
# NOTE: DST analysis tools were removed in favor of using RLM/REPL primitives.
# Use repl_load + repl_sub_llm to analyze DST coverage, gaps, and harness capabilities.
# Example:
#   repl_load(pattern="**/*_dst.rs", var_name="dst_tests")
#   repl_sub_llm(var_name="dst_tests", query="Analyze DST coverage and harness usage")

DST_TOOLS: list[Tool] = []


# ==================== Codebase Tools ====================
# NOTE: Codebase tools were removed - redundant with Claude's built-in tools and RLM primitives.
# Use instead:
#   Grep tool for searching (codebase_grep)
#   Read tool for reading files (codebase_peek, codebase_read_section, codebase_read_context)
#   Glob tool for listing files (codebase_list_files)
#   repl_load for loading files into variables (codebase_get_module, codebase_partition_by_crate)
#   index_* tools for structural queries (codebase_list_modules, codebase_list_tests, codebase_get_index)

CODEBASE_TOOLS: list[Tool] = []


# ==================== Examination Tools ====================
# Thorough examination system for building codebase understanding and surfacing issues.
# Used for both full codebase mapping and scoped thorough answers.
#
# Workflow:
#   1. exam_start(task, scope) - Define what needs to be examined
#   2. exam_record(component, ...) - Record findings for each component
#   3. exam_status() - Check progress (examined vs remaining)
#   4. exam_complete() - Gate: returns True only if all examined
#   5. exam_export() - Generate human-readable MAP.md, ISSUES.md
#   6. issue_list() - Query issues by component or severity

EXAMINATION_TOOLS: list[Tool] = [
    {
        "name": "exam_start",
        "description": "Start a thorough examination. Scope can be 'all' for full codebase map, or a list of specific components for scoped questions.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "What you're trying to understand (e.g., 'Build codebase map', 'How does storage work?')"},
                "scope": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Components to examine. Use ['all'] for full codebase, or specific components like ['kelpie-storage', 'kelpie-core']",
                },
            },
            "required": ["task", "scope"],
        },
    },
    {
        "name": "exam_record",
        "description": "Record findings for a component during examination. Captures understanding and surfaces issues.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "component": {"type": "string", "description": "Component name (e.g., 'kelpie-storage')"},
                "summary": {"type": "string", "description": "Brief summary of what this component does"},
                "details": {"type": "string", "description": "Detailed explanation of how it works"},
                "connections": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of related components this connects to",
                },
                "issues": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "severity": {"type": "string", "enum": ["critical", "high", "medium", "low"]},
                            "description": {"type": "string"},
                            "evidence": {"type": "string"},
                        },
                        "required": ["severity", "description"],
                    },
                    "description": "Issues found in this component",
                },
            },
            "required": ["component", "summary"],
        },
    },
    {
        "name": "exam_status",
        "description": "Get examination progress. Shows what's been examined, what remains, and overall progress.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "exam_complete",
        "description": "Check if examination is complete. Returns True only if ALL components in scope have been examined. Use this before answering questions to ensure thoroughness.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    },
    {
        "name": "exam_export",
        "description": "Export examination findings to human-readable markdown. Creates MAP.md (codebase understanding) and ISSUES.md (all issues found) in .kelpie-index/understanding/.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "include_details": {"type": "boolean", "description": "Include detailed explanations (default: true)"},
            },
        },
    },
    {
        "name": "issue_list",
        "description": "List issues found during examination. Can filter by component or severity.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "component": {"type": "string", "description": "Filter by component (optional)"},
                "severity": {"type": "string", "enum": ["critical", "high", "medium", "low"], "description": "Filter by severity (optional)"},
            },
        },
    },
]


# All tools combined
ALL_TOOLS: list[Tool] = [
    *REPL_TOOLS,
    *AGENTFS_TOOLS,
    *INDEX_TOOLS,
    *VERIFICATION_TOOLS,
    *DST_TOOLS,
    *CODEBASE_TOOLS,
    *EXAMINATION_TOOLS,
]
