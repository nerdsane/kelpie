"""
Codebase access layer for RLM environment.

Provides programmatic access to the codebase without loading it into LLM context.
All operations are read-only and work with the structural indexes.

Based on existing rlm-env implementation with VDE enhancements.
"""

import json
import re
from pathlib import Path
from typing import Any

from .types import GrepMatch, ModuleContext

# TigerStyle: Explicit constants with units
PEEK_LINES_MAX = 500
GREP_MATCHES_MAX = 1000
READ_SECTION_LINES_MAX = 500
READ_CONTEXT_PADDING_MAX = 50


class CodebaseContext:
    """Codebase access layer - never loads full content into LLM context.

    TigerStyle: All methods are read-only. No mutation of codebase allowed.
    """

    def __init__(self, root_path: str, indexes_path: str | None = None):
        """Initialize codebase context.

        Args:
            root_path: Path to codebase root
            indexes_path: Path to .kelpie-index/ directory (optional, defaults to root/.kelpie-index)
        """
        self.root = Path(root_path)
        self.indexes_path = Path(indexes_path) if indexes_path else self.root / ".kelpie-index"

        # TigerStyle: Validate paths exist
        assert self.root.exists(), f"Codebase root not found: {root_path}"
        # Indexes are optional - may not exist yet
        self.indexes = self._load_indexes() if self.indexes_path.exists() else {}

    def _load_indexes(self) -> dict[str, Any]:
        """Load all structural indexes into memory."""
        indexes = {}

        structural_dir = self.indexes_path / "structural"
        if structural_dir.exists():
            for index_file in structural_dir.glob("*.json"):
                index_name = index_file.stem
                with open(index_file) as f:
                    indexes[index_name] = json.load(f)

        return indexes

    def list_files(self, glob_pattern: str = "**/*.rs") -> list[str]:
        """List files matching glob pattern.

        Args:
            glob_pattern: Glob pattern (default: all Rust files)

        Returns:
            List of file paths relative to root
        """
        return [str(p.relative_to(self.root)) for p in self.root.glob(glob_pattern) if p.is_file()]

    def list_crates(self) -> list[str]:
        """List all crates in the workspace.

        Returns:
            List of crate names from modules index
        """
        if "modules" not in self.indexes:
            return []

        return [crate["name"] for crate in self.indexes["modules"].get("crates", [])]

    def list_modules(self, crate_name: str | None = None) -> list[str]:
        """List all modules, optionally filtered by crate.

        Args:
            crate_name: If provided, only list modules from this crate

        Returns:
            List of module paths (e.g., "kelpie_core::actor")
        """
        if "modules" not in self.indexes:
            return []

        modules = []
        for crate in self.indexes["modules"].get("crates", []):
            if crate_name and crate["name"] != crate_name:
                continue

            for module in crate.get("modules", []):
                modules.append(module["path"])

        return modules

    def peek(self, file: str, lines: int = 50) -> str:
        """Sample first N lines of a file (for structure understanding).

        Args:
            file: File path relative to root
            lines: Number of lines to read (default: 50)

        Returns:
            First N lines of file

        TigerStyle: Explicit line limit to prevent accidental full file loads
        """
        assert lines > 0, "lines must be positive"
        assert lines <= PEEK_LINES_MAX, f"lines exceeds maximum ({PEEK_LINES_MAX})"

        path = self.root / file
        if not path.exists():
            return f"File not found: {file}"

        try:
            with open(path) as f:
                return "".join(f.readline() for _ in range(lines))
        except Exception as e:
            return f"Error reading file: {e}"

    def grep(self, pattern: str, glob: str = "**/*.rs", max_matches: int = 100) -> list[GrepMatch]:
        """Search for pattern without loading files into context.

        Args:
            pattern: Regular expression pattern
            glob: Glob pattern for files to search (default: all Rust files)
            max_matches: Maximum matches to return (default: 100)

        Returns:
            List of GrepMatch objects

        TigerStyle: Explicit max_matches to prevent unbounded memory use
        """
        assert max_matches > 0, "max_matches must be positive"
        assert max_matches <= GREP_MATCHES_MAX, f"max_matches exceeds maximum ({GREP_MATCHES_MAX})"

        matches = []
        try:
            regex = re.compile(pattern)
        except re.error as e:
            return [GrepMatch("error", 0, f"Invalid regex: {e}")]

        for file in self.list_files(glob):
            if len(matches) >= max_matches:
                break

            path = self.root / file
            try:
                with open(path) as f:
                    for i, line in enumerate(f, 1):
                        if regex.search(line):
                            matches.append(GrepMatch(file, i, line.rstrip()))
                            if len(matches) >= max_matches:
                                break
            except Exception:
                # Skip files that can't be read
                continue

        return matches

    def read_file(self, file: str) -> str:
        """Read entire file content.

        Args:
            file: File path relative to root

        Returns:
            File content or error message

        Note: Use with caution - prefer read_section for large files.
        """
        path = self.root / file
        if not path.exists():
            return f"File not found: {file}"

        try:
            with open(path) as f:
                return f.read()
        except Exception as e:
            return f"Error reading file: {e}"

    def read_section(self, file: str, start: int, end: int) -> str:
        """Read specific line range from file.

        Args:
            file: File path relative to root
            start: Start line (1-indexed, inclusive)
            end: End line (1-indexed, inclusive)

        Returns:
            Lines from start to end

        TigerStyle: Explicit bounds to prevent accidental full file loads
        """
        assert start > 0, "start must be positive"
        assert end >= start, "end must be >= start"
        assert (end - start) <= READ_SECTION_LINES_MAX, f"section size exceeds maximum ({READ_SECTION_LINES_MAX} lines)"

        path = self.root / file
        if not path.exists():
            return f"File not found: {file}"

        try:
            with open(path) as f:
                lines = f.readlines()
                # TigerStyle: Validate bounds
                if start > len(lines):
                    return f"Start line {start} exceeds file length {len(lines)}"
                return "".join(lines[start - 1 : end])
        except Exception as e:
            return f"Error reading file: {e}"

    def read_context(self, file: str, line: int, padding: int = 10) -> str:
        """Read context around a specific line.

        Args:
            file: File path relative to root
            line: Target line number (1-indexed)
            padding: Lines before and after to include (default: 10)

        Returns:
            Context around the line
        """
        assert padding >= 0, "padding must be non-negative"
        assert padding <= READ_CONTEXT_PADDING_MAX, f"padding exceeds maximum ({READ_CONTEXT_PADDING_MAX})"

        start = max(1, line - padding)
        end = line + padding
        return self.read_section(file, start, end)

    def get_module(self, module_path: str) -> ModuleContext | None:
        """Get focused context for a single module.

        Args:
            module_path: Module path (e.g., "kelpie_core::actor")

        Returns:
            ModuleContext or None if not found
        """
        if "modules" not in self.indexes:
            return None

        for crate in self.indexes["modules"].get("crates", []):
            for module in crate.get("modules", []):
                if module["path"] == module_path:
                    return ModuleContext(
                        module_name=module_path,
                        files=tuple([module["file"]]),
                        root_path=str(self.root),
                    )

        return None

    def partition_by_crate(self) -> list[ModuleContext]:
        """Partition codebase by crate for map-reduce operations.

        Returns:
            List of ModuleContext objects, one per crate
        """
        if "modules" not in self.indexes:
            return []

        contexts = []
        for crate in self.indexes["modules"].get("crates", []):
            files = []
            for module in crate.get("modules", []):
                files.append(module["file"])

            if files:
                contexts.append(
                    ModuleContext(
                        module_name=crate["name"],
                        files=tuple(files),
                        root_path=str(self.root),
                    )
                )

        return contexts

    def get_index(self, index_name: str) -> dict[str, Any] | None:
        """Get a specific index by name.

        Args:
            index_name: Index name (e.g., "symbols", "dependencies", "tests", "modules")

        Returns:
            Index data or None if not found
        """
        return self.indexes.get(index_name)

    def list_tests(self, topic: str | None = None, test_type: str | None = None) -> list[dict[str, Any]]:
        """List tests, optionally filtered by topic or type.

        Args:
            topic: Filter by topic (e.g., "storage", "actor")
            test_type: Filter by type (e.g., "unit", "dst", "integration")

        Returns:
            List of test info dictionaries
        """
        if "tests" not in self.indexes:
            return []

        tests = self.indexes["tests"].get("tests", [])

        if topic:
            # Filter tests that have this topic
            tests = [t for t in tests if topic in t.get("topics", [])]

        if test_type:
            # Filter tests by type
            tests = [t for t in tests if t.get("type") == test_type]

        return tests
