"""
Types for RLM environment.

TigerStyle: All types are immutable dataclasses.
"""

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class GrepMatch:
    """A single grep match result."""

    file: str
    line: int
    content: str

    def __repr__(self) -> str:
        content_preview = self.content[:50] + "..." if len(self.content) > 50 else self.content
        return f"GrepMatch({self.file}:{self.line}: {content_preview})"


@dataclass(frozen=True)
class ModuleContext:
    """Context for a single module, used for partitioning."""

    module_name: str
    files: tuple[str, ...]  # Immutable tuple instead of list
    root_path: str

    def __repr__(self) -> str:
        return f"ModuleContext({self.module_name}, {len(self.files)} files)"


@dataclass
class ExecutionResult:
    """Result of RLM code execution."""

    success: bool
    result: Any | None = None
    error: str | None = None
    execution_log: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "success": self.success,
            "result": str(self.result) if self.result is not None else None,
            "error": self.error,
            "execution_log": self.execution_log,
        }


@dataclass
class LoadedVariable:
    """Represents a variable loaded into the REPL environment.

    VDE insight: Code becomes data in variables, not tokens in context.
    """

    name: str
    glob_pattern: str
    file_count: int
    total_bytes: int
    files: dict[str, str]  # path -> content

    def __repr__(self) -> str:
        size_kb = self.total_bytes / 1024
        return f"LoadedVariable({self.name}: {self.file_count} files, {size_kb:.1f}KB)"

    def summary(self) -> str:
        """Return human-readable summary."""
        size_kb = self.total_bytes / 1024
        return f"Loaded {self.file_count} files ({size_kb:.1f}KB) into '{self.name}'"


@dataclass
class SubLLMResult:
    """Result from a sub-LLM call."""

    success: bool
    response: str | None = None
    error: str | None = None
    model: str | None = None
    input_tokens: int = 0
    output_tokens: int = 0

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "success": self.success,
            "response": self.response,
            "error": self.error,
            "model": self.model,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
        }
