"""
Kelpie indexer module - structural code indexing using tree-sitter.

Builds 4 indexes:
- symbols.json: Functions, structs, traits, etc.
- modules.json: Module hierarchy per crate
- dependencies.json: Crate dependency graph
- tests.json: Test cases with attributes
"""

from .types import (
    Symbol,
    SymbolKind,
    Visibility,
    Import,
    FileSymbols,
    SymbolIndex,
    ModuleInfo,
    CrateModules,
    ModuleIndex,
    Dependency,
    CrateDependencies,
    DependencyGraph,
    TestCase,
    TestModule,
    CrateTests,
    TestIndex,
)
from .parser import RustParser
from .indexer import Indexer, build_indexes

__all__ = [
    # Types
    "Symbol",
    "SymbolKind",
    "Visibility",
    "Import",
    "FileSymbols",
    "SymbolIndex",
    "ModuleInfo",
    "CrateModules",
    "ModuleIndex",
    "Dependency",
    "CrateDependencies",
    "DependencyGraph",
    "TestCase",
    "TestModule",
    "CrateTests",
    "TestIndex",
    # Parser
    "RustParser",
    # Indexer
    "Indexer",
    "build_indexes",
]
