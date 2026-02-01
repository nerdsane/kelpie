"""
Main indexer for Kelpie Repo OS.

Builds structural indexes for the Rust codebase:
- symbols.json: Functions, structs, traits, etc.
- modules.json: Module hierarchy
- dependencies.json: Crate dependency graph
- tests.json: Test cases
"""

import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .parser import RustParser
from .types import (
    SymbolIndex,
    FileSymbols,
    ModuleIndex,
    CrateModules,
    ModuleInfo,
    DependencyGraph,
    CrateDependencies,
    Dependency,
    TestIndex,
    CrateTests,
    TestModule,
    TestCase,
    SymbolKind,
)


INDEX_VERSION = "2.0.0"  # Python port version


def _utcnow() -> str:
    """Get current UTC time as ISO string."""
    return datetime.now(timezone.utc).isoformat()


class Indexer:
    """Build structural indexes for a Rust workspace."""

    def __init__(self, workspace_root: Path):
        """Initialize indexer.

        Args:
            workspace_root: Path to workspace root (containing Cargo.toml)
        """
        # Resolve symlinks to avoid path comparison issues (e.g., /var vs /private/var on macOS)
        self.workspace_root = workspace_root.resolve()
        self.parser = RustParser()
        self._cargo_metadata: dict[str, Any] | None = None

    def get_cargo_metadata(self) -> dict[str, Any]:
        """Get cargo metadata for workspace.

        Returns:
            Parsed cargo metadata JSON
        """
        if self._cargo_metadata is None:
            result = subprocess.run(
                ["cargo", "metadata", "--format-version", "1", "--no-deps"],
                cwd=self.workspace_root,
                capture_output=True,
                text=True,
            )
            if result.returncode != 0:
                raise RuntimeError(f"cargo metadata failed: {result.stderr}")
            self._cargo_metadata = json.loads(result.stdout)
        return self._cargo_metadata

    def find_workspace_crates(self) -> list[dict[str, Any]]:
        """Find all crates in the workspace.

        Returns:
            List of crate info dicts with name, path, etc.
        """
        metadata = self.get_cargo_metadata()
        workspace_members = set(metadata.get("workspace_members", []))

        crates = []
        for package in metadata.get("packages", []):
            pkg_id = package.get("id", "")
            if pkg_id in workspace_members:
                manifest_path = Path(package.get("manifest_path", ""))
                crate_root = manifest_path.parent
                crates.append(
                    {
                        "name": package.get("name", ""),
                        "version": package.get("version", ""),
                        "path": str(crate_root),
                        "manifest_path": str(manifest_path),
                        "dependencies": package.get("dependencies", []),
                    }
                )
        return crates

    def find_rust_files(self, crate_path: Path) -> list[Path]:
        """Find all Rust source files in a crate.

        Args:
            crate_path: Path to crate root

        Returns:
            List of .rs file paths
        """
        src_dir = crate_path / "src"
        if not src_dir.exists():
            return []

        return list(src_dir.rglob("*.rs"))

    def build_symbol_index(self) -> SymbolIndex:
        """Build symbol index for entire workspace.

        Returns:
            SymbolIndex with all symbols
        """
        files: list[FileSymbols] = []

        for crate in self.find_workspace_crates():
            crate_path = Path(crate["path"]).resolve()
            rust_files = self.find_rust_files(crate_path)

            for file_path in rust_files:
                file_symbols = self.parser.parse_file(file_path)
                if file_symbols:
                    # Make path relative to workspace (resolve to handle symlinks)
                    rel_path = file_path.resolve().relative_to(self.workspace_root)
                    file_symbols.path = str(rel_path)
                    files.append(file_symbols)

        return SymbolIndex(
            version=INDEX_VERSION,
            generated_at=_utcnow(),
            files=files,
        )

    def build_module_index(self) -> ModuleIndex:
        """Build module index for workspace.

        Returns:
            ModuleIndex with module hierarchy
        """
        crates: list[CrateModules] = []

        for crate in self.find_workspace_crates():
            crate_path = Path(crate["path"]).resolve()
            crate_name = crate["name"]

            modules = self._build_crate_modules(crate_path, crate_name)
            crates.append(
                CrateModules(
                    crate_name=crate_name,
                    root_path=str(crate_path.relative_to(self.workspace_root)),
                    modules=modules,
                )
            )

        return ModuleIndex(
            version=INDEX_VERSION,
            generated_at=_utcnow(),
            crates=crates,
        )

    def _build_crate_modules(self, crate_path: Path, crate_name: str) -> list[ModuleInfo]:
        """Build module hierarchy for a single crate.

        Args:
            crate_path: Path to crate
            crate_name: Name of crate

        Returns:
            List of ModuleInfo
        """
        modules: list[ModuleInfo] = []
        src_dir = (crate_path / "src").resolve()

        if not src_dir.exists():
            return modules

        # Find all module files
        for rs_file in src_dir.rglob("*.rs"):
            rel_path = rs_file.resolve().relative_to(src_dir)

            # Determine module name
            if rs_file.name == "lib.rs":
                mod_name = crate_name
            elif rs_file.name == "main.rs":
                mod_name = f"{crate_name}::main"
            elif rs_file.name == "mod.rs":
                mod_name = f"{crate_name}::{str(rel_path.parent).replace('/', '::')}"
            else:
                mod_name = f"{crate_name}::{str(rel_path.with_suffix('')).replace('/', '::')}"

            # Parse file to get doc comment and visibility
            file_symbols = self.parser.parse_file(rs_file)

            # Look for module-level doc comment
            doc = None
            is_public = False
            children: list[str] = []

            if file_symbols:
                # First symbol's doc might be module doc
                for symbol in file_symbols.symbols:
                    if symbol.kind == SymbolKind.MOD:
                        children.append(symbol.name)

            modules.append(
                ModuleInfo(
                    name=mod_name,
                    path=str(rs_file.resolve().relative_to(self.workspace_root)),
                    is_public=is_public,
                    doc=doc,
                    children=children,
                )
            )

        return modules

    def build_dependency_graph(self) -> DependencyGraph:
        """Build dependency graph for workspace.

        Returns:
            DependencyGraph with all crate dependencies
        """
        crates: list[CrateDependencies] = []

        for crate in self.find_workspace_crates():
            deps: list[Dependency] = []
            dev_deps: list[Dependency] = []
            build_deps: list[Dependency] = []

            for dep in crate.get("dependencies", []):
                dep_obj = Dependency(
                    name=dep.get("name", ""),
                    version=dep.get("req", None),
                    path=dep.get("path", None),
                    features=dep.get("features", []),
                )

                kind = dep.get("kind", None)
                if kind == "dev":
                    dep_obj.is_dev = True
                    dev_deps.append(dep_obj)
                elif kind == "build":
                    dep_obj.is_build = True
                    build_deps.append(dep_obj)
                else:
                    deps.append(dep_obj)

            crates.append(
                CrateDependencies(
                    crate_name=crate["name"],
                    dependencies=deps,
                    dev_dependencies=dev_deps,
                    build_dependencies=build_deps,
                )
            )

        return DependencyGraph(
            version=INDEX_VERSION,
            generated_at=_utcnow(),
            crates=crates,
        )

    def build_test_index(self) -> TestIndex:
        """Build test index for workspace.

        Returns:
            TestIndex with all test cases
        """
        crates: list[CrateTests] = []

        for crate in self.find_workspace_crates():
            crate_path = Path(crate["path"]).resolve()
            crate_name = crate["name"]

            test_modules = self._build_crate_tests(crate_path, crate_name)
            if test_modules:
                crates.append(
                    CrateTests(
                        crate_name=crate_name,
                        modules=test_modules,
                    )
                )

        return TestIndex(
            version=INDEX_VERSION,
            generated_at=_utcnow(),
            crates=crates,
        )

    def _build_crate_tests(self, crate_path: Path, crate_name: str) -> list[TestModule]:
        """Find all tests in a crate.

        Args:
            crate_path: Path to crate
            crate_name: Name of crate

        Returns:
            List of TestModule
        """
        test_modules: list[TestModule] = []

        # Check src/ for inline tests
        src_dir = (crate_path / "src").resolve()
        if src_dir.exists():
            for rs_file in src_dir.rglob("*.rs"):
                tests = self._find_tests_in_file(rs_file)
                if tests:
                    rel_path = rs_file.resolve().relative_to(self.workspace_root)
                    test_modules.append(
                        TestModule(
                            module_path=str(rel_path),
                            tests=tests,
                        )
                    )

        # Check tests/ directory
        tests_dir = (crate_path / "tests").resolve()
        if tests_dir.exists():
            for rs_file in tests_dir.rglob("*.rs"):
                tests = self._find_tests_in_file(rs_file)
                if tests:
                    rel_path = rs_file.resolve().relative_to(self.workspace_root)
                    test_modules.append(
                        TestModule(
                            module_path=str(rel_path),
                            tests=tests,
                        )
                    )

        return test_modules

    def _find_tests_in_file(self, file_path: Path) -> list[TestCase]:
        """Find all test functions in a file.

        Args:
            file_path: Path to Rust file

        Returns:
            List of TestCase
        """
        file_symbols = self.parser.parse_file(file_path)
        if not file_symbols:
            return []

        tests: list[TestCase] = []
        for symbol in file_symbols.symbols:
            if symbol.is_test:
                is_ignored = any("ignore" in a for a in symbol.attributes)
                tests.append(
                    TestCase(
                        name=symbol.name,
                        path=str(file_path),
                        line=symbol.line,
                        is_ignored=is_ignored,
                        is_async=symbol.is_async,
                        attributes=symbol.attributes,
                        doc=symbol.doc,
                    )
                )

        return tests

    def build_all(self, output_dir: Path | None = None) -> dict[str, Any]:
        """Build all indexes and optionally write to disk.

        Args:
            output_dir: Directory to write JSON files (optional)

        Returns:
            Dict with all index data
        """
        symbol_index = self.build_symbol_index()
        module_index = self.build_module_index()
        dependency_graph = self.build_dependency_graph()
        test_index = self.build_test_index()

        result = {
            "symbols": symbol_index.to_dict(),
            "modules": module_index.to_dict(),
            "dependencies": dependency_graph.to_dict(),
            "tests": test_index.to_dict(),
        }

        if output_dir:
            output_dir.mkdir(parents=True, exist_ok=True)

            # Write each index
            (output_dir / "symbols.json").write_text(
                json.dumps(result["symbols"], indent=2), encoding="utf-8"
            )
            (output_dir / "modules.json").write_text(
                json.dumps(result["modules"], indent=2), encoding="utf-8"
            )
            (output_dir / "dependencies.json").write_text(
                json.dumps(result["dependencies"], indent=2), encoding="utf-8"
            )
            (output_dir / "tests.json").write_text(
                json.dumps(result["tests"], indent=2), encoding="utf-8"
            )

        return result


def build_indexes(workspace_root: str | Path, output_dir: str | Path | None = None) -> dict[str, Any]:
    """Convenience function to build all indexes.

    Args:
        workspace_root: Path to workspace root
        output_dir: Optional directory to write JSON files

    Returns:
        Dict with all index data
    """
    indexer = Indexer(Path(workspace_root))
    output = Path(output_dir) if output_dir else None
    return indexer.build_all(output)
