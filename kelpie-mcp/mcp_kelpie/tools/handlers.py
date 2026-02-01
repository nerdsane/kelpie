"""
Tool handlers for Kelpie MCP Server.

Implements the actual logic for each tool defined in definitions.py.
"""

import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ..rlm import CodebaseContext, REPLEnvironment, SubLLM
from ..indexer import Indexer, build_indexes


def _utcnow() -> str:
    """Get current UTC time as ISO string."""
    return datetime.now(timezone.utc).isoformat()


def _run_command(command: str, cwd: Path, timeout_seconds: int = 120) -> dict[str, Any]:
    """Run a shell command and capture output.

    Args:
        command: Command to run
        cwd: Working directory
        timeout_seconds: Timeout in seconds

    Returns:
        Dict with success, output, error
    """
    try:
        result = subprocess.run(
            command,
            shell=True,
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
        )
        return {
            "success": result.returncode == 0,
            "output": result.stdout,
            "error": result.stderr if result.returncode != 0 else None,
        }
    except subprocess.TimeoutExpired:
        return {
            "success": False,
            "output": "",
            "error": f"Command timed out after {timeout_seconds}s",
        }
    except Exception as e:
        return {
            "success": False,
            "output": "",
            "error": str(e),
        }


class ToolHandlers:
    """Handlers for all MCP tools.

    This class manages tool state and provides handler methods for each tool.
    """

    def __init__(self, codebase_path: Path):
        """Initialize tool handlers.

        Args:
            codebase_path: Path to codebase root
        """
        self.codebase_path = codebase_path.resolve()
        self.indexes_path = self.codebase_path / ".kelpie-index"
        self.agentfs_path = self.codebase_path / ".agentfs"

        # Initialize RLM components
        self._codebase_context = CodebaseContext(str(self.codebase_path))
        self._sub_llm = SubLLM()  # Sub-LLM for analyzing loaded variables
        # TRUE RLM: Pass sub_llm to REPL so sub_llm() is available inside repl_exec code
        self._repl_env = REPLEnvironment(self._codebase_context, sub_llm=self._sub_llm)

        # AgentFS session (initialized lazily)
        self._vfs_session = None
        self._session_manager = None

    async def handle_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        """Route tool call to appropriate handler.

        Args:
            name: Tool name
            arguments: Tool arguments

        Returns:
            Tool result
        """
        handler = getattr(self, f"_handle_{name}", None)
        if handler is None:
            raise ValueError(f"Unknown tool: {name}")
        return await handler(arguments)

    # ==================== RLM Tools ====================

    async def _handle_repl_load(self, args: dict[str, Any]) -> dict[str, Any]:
        """Load files into server variable by glob pattern."""
        pattern = args.get("pattern", "")
        var_name = args.get("var_name", "")

        result = self._repl_env.load(pattern, var_name)
        return {
            "success": "Error" not in result,
            "message": result,
            "variable": var_name,
        }

    async def _handle_repl_exec(self, args: dict[str, Any]) -> dict[str, Any]:
        """Execute Python code on loaded variables."""
        code = args.get("code", "")

        result = self._repl_env.execute(code)
        return {
            "success": result.success,
            "result": result.result,
            "error": result.error,
            "execution_log": result.execution_log[-10:] if result.execution_log else [],
        }

    async def _handle_repl_query(self, args: dict[str, Any]) -> dict[str, Any]:
        """Evaluate a Python expression."""
        expression = args.get("expression", "")

        result = self._repl_env.query(expression)
        return {
            "success": result.success,
            "result": result.result,
            "error": result.error,
        }

    async def _handle_repl_state(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get current REPL state."""
        return self._repl_env.state()

    async def _handle_repl_clear(self, args: dict[str, Any]) -> dict[str, Any]:
        """Clear loaded variables."""
        var_name = args.get("var_name")

        result = self._repl_env.clear(var_name)
        return {
            "success": True,
            "message": result,
        }

    async def _handle_repl_sub_llm(self, args: dict[str, Any]) -> dict[str, Any]:
        """Have a sub-LLM (Claude 3.5 Haiku) analyze a loaded variable.

        The sub-model analyzes server-side without using primary model's context.
        Supports selector to filter/transform context before sending to sub-LLM.
        Selector is executed in a sandboxed RestrictedPython environment.
        """
        from RestrictedPython import compile_restricted
        from RestrictedPython.Guards import safe_builtins, guarded_iter_unpack_sequence

        var_name = args.get("var_name", "")
        query = args.get("query", "")
        selector = args.get("selector", "")
        model = args.get("model")

        # Get the loaded variable
        variable = self._repl_env.get_variable(var_name)
        if variable is None:
            return {
                "success": False,
                "error": f"Variable not found: {var_name}. Use repl_load first to load files.",
            }

        # Apply selector to filter/transform context if provided
        context = variable.files  # dict[path, content]
        if selector:
            try:
                # Wrap selector in assignment for RestrictedPython
                # Transform 'var' references to 'context'
                selector_code = f"result = {selector.replace('var', 'context')}"

                # Compile with RestrictedPython
                byte_code = compile_restricted(selector_code, filename="<selector>", mode="exec")

                # Define safe getitem for dict/list access
                def _getitem_(obj, key):
                    return obj[key]

                def _write_(obj):
                    return obj

                # Build restricted globals with only safe operations
                restricted_globals = {
                    "__builtins__": safe_builtins,
                    "_getiter_": iter,
                    "_iter_unpack_sequence_": guarded_iter_unpack_sequence,
                    "_getitem_": _getitem_,
                    "_write_": _write_,
                    "_getattr_": getattr,
                    # Safe builtins for selector
                    "len": len,
                    "str": str,
                    "int": int,
                    "list": list,
                    "dict": dict,
                    "sorted": sorted,
                    "filter": filter,
                    "map": map,
                    "any": any,
                    "all": all,
                    # The context variable
                    "context": context,
                    "result": None,
                }

                # Execute in sandbox
                exec(byte_code, restricted_globals)
                context = restricted_globals.get("result", context)

            except SyntaxError as e:
                return {
                    "success": False,
                    "error": f"Selector syntax error: {e}",
                }
            except Exception as e:
                return {
                    "success": False,
                    "error": f"Selector error: {type(e).__name__}: {e}",
                }

        # Call sub-LLM to analyze the (possibly filtered) content
        result = await self._sub_llm.analyze_content(
            content=self._format_context(context),
            query=query,
            context_name=f"{var_name} (selector: {selector})" if selector else var_name,
            model=model,
        )
        return result.to_dict()

    def _format_context(self, context: Any) -> str:
        """Format context for sub-LLM analysis."""
        if isinstance(context, dict):
            return "\n\n".join(f"=== {k} ===\n{v}" for k, v in context.items())
        elif isinstance(context, list):
            return "\n".join(str(x) for x in context)
        else:
            return str(context)

    # ==================== AgentFS/VFS Tools ====================

    async def _handle_vfs_init(self, args: dict[str, Any]) -> dict[str, Any]:
        """Initialize or resume a verification session."""
        from ..agentfs import SessionManager

        task = args.get("task", "")
        session_id = args.get("session_id")

        if self._session_manager is None:
            self._session_manager = SessionManager(str(self.codebase_path))

        self._vfs_session = await self._session_manager.init_session(task, session_id)

        return {
            "success": True,
            "session_id": self._session_manager.get_session_id(),
            "task": task,
        }

    async def _handle_vfs_fact_add(self, args: dict[str, Any]) -> dict[str, Any]:
        """Record a verified fact with evidence."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        fact_id = await self._vfs_session.add_fact(
            claim=args.get("claim", ""),
            evidence=args.get("evidence", ""),
            source=args.get("source", ""),
            command=args.get("command"),
        )

        return {
            "success": True,
            "fact_id": fact_id,
        }

    async def _handle_vfs_fact_check(self, args: dict[str, Any]) -> dict[str, Any]:
        """Check if a claim has been verified."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        facts = await self._vfs_session.check_fact(args.get("claim_pattern", ""))

        return {
            "success": True,
            "facts": facts,
            "count": len(facts),
        }

    async def _handle_vfs_fact_list(self, args: dict[str, Any]) -> dict[str, Any]:
        """List all verified facts."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        facts = await self._vfs_session.list_facts()

        return {
            "success": True,
            "facts": facts,
            "count": len(facts),
        }

    async def _handle_vfs_invariant_verify(self, args: dict[str, Any]) -> dict[str, Any]:
        """Mark an invariant as verified."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        await self._vfs_session.verify_invariant(
            name=args.get("name", ""),
            component=args.get("component", ""),
            method=args.get("method", "manual"),
            evidence=args.get("evidence"),
        )

        return {"success": True}

    async def _handle_vfs_invariant_status(self, args: dict[str, Any]) -> dict[str, Any]:
        """Check invariant verification status."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        status = await self._vfs_session.invariant_status(args.get("component", ""))

        return {
            "success": True,
            **status,
        }

    async def _handle_vfs_status(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get current verification session status."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        status = await self._vfs_session.status()

        return {
            "success": True,
            **status,
        }

    async def _handle_vfs_tool_start(self, args: dict[str, Any]) -> dict[str, Any]:
        """Start tracking a tool call."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        call_id = await self._vfs_session.tool_start(
            name=args.get("name", ""),
            args=args.get("args", {}),
        )

        return {
            "success": True,
            "call_id": call_id,
        }

    async def _handle_vfs_tool_success(self, args: dict[str, Any]) -> dict[str, Any]:
        """Mark tool call as successful."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        await self._vfs_session.tool_success(
            call_id=args.get("call_id", 0),
            result=args.get("result"),
        )

        return {"success": True}

    async def _handle_vfs_tool_error(self, args: dict[str, Any]) -> dict[str, Any]:
        """Mark tool call as failed."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        await self._vfs_session.tool_error(
            call_id=args.get("call_id", 0),
            error=args.get("error", ""),
        )

        return {"success": True}

    async def _handle_vfs_tool_list(self, args: dict[str, Any]) -> dict[str, Any]:
        """List all tool calls."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        tools = await self._vfs_session.tool_list()

        return {
            "success": True,
            "tools": tools,
            "count": len(tools),
        }

    # ========== Spec Tracking ==========

    async def _handle_vfs_spec_read(self, args: dict[str, Any]) -> dict[str, Any]:
        """Record that a TLA+ spec was read."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        await self._vfs_session.spec_read(
            name=args.get("name", ""),
            path=args.get("path", ""),
            description=args.get("description"),
            invariants=args.get("invariants"),
        )

        return {
            "success": True,
            "message": f"Recorded: read {args.get('name', '')}",
        }

    async def _handle_vfs_specs_list(self, args: dict[str, Any]) -> dict[str, Any]:
        """List TLA+ specs read in session."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        specs = await self._vfs_session.list_specs()

        return {
            "success": True,
            "specs": specs,
            "count": len(specs),
        }

    # ========== Exploration Logging ==========

    async def _handle_vfs_exploration_log(self, args: dict[str, Any]) -> dict[str, Any]:
        """Log exploration action for audit trail."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        await self._vfs_session.exploration_log(
            action=args.get("action", ""),
            target=args.get("target", ""),
            result=args.get("result"),
        )

        return {
            "success": True,
            "message": f"Logged: {args.get('action', '')} {args.get('target', '')}",
        }

    # ========== Cache with TTL ==========

    async def _handle_vfs_cache_get(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get cached value (respects TTL)."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        key = args.get("key", "")
        value = await self._vfs_session.get_cached_query(key)

        if value is None:
            return {
                "success": True,
                "hit": False,
                "message": f"Cache miss: '{key}'",
            }

        return {
            "success": True,
            "hit": True,
            "value": value,
        }

    async def _handle_vfs_cache_set(self, args: dict[str, Any]) -> dict[str, Any]:
        """Cache value with TTL."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        key = args.get("key", "")
        value_str = args.get("value", "")
        ttl_minutes = args.get("ttl_minutes", 30)

        # Parse value as JSON if possible
        try:
            value = json.loads(value_str)
        except (json.JSONDecodeError, TypeError):
            value = {"raw": value_str}

        await self._vfs_session.cache_query(
            query=key,
            result=value,
            ttl_minutes=ttl_minutes,
            query_type="manual",
        )

        return {
            "success": True,
            "message": f"Cached: '{key}' (TTL: {ttl_minutes}m)",
        }

    # ========== Export ==========

    async def _handle_vfs_export(self, args: dict[str, Any]) -> dict[str, Any]:
        """Export full session data as JSON."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        data = await self._vfs_session.export()

        return {
            "success": True,
            "data": data,
        }

    async def _handle_vfs_explorations_list(self, args: dict[str, Any]) -> dict[str, Any]:
        """List all exploration logs."""
        if self._vfs_session is None:
            return {"success": False, "error": "No active VFS session. Call vfs_init first."}

        explorations = await self._vfs_session.list_explorations()

        return {
            "success": True,
            "explorations": explorations,
            "count": len(explorations),
        }

    # ==================== REPL Map-Reduce ====================

    async def _handle_repl_map_reduce(self, args: dict[str, Any]) -> dict[str, Any]:
        """Map-reduce pattern for partitioned analysis using parallel sub-LLM calls.

        This tool calls the sub-LLM (Claude 3.5 Haiku) in parallel for each partition,
        then aggregates the results. This enables cost-efficient analysis of large
        codebases without loading everything into the primary model's context.
        """
        import asyncio

        query = args.get("query", "")
        partitions_var = args.get("partitions_var", "")

        # Get the partitions variable
        partitions = self._repl_env.get_variable(partitions_var)
        if partitions is None:
            return {
                "success": False,
                "error": f"Variable not found: {partitions_var}. Load partitions first with repl_load or codebase_partition_by_crate.",
            }

        # Convert LoadedVariable to list if needed
        if hasattr(partitions, 'files'):
            # It's a LoadedVariable, convert to list of dicts
            partition_list = [{"name": k, "content": v} for k, v in partitions.files.items()]
        else:
            partition_list = partitions

        # Define async task for analyzing one partition
        async def analyze_partition(idx: int, partition: dict) -> dict:
            name = partition.get("name", f"partition_{idx}")
            content = partition.get("content", "")

            # Format content for sub-LLM
            if isinstance(content, dict):
                # Multiple files in partition
                formatted = "\n\n".join(f"=== {k} ===\n{v}" for k, v in content.items())
            else:
                formatted = str(content)

            # Call sub-LLM
            result = await self._sub_llm.analyze_content(
                content=formatted,
                query=query,
                context_name=f"partition: {name}",
            )

            return {
                "partition": idx,
                "partition_name": name,
                "success": result.success,
                "response": result.response if result.success else None,
                "error": result.error if not result.success else None,
                "input_tokens": result.input_tokens,
                "output_tokens": result.output_tokens,
            }

        # Run all partition analyses in parallel
        tasks = [analyze_partition(i, p) for i, p in enumerate(partition_list)]
        results = await asyncio.gather(*tasks, return_exceptions=True)

        # Process results (handle any exceptions)
        processed_results = []
        total_input_tokens = 0
        total_output_tokens = 0

        for i, result in enumerate(results):
            if isinstance(result, Exception):
                processed_results.append({
                    "partition": i,
                    "partition_name": partition_list[i].get("name", f"partition_{i}"),
                    "success": False,
                    "error": str(result),
                })
            else:
                processed_results.append(result)
                total_input_tokens += result.get("input_tokens", 0) or 0
                total_output_tokens += result.get("output_tokens", 0) or 0

        return {
            "success": True,
            "results": processed_results,
            "partition_count": len(partition_list),
            "total_input_tokens": total_input_tokens,
            "total_output_tokens": total_output_tokens,
        }

    # ==================== Index Tools ====================

    def _ensure_indexes_exist(self) -> tuple[bool, str | None]:
        """Ensure indexes exist, auto-building if needed.

        Returns:
            Tuple of (success, error_message). error_message is None on success.
        """
        symbols_path = self.indexes_path / "structural" / "symbols.json"
        if symbols_path.exists():
            return (True, None)

        # Auto-build indexes
        try:
            output_dir = self.indexes_path / "structural"
            indexer = Indexer(self.codebase_path)
            indexer.build_all(output_dir)
            return (True, None)
        except Exception as e:
            return (False, f"Auto-build failed: {type(e).__name__}: {str(e)[:200]}")

    async def _handle_index_symbols(self, args: dict[str, Any]) -> dict[str, Any]:
        """Find symbols matching a pattern."""
        pattern = args.get("pattern", "")
        kind = args.get("kind")

        # Auto-build indexes if missing
        success, error = self._ensure_indexes_exist()
        if not success:
            return {"success": False, "error": error or "Failed to build indexes. Check codebase path."}

        # Load symbols index
        symbols_path = self.indexes_path / "structural" / "symbols.json"
        if not symbols_path.exists():
            return {"success": False, "error": "Symbols index not found after rebuild attempt."}

        symbols_data = json.loads(symbols_path.read_text())
        matches = []

        regex = re.compile(pattern, re.IGNORECASE)

        # files is a dict: {filepath: {symbols: [...]}}
        files_dict = symbols_data.get("files", {})
        for file_path, file_data in files_dict.items():
            for symbol in file_data.get("symbols", []):
                name = symbol.get("name", "")
                symbol_kind = symbol.get("kind", "")

                if regex.search(name):
                    if kind is None or symbol_kind == kind:
                        matches.append({
                            "file": file_path,
                            "name": name,
                            "kind": symbol_kind,
                            "visibility": symbol.get("visibility", ""),
                            "line": symbol.get("line", 0),
                        })

        return {
            "success": True,
            "matches": matches[:100],  # Limit to 100
            "count": len(matches),
        }

    async def _handle_index_tests(self, args: dict[str, Any]) -> dict[str, Any]:
        """Find tests by pattern or crate."""
        pattern = args.get("pattern")
        crate_filter = args.get("crate")

        # Auto-build indexes if missing
        success, error = self._ensure_indexes_exist()
        if not success:
            return {"success": False, "error": error or "Failed to build indexes. Check codebase path."}

        tests_path = self.indexes_path / "structural" / "tests.json"
        if not tests_path.exists():
            return {"success": False, "error": "Tests index not found after rebuild attempt."}

        tests_data = json.loads(tests_path.read_text())
        matches = []

        # tests.json has a flat "tests" array
        for test in tests_data.get("tests", []):
            test_name = test.get("name", "")
            test_file = test.get("file", "")
            test_type = test.get("type", "")

            # Extract crate from file path (e.g., "crates/kelpie-core/..." -> "kelpie-core")
            crate_name = ""
            if "crates/" in test_file:
                parts = test_file.split("crates/")[1].split("/")
                if parts:
                    crate_name = parts[0]

            if crate_filter and crate_name != crate_filter:
                continue

            if pattern is None or re.search(pattern, test_name, re.IGNORECASE):
                matches.append({
                    "crate": crate_name,
                    "file": test_file,
                    "name": test_name,
                    "line": test.get("line", 0),
                    "type": test_type,
                    "topics": test.get("topics", []),
                    "command": test.get("command", ""),
                })

        return {
            "success": True,
            "tests": matches[:100],
            "count": len(matches),
        }

    async def _handle_index_modules(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get module hierarchy."""
        crate_filter = args.get("crate")

        # Auto-build indexes if missing
        success, error = self._ensure_indexes_exist()
        if not success:
            return {"success": False, "error": error or "Failed to build indexes. Check codebase path."}

        modules_path = self.indexes_path / "structural" / "modules.json"
        if not modules_path.exists():
            return {"success": False, "error": "Modules index not found after rebuild attempt."}

        modules_data = json.loads(modules_path.read_text())
        crates = modules_data.get("crates", [])

        if crate_filter:
            crates = [c for c in crates if c.get("name") == crate_filter]

        return {
            "success": True,
            "crates": crates,
            "count": len(crates),
        }

    async def _handle_index_deps(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get dependency graph."""
        crate_filter = args.get("crate")

        # Auto-build indexes if missing
        success, error = self._ensure_indexes_exist()
        if not success:
            return {"success": False, "error": error or "Failed to build indexes. Check codebase path."}

        deps_path = self.indexes_path / "structural" / "dependencies.json"
        if not deps_path.exists():
            return {"success": False, "error": "Dependencies index not found after rebuild attempt."}

        deps_data = json.loads(deps_path.read_text())
        crates = deps_data.get("crates", [])

        if crate_filter:
            crates = [c for c in crates if c.get("name") == crate_filter]

        return {
            "success": True,
            "crates": crates,
            "count": len(crates),
        }

    async def _handle_index_status(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get status of all indexes."""
        indexes = ["symbols", "modules", "dependencies", "tests"]
        status = {}

        for index_name in indexes:
            index_path = self.indexes_path / "structural" / f"{index_name}.json"
            if index_path.exists():
                stat = index_path.stat()
                data = json.loads(index_path.read_text())
                status[index_name] = {
                    "exists": True,
                    "size_bytes": stat.st_size,
                    "modified_at": stat.st_mtime,
                    "version": data.get("version", "unknown"),
                    "generated_at": data.get("generated_at"),
                }
            else:
                status[index_name] = {"exists": False}

        return {
            "success": True,
            "status": status,
        }

    async def _handle_index_refresh(self, args: dict[str, Any]) -> dict[str, Any]:
        """Rebuild indexes using Python indexer."""
        scope = args.get("scope", "all")

        try:
            output_dir = self.indexes_path / "structural"
            indexer = Indexer(self.codebase_path)
            result = indexer.build_all(output_dir)

            return {
                "success": True,
                "scope": scope,
                "files_indexed": len(result.get("symbols", {}).get("files", [])),
                "crates_indexed": len(result.get("dependencies", {}).get("crates", [])),
            }
        except Exception as e:
            return {
                "success": False,
                "error": str(e),
            }

    # ==================== Verification Tools ====================
    # NOTE: Verification tools removed - redundant with Claude's Bash tool.
    # Use Bash directly: cargo test, cargo clippy, cargo fmt --check

    # ==================== DST Tools ====================
    # NOTE: DST tools removed - use RLM/REPL primitives instead.
    # See definitions.py for usage examples.

    # ==================== Codebase Tools ====================
    # NOTE: Codebase tools removed - redundant with Claude's built-in tools.
    # Use: Grep, Read, Glob tools, or repl_load + repl_exec

    # ==================== Examination Tools ====================
    # Thorough examination system for building codebase understanding and surfacing issues.

    # In-memory examination state (also persisted to AgentFS)
    _exam_state: dict[str, Any] | None = None

    async def _handle_exam_start(self, args: dict[str, Any]) -> dict[str, Any]:
        """Start a thorough examination with defined scope."""
        task = args.get("task", "")
        scope = args.get("scope", [])

        # Initialize VFS session if not already
        if self._vfs_session is None:
            from ..agentfs import SessionManager
            if self._session_manager is None:
                self._session_manager = SessionManager(str(self.codebase_path))
            self._vfs_session = await self._session_manager.init_session(f"exam: {task}")

        # Determine actual components to examine
        if scope == ["all"] or "all" in scope:
            # Get all crates/modules from index
            modules_result = await self._handle_index_modules({})
            if modules_result.get("success"):
                components = [c.get("name") for c in modules_result.get("crates", [])]
            else:
                components = []
        else:
            components = scope

        # Initialize examination state
        self._exam_state = {
            "task": task,
            "scope": components,
            "examined": {},  # component -> findings
            "issues": [],    # flat list of all issues
            "started_at": _utcnow(),
            "completed_at": None,
        }

        # Persist to AgentFS
        await self._vfs_session.kv_set("exam:current", json.dumps(self._exam_state))

        return {
            "success": True,
            "task": task,
            "scope": components,
            "total_components": len(components),
            "message": f"Examination started. {len(components)} components to examine.",
        }

    async def _handle_exam_record(self, args: dict[str, Any]) -> dict[str, Any]:
        """Record findings for a component during examination."""
        if self._exam_state is None:
            return {"success": False, "error": "No active examination. Call exam_start first."}

        component = args.get("component", "")
        summary = args.get("summary", "")
        details = args.get("details", "")
        connections = args.get("connections", [])
        issues = args.get("issues", [])

        # Record the component's understanding
        self._exam_state["examined"][component] = {
            "summary": summary,
            "details": details,
            "connections": connections,
            "examined_at": _utcnow(),
        }

        # Add issues with component reference
        for issue in issues:
            self._exam_state["issues"].append({
                "component": component,
                "severity": issue.get("severity", "medium"),
                "description": issue.get("description", ""),
                "evidence": issue.get("evidence", ""),
                "found_at": _utcnow(),
            })

        # Persist to AgentFS
        await self._vfs_session.kv_set("exam:current", json.dumps(self._exam_state))
        await self._vfs_session.kv_set(f"exam:component:{component}", json.dumps(self._exam_state["examined"][component]))

        # Calculate progress
        examined_count = len(self._exam_state["examined"])
        total_count = len(self._exam_state["scope"])
        remaining = [c for c in self._exam_state["scope"] if c not in self._exam_state["examined"]]

        return {
            "success": True,
            "component": component,
            "issues_found": len(issues),
            "progress": f"{examined_count}/{total_count}",
            "remaining": remaining[:10],  # Show first 10 remaining
            "remaining_count": len(remaining),
        }

    async def _handle_exam_status(self, args: dict[str, Any]) -> dict[str, Any]:
        """Get examination progress."""
        if self._exam_state is None:
            # Try to load from AgentFS
            if self._vfs_session is not None:
                stored = await self._vfs_session.kv_get("exam:current")
                if stored:
                    self._exam_state = json.loads(stored)

        if self._exam_state is None:
            return {"success": False, "error": "No active examination. Call exam_start first."}

        examined = list(self._exam_state["examined"].keys())
        remaining = [c for c in self._exam_state["scope"] if c not in self._exam_state["examined"]]

        # Count issues by severity
        issue_counts = {"critical": 0, "high": 0, "medium": 0, "low": 0}
        for issue in self._exam_state["issues"]:
            severity = issue.get("severity", "medium")
            issue_counts[severity] = issue_counts.get(severity, 0) + 1

        return {
            "success": True,
            "task": self._exam_state["task"],
            "progress": f"{len(examined)}/{len(self._exam_state['scope'])}",
            "examined": examined,
            "remaining": remaining,
            "examined_count": len(examined),
            "remaining_count": len(remaining),
            "total_issues": len(self._exam_state["issues"]),
            "issues_by_severity": issue_counts,
            "started_at": self._exam_state["started_at"],
            "is_complete": len(remaining) == 0,
        }

    async def _handle_exam_complete(self, args: dict[str, Any]) -> dict[str, Any]:
        """Check if examination is complete (gate for answering questions)."""
        if self._exam_state is None:
            return {
                "success": True,
                "complete": False,
                "can_answer": False,
                "reason": "No active examination. Call exam_start first.",
            }

        remaining = [c for c in self._exam_state["scope"] if c not in self._exam_state["examined"]]

        if len(remaining) > 0:
            return {
                "success": True,
                "complete": False,
                "can_answer": False,
                "reason": f"Examination incomplete. {len(remaining)} components not yet examined.",
                "remaining": remaining,
            }

        # Mark as complete
        self._exam_state["completed_at"] = _utcnow()
        await self._vfs_session.kv_set("exam:current", json.dumps(self._exam_state))

        return {
            "success": True,
            "complete": True,
            "can_answer": True,
            "examined_count": len(self._exam_state["examined"]),
            "total_issues": len(self._exam_state["issues"]),
            "message": "All components examined. You may now provide a thorough answer.",
        }

    async def _handle_exam_export(self, args: dict[str, Any]) -> dict[str, Any]:
        """Export examination findings to human-readable markdown."""
        if self._exam_state is None:
            return {"success": False, "error": "No active examination. Call exam_start first."}

        include_details = args.get("include_details", True)

        # Generate timestamped directory name with task slug
        # Format: YYYYMMDD_HHMMSS_task-slug
        import re
        from datetime import datetime, timezone

        now = datetime.now(timezone.utc)
        timestamp = now.strftime("%Y%m%d_%H%M%S")

        # Create slug from task (first 50 chars, lowercase, alphanumeric + hyphens)
        task = self._exam_state.get("task", "examination")
        slug = re.sub(r"[^a-z0-9]+", "-", task.lower())[:50].strip("-")
        if not slug:
            slug = "examination"

        dir_name = f"{timestamp}_{slug}"

        # Create output directory with history preservation
        output_dir = self.indexes_path / "understanding" / dir_name
        output_dir.mkdir(parents=True, exist_ok=True)

        # Update "latest" symlink for convenience
        latest_link = self.indexes_path / "understanding" / "latest"
        if latest_link.is_symlink():
            latest_link.unlink()
        elif latest_link.exists():
            # If it's a real directory (from old implementation), leave it
            pass
        else:
            latest_link.symlink_to(dir_name)

        # Generate MAP.md
        map_content = self._generate_map_markdown(include_details)
        map_path = output_dir / "MAP.md"
        map_path.write_text(map_content)

        # Generate ISSUES.md
        issues_content = self._generate_issues_markdown()
        issues_path = output_dir / "ISSUES.md"
        issues_path.write_text(issues_content)

        # Generate per-component files
        components_dir = output_dir / "components"
        components_dir.mkdir(exist_ok=True)

        for component, findings in self._exam_state["examined"].items():
            component_content = self._generate_component_markdown(component, findings, include_details)
            # Sanitize component name for filename
            safe_name = component.replace("/", "_").replace("\\", "_")
            component_path = components_dir / f"{safe_name}.md"
            component_path.write_text(component_content)

        return {
            "success": True,
            "output_dir": str(output_dir),
            "files": {
                "map": str(map_path),
                "issues": str(issues_path),
                "components": str(components_dir),
            },
            "component_count": len(self._exam_state["examined"]),
            "issue_count": len(self._exam_state["issues"]),
        }

    def _generate_map_markdown(self, include_details: bool) -> str:
        """Generate MAP.md content."""
        lines = [
            "# Codebase Map",
            "",
            f"**Task:** {self._exam_state['task']}",
            f"**Generated:** {_utcnow()}",
            f"**Components:** {len(self._exam_state['examined'])}",
            f"**Issues Found:** {len(self._exam_state['issues'])}",
            "",
            "---",
            "",
            "## Components Overview",
            "",
        ]

        # Sort components by name
        for component in sorted(self._exam_state["examined"].keys()):
            findings = self._exam_state["examined"][component]
            lines.append(f"### {component}")
            lines.append("")
            lines.append(f"**Summary:** {findings.get('summary', 'No summary')}")
            lines.append("")

            connections = findings.get("connections", [])
            if connections:
                lines.append(f"**Connects to:** {', '.join(connections)}")
                lines.append("")

            if include_details and findings.get("details"):
                lines.append("**Details:**")
                lines.append("")
                lines.append(findings["details"])
                lines.append("")

            # Component-specific issues
            component_issues = [i for i in self._exam_state["issues"] if i.get("component") == component]
            if component_issues:
                lines.append(f"**Issues ({len(component_issues)}):**")
                for issue in component_issues:
                    severity = issue.get("severity", "medium")
                    desc = issue.get("description", "")
                    lines.append(f"- [{severity.upper()}] {desc}")
                lines.append("")

            lines.append("---")
            lines.append("")

        # Connection graph (text-based)
        lines.append("## Component Connections")
        lines.append("")
        lines.append("```")
        for component in sorted(self._exam_state["examined"].keys()):
            findings = self._exam_state["examined"][component]
            connections = findings.get("connections", [])
            if connections:
                lines.append(f"{component} -> {', '.join(connections)}")
        lines.append("```")
        lines.append("")

        return "\n".join(lines)

    def _generate_issues_markdown(self) -> str:
        """Generate ISSUES.md content."""
        lines = [
            "# Issues Found During Examination",
            "",
            f"**Task:** {self._exam_state['task']}",
            f"**Generated:** {_utcnow()}",
            f"**Total Issues:** {len(self._exam_state['issues'])}",
            "",
            "---",
            "",
        ]

        # Group by severity
        by_severity = {"critical": [], "high": [], "medium": [], "low": []}
        for issue in self._exam_state["issues"]:
            severity = issue.get("severity", "medium")
            by_severity.setdefault(severity, []).append(issue)

        for severity in ["critical", "high", "medium", "low"]:
            issues = by_severity.get(severity, [])
            if issues:
                lines.append(f"## {severity.upper()} ({len(issues)})")
                lines.append("")
                for issue in issues:
                    component = issue.get("component", "unknown")
                    desc = issue.get("description", "")
                    evidence = issue.get("evidence", "")
                    lines.append(f"### [{component}] {desc}")
                    lines.append("")
                    if evidence:
                        lines.append(f"**Evidence:** {evidence}")
                        lines.append("")
                    lines.append(f"*Found: {issue.get('found_at', 'unknown')}*")
                    lines.append("")
                    lines.append("---")
                    lines.append("")

        if not self._exam_state["issues"]:
            lines.append("*No issues found during examination.*")
            lines.append("")

        return "\n".join(lines)

    def _generate_component_markdown(self, component: str, findings: dict, include_details: bool) -> str:
        """Generate per-component markdown."""
        lines = [
            f"# {component}",
            "",
            f"**Examined:** {findings.get('examined_at', 'unknown')}",
            "",
            "## Summary",
            "",
            findings.get("summary", "No summary"),
            "",
        ]

        connections = findings.get("connections", [])
        if connections:
            lines.append("## Connections")
            lines.append("")
            for conn in connections:
                lines.append(f"- {conn}")
            lines.append("")

        if include_details and findings.get("details"):
            lines.append("## Details")
            lines.append("")
            lines.append(findings["details"])
            lines.append("")

        # Component-specific issues
        component_issues = [i for i in self._exam_state["issues"] if i.get("component") == component]
        if component_issues:
            lines.append("## Issues")
            lines.append("")
            for issue in component_issues:
                severity = issue.get("severity", "medium")
                desc = issue.get("description", "")
                evidence = issue.get("evidence", "")
                lines.append(f"### [{severity.upper()}] {desc}")
                lines.append("")
                if evidence:
                    lines.append(f"**Evidence:** {evidence}")
                    lines.append("")

        return "\n".join(lines)

    async def _handle_issue_list(self, args: dict[str, Any]) -> dict[str, Any]:
        """List issues found during examination."""
        if self._exam_state is None:
            return {"success": False, "error": "No active examination. Call exam_start first."}

        component_filter = args.get("component")
        severity_filter = args.get("severity")

        issues = self._exam_state["issues"]

        # Apply filters
        if component_filter:
            issues = [i for i in issues if i.get("component") == component_filter]
        if severity_filter:
            issues = [i for i in issues if i.get("severity") == severity_filter]

        # Count by severity
        counts = {"critical": 0, "high": 0, "medium": 0, "low": 0}
        for issue in issues:
            severity = issue.get("severity", "medium")
            counts[severity] = counts.get(severity, 0) + 1

        return {
            "success": True,
            "issues": issues,
            "count": len(issues),
            "by_severity": counts,
            "filters_applied": {
                "component": component_filter,
                "severity": severity_filter,
            },
        }
