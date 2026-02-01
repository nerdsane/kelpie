"""
Index data types for Kelpie Repo OS.

Matches the JSON output format of the Rust kelpie-indexer.
"""

from dataclasses import dataclass, field
from enum import Enum
from typing import Any


class SymbolKind(str, Enum):
    """Kind of code symbol."""

    FUNCTION = "function"
    STRUCT = "struct"
    ENUM = "enum"
    TRAIT = "trait"
    IMPL = "impl"
    CONST = "const"
    STATIC = "static"
    TYPE_ALIAS = "type_alias"
    MACRO = "macro"
    MOD = "mod"


class Visibility(str, Enum):
    """Symbol visibility."""

    PUBLIC = "pub"
    CRATE = "pub(crate)"
    SUPER = "pub(super)"
    PRIVATE = "private"


@dataclass
class Symbol:
    """A code symbol (function, struct, trait, etc.)."""

    name: str
    kind: SymbolKind
    line: int
    visibility: Visibility = Visibility.PRIVATE
    signature: str | None = None
    doc: str | None = None
    is_async: bool = False
    is_test: bool = False
    is_unsafe: bool = False
    generic_params: list[str] = field(default_factory=list)
    attributes: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result: dict[str, Any] = {
            "name": self.name,
            "kind": self.kind.value,
            "line": self.line,
            "visibility": self.visibility.value,
        }
        if self.signature:
            result["signature"] = self.signature
        if self.doc:
            result["doc"] = self.doc
        if self.is_async:
            result["is_async"] = True
        if self.is_test:
            result["is_test"] = True
        if self.is_unsafe:
            result["is_unsafe"] = True
        if self.generic_params:
            result["generic_params"] = self.generic_params
        if self.attributes:
            result["attributes"] = self.attributes
        return result


@dataclass
class Import:
    """An import statement."""

    path: str
    alias: str | None = None
    is_glob: bool = False

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result: dict[str, Any] = {"path": self.path}
        if self.alias:
            result["alias"] = self.alias
        if self.is_glob:
            result["is_glob"] = True
        return result


@dataclass
class FileSymbols:
    """Symbols from a single file."""

    path: str
    symbols: list[Symbol] = field(default_factory=list)
    imports: list[Import] = field(default_factory=list)
    exports_to: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "path": self.path,
            "symbols": [s.to_dict() for s in self.symbols],
            "imports": [i.to_dict() for i in self.imports],
            "exports_to": self.exports_to,
        }


@dataclass
class SymbolIndex:
    """Index of all symbols in the codebase."""

    version: str
    generated_at: str
    files: list[FileSymbols] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "version": self.version,
            "generated_at": self.generated_at,
            "files": [f.to_dict() for f in self.files],
        }


# ==================== Module Index ====================


@dataclass
class ModuleInfo:
    """Information about a Rust module."""

    name: str
    path: str
    is_public: bool = False
    doc: str | None = None
    children: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result: dict[str, Any] = {
            "name": self.name,
            "path": self.path,
            "is_public": self.is_public,
        }
        if self.doc:
            result["doc"] = self.doc
        if self.children:
            result["children"] = self.children
        return result


@dataclass
class CrateModules:
    """Modules in a crate."""

    crate_name: str
    root_path: str
    modules: list[ModuleInfo] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "crate_name": self.crate_name,
            "root_path": self.root_path,
            "modules": [m.to_dict() for m in self.modules],
        }


@dataclass
class ModuleIndex:
    """Index of all modules in the workspace."""

    version: str
    generated_at: str
    crates: list[CrateModules] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "version": self.version,
            "generated_at": self.generated_at,
            "crates": [c.to_dict() for c in self.crates],
        }


# ==================== Dependency Index ====================


@dataclass
class Dependency:
    """A crate dependency."""

    name: str
    version: str | None = None
    path: str | None = None
    is_dev: bool = False
    is_build: bool = False
    features: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result: dict[str, Any] = {"name": self.name}
        if self.version:
            result["version"] = self.version
        if self.path:
            result["path"] = self.path
        if self.is_dev:
            result["is_dev"] = True
        if self.is_build:
            result["is_build"] = True
        if self.features:
            result["features"] = self.features
        return result


@dataclass
class CrateDependencies:
    """Dependencies of a crate."""

    crate_name: str
    dependencies: list[Dependency] = field(default_factory=list)
    dev_dependencies: list[Dependency] = field(default_factory=list)
    build_dependencies: list[Dependency] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "crate_name": self.crate_name,
            "dependencies": [d.to_dict() for d in self.dependencies],
            "dev_dependencies": [d.to_dict() for d in self.dev_dependencies],
            "build_dependencies": [d.to_dict() for d in self.build_dependencies],
        }


@dataclass
class DependencyGraph:
    """Dependency graph for the workspace."""

    version: str
    generated_at: str
    crates: list[CrateDependencies] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "version": self.version,
            "generated_at": self.generated_at,
            "crates": [c.to_dict() for c in self.crates],
        }


# ==================== Test Index ====================


@dataclass
class TestCase:
    """A test case."""

    name: str
    path: str
    line: int
    is_ignored: bool = False
    is_async: bool = False
    attributes: list[str] = field(default_factory=list)
    doc: str | None = None

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result: dict[str, Any] = {
            "name": self.name,
            "path": self.path,
            "line": self.line,
        }
        if self.is_ignored:
            result["is_ignored"] = True
        if self.is_async:
            result["is_async"] = True
        if self.attributes:
            result["attributes"] = self.attributes
        if self.doc:
            result["doc"] = self.doc
        return result


@dataclass
class TestModule:
    """Tests in a module."""

    module_path: str
    tests: list[TestCase] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "module_path": self.module_path,
            "tests": [t.to_dict() for t in self.tests],
        }


@dataclass
class CrateTests:
    """Tests in a crate."""

    crate_name: str
    modules: list[TestModule] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "crate_name": self.crate_name,
            "modules": [m.to_dict() for m in self.modules],
        }


@dataclass
class TestIndex:
    """Index of all tests in the workspace."""

    version: str
    generated_at: str
    crates: list[CrateTests] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        return {
            "version": self.version,
            "generated_at": self.generated_at,
            "crates": [c.to_dict() for c in self.crates],
        }
