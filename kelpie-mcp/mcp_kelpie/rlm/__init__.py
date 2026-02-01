"""
RLM (Recursive Language Model) environment for Kelpie MCP

Based on VDE implementation (sections 3.4, 4.2). The key insight: code becomes data
in variables, not tokens in context. Sub-models can analyze large codebases server-side.

Key components:
- REPLEnvironment: Manages variables and code execution
- CodebaseContext: Programmatic codebase access (grep, peek, read_section)
- SubLLM: Spawns sub-model calls to analyze variables

Tools exposed via MCP:
- repl_load: Load files into server variable by glob pattern
- repl_exec: Execute Python code on loaded variables
- repl_query: Evaluate expression and return result
- repl_sub_llm: Have sub-model analyze a variable IN THE SERVER
- repl_state: Show current variable names and sizes
- repl_clear: Clear variables to free memory
"""

from .types import ExecutionResult, GrepMatch, LoadedVariable, ModuleContext, SubLLMResult
from .context import CodebaseContext
from .repl import REPLEnvironment
from .llm import SubLLM

__all__ = [
    "REPLEnvironment",
    "CodebaseContext",
    "SubLLM",
    "GrepMatch",
    "ModuleContext",
    "ExecutionResult",
    "LoadedVariable",
    "SubLLMResult",
]
