"""
Tests for the indexer module.

Tests cover:
- RustParser: Tree-sitter based parsing
- Indexer: Building all 4 indexes
- Output format compatibility
"""

import tempfile
from pathlib import Path

import pytest

from mcp_kelpie.indexer import (
    RustParser,
    Indexer,
    Symbol,
    SymbolKind,
    Visibility,
    Import,
    FileSymbols,
)


# ==================== Fixtures ====================


@pytest.fixture
def parser():
    """Create a RustParser instance."""
    return RustParser()


@pytest.fixture
def temp_workspace():
    """Create a temporary Rust workspace for testing."""
    with tempfile.TemporaryDirectory() as tmpdir:
        root = Path(tmpdir)

        # Create workspace Cargo.toml
        (root / "Cargo.toml").write_text("""
[workspace]
members = ["crates/test-crate"]
""")

        # Create crate directory
        crate_dir = root / "crates" / "test-crate"
        crate_dir.mkdir(parents=True)

        # Create crate Cargo.toml
        (crate_dir / "Cargo.toml").write_text("""
[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"

[dev-dependencies]
tokio = { version = "1.0", features = ["macros", "rt"] }
""")

        # Create src directory
        src_dir = crate_dir / "src"
        src_dir.mkdir()

        # Create lib.rs
        (src_dir / "lib.rs").write_text("""
//! Test crate for indexer testing.

pub mod utils;

use std::collections::HashMap;

/// A test struct.
#[derive(Debug, Clone)]
pub struct TestStruct<T> {
    pub value: T,
}

/// A test enum.
pub enum TestEnum {
    Variant1,
    Variant2(String),
}

/// A test trait.
pub trait TestTrait {
    fn do_something(&self);
}

impl<T> TestTrait for TestStruct<T> {
    fn do_something(&self) {}
}

/// A test function.
pub fn test_function(x: i32, y: i32) -> i32 {
    x + y
}

/// An async function.
pub async fn async_function() -> String {
    "hello".to_string()
}

/// An unsafe function.
pub unsafe fn unsafe_function() -> *mut u8 {
    std::ptr::null_mut()
}

/// A constant.
pub const MAX_SIZE: usize = 1024;

/// A static variable.
pub static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A type alias.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(test_function(2, 2), 4);
    }

    #[test]
    #[ignore]
    fn test_ignored() {
        // This test is ignored
    }

    #[tokio::test]
    async fn test_async() {
        let result = async_function().await;
        assert_eq!(result, "hello");
    }
}
""")

        # Create utils.rs
        (src_dir / "utils.rs").write_text("""
//! Utility functions.

use std::fs::File;

/// A helper function.
pub fn helper() -> String {
    "helper".to_string()
}

pub(crate) fn crate_only() -> i32 {
    42
}
""")

        # Create tests directory
        tests_dir = crate_dir / "tests"
        tests_dir.mkdir()

        (tests_dir / "integration.rs").write_text("""
//! Integration tests.

use test_crate::test_function;

#[test]
fn test_integration() {
    assert_eq!(test_function(10, 20), 30);
}
""")

        yield root


# ==================== Parser Tests ====================


class TestRustParser:
    """Test RustParser tree-sitter parsing."""

    def test_parse_function(self, parser):
        """Test parsing a function."""
        content = """
/// A documented function.
pub fn my_function(x: i32) -> i32 {
    x + 1
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        func = result.symbols[0]
        assert func.name == "my_function"
        assert func.kind == SymbolKind.FUNCTION
        assert func.visibility == Visibility.PUBLIC
        assert "A documented function" in (func.doc or "")

    def test_parse_async_function(self, parser):
        """Test parsing an async function."""
        content = """
pub async fn async_fn() -> String {
    "hello".to_string()
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        func = result.symbols[0]
        assert func.name == "async_fn"
        assert func.is_async is True

    def test_parse_test_function(self, parser):
        """Test parsing a test function."""
        content = """
#[test]
fn test_something() {
    assert!(true);
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        func = result.symbols[0]
        assert func.name == "test_something"
        assert func.is_test is True
        assert "test" in func.attributes

    def test_parse_struct(self, parser):
        """Test parsing a struct."""
        content = """
/// A test struct.
#[derive(Debug, Clone)]
pub struct MyStruct<T, U> {
    pub field1: T,
    field2: U,
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        struct = result.symbols[0]
        assert struct.name == "MyStruct"
        assert struct.kind == SymbolKind.STRUCT
        assert struct.visibility == Visibility.PUBLIC
        assert "T" in struct.generic_params
        assert "U" in struct.generic_params

    def test_parse_enum(self, parser):
        """Test parsing an enum."""
        content = """
pub enum MyEnum {
    Variant1,
    Variant2(String),
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        enum = result.symbols[0]
        assert enum.name == "MyEnum"
        assert enum.kind == SymbolKind.ENUM
        assert enum.visibility == Visibility.PUBLIC

    def test_parse_trait(self, parser):
        """Test parsing a trait."""
        content = """
pub trait MyTrait {
    fn required(&self);
}
"""
        result = parser.parse_content("test.rs", content)
        # Should have trait and function inside
        assert any(s.name == "MyTrait" and s.kind == SymbolKind.TRAIT for s in result.symbols)

    def test_parse_impl(self, parser):
        """Test parsing an impl block."""
        content = """
impl MyStruct {
    pub fn new() -> Self {
        Self {}
    }
}
"""
        result = parser.parse_content("test.rs", content)
        assert any(s.kind == SymbolKind.IMPL for s in result.symbols)

    def test_parse_const(self, parser):
        """Test parsing a const."""
        content = """
pub const MAX_SIZE: usize = 1024;
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        const = result.symbols[0]
        assert const.name == "MAX_SIZE"
        assert const.kind == SymbolKind.CONST

    def test_parse_use(self, parser):
        """Test parsing use statements."""
        content = """
use std::collections::HashMap;
use std::io::{Read, Write};
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.imports) >= 1
        assert any("HashMap" in i.path for i in result.imports)

    def test_parse_visibility_crate(self, parser):
        """Test parsing pub(crate) visibility."""
        content = """
pub(crate) fn crate_only() -> i32 {
    42
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        func = result.symbols[0]
        assert func.visibility == Visibility.CRATE

    def test_parse_unsafe_function(self, parser):
        """Test parsing an unsafe function."""
        content = """
pub unsafe fn unsafe_fn() -> *mut u8 {
    std::ptr::null_mut()
}
"""
        result = parser.parse_content("test.rs", content)
        assert len(result.symbols) == 1

        func = result.symbols[0]
        assert func.is_unsafe is True


class TestFileSymbols:
    """Test FileSymbols data structure."""

    def test_to_dict(self, parser):
        """Test converting FileSymbols to dict."""
        content = """
pub fn my_function() {}
"""
        result = parser.parse_content("test.rs", content)
        d = result.to_dict()

        assert d["path"] == "test.rs"
        assert "symbols" in d
        assert len(d["symbols"]) == 1
        assert d["symbols"][0]["name"] == "my_function"


# ==================== Indexer Tests ====================


class TestIndexer:
    """Test Indexer building all indexes."""

    def test_find_workspace_crates(self, temp_workspace):
        """Test finding crates in workspace."""
        indexer = Indexer(temp_workspace)
        crates = indexer.find_workspace_crates()

        assert len(crates) == 1
        assert crates[0]["name"] == "test-crate"

    def test_find_rust_files(self, temp_workspace):
        """Test finding Rust files in a crate."""
        indexer = Indexer(temp_workspace)
        crate_path = temp_workspace / "crates" / "test-crate"

        files = indexer.find_rust_files(crate_path)
        assert len(files) == 2  # lib.rs and utils.rs

        file_names = {f.name for f in files}
        assert "lib.rs" in file_names
        assert "utils.rs" in file_names

    def test_build_symbol_index(self, temp_workspace):
        """Test building symbol index."""
        indexer = Indexer(temp_workspace)
        symbol_index = indexer.build_symbol_index()

        assert symbol_index.version == "2.0.0"
        assert len(symbol_index.files) >= 2

        # Find lib.rs symbols
        lib_file = next((f for f in symbol_index.files if "lib.rs" in f.path), None)
        assert lib_file is not None

        symbol_names = {s.name for s in lib_file.symbols}
        assert "TestStruct" in symbol_names
        assert "TestEnum" in symbol_names
        assert "TestTrait" in symbol_names
        assert "test_function" in symbol_names
        assert "async_function" in symbol_names

    def test_build_module_index(self, temp_workspace):
        """Test building module index."""
        indexer = Indexer(temp_workspace)
        module_index = indexer.build_module_index()

        assert module_index.version == "2.0.0"
        assert len(module_index.crates) == 1
        assert module_index.crates[0].crate_name == "test-crate"

        module_names = {m.name for m in module_index.crates[0].modules}
        assert "test-crate" in module_names  # lib.rs
        assert "test-crate::utils" in module_names

    def test_build_dependency_graph(self, temp_workspace):
        """Test building dependency graph."""
        indexer = Indexer(temp_workspace)
        dep_graph = indexer.build_dependency_graph()

        assert dep_graph.version == "2.0.0"
        assert len(dep_graph.crates) == 1

        crate_deps = dep_graph.crates[0]
        assert crate_deps.crate_name == "test-crate"

        dep_names = {d.name for d in crate_deps.dependencies}
        assert "serde" in dep_names

        dev_dep_names = {d.name for d in crate_deps.dev_dependencies}
        assert "tokio" in dev_dep_names

    def test_build_test_index(self, temp_workspace):
        """Test building test index."""
        indexer = Indexer(temp_workspace)
        test_index = indexer.build_test_index()

        assert test_index.version == "2.0.0"
        assert len(test_index.crates) == 1

        # Flatten all tests
        all_tests = []
        for crate in test_index.crates:
            for module in crate.modules:
                all_tests.extend(module.tests)

        test_names = {t.name for t in all_tests}
        assert "test_add" in test_names
        assert "test_ignored" in test_names
        assert "test_async" in test_names
        assert "test_integration" in test_names

        # Check ignored test is marked
        ignored_test = next((t for t in all_tests if t.name == "test_ignored"), None)
        assert ignored_test is not None
        assert ignored_test.is_ignored is True

    def test_build_all(self, temp_workspace):
        """Test building all indexes."""
        indexer = Indexer(temp_workspace)
        result = indexer.build_all()

        assert "symbols" in result
        assert "modules" in result
        assert "dependencies" in result
        assert "tests" in result

        # Verify structure
        assert result["symbols"]["version"] == "2.0.0"
        assert result["modules"]["version"] == "2.0.0"
        assert result["dependencies"]["version"] == "2.0.0"
        assert result["tests"]["version"] == "2.0.0"

    def test_build_all_with_output(self, temp_workspace):
        """Test building all indexes with file output."""
        import tempfile

        indexer = Indexer(temp_workspace)

        with tempfile.TemporaryDirectory() as output_dir:
            output_path = Path(output_dir)
            indexer.build_all(output_path)

            # Check files exist
            assert (output_path / "symbols.json").exists()
            assert (output_path / "modules.json").exists()
            assert (output_path / "dependencies.json").exists()
            assert (output_path / "tests.json").exists()

            # Check content is valid JSON
            import json

            symbols = json.loads((output_path / "symbols.json").read_text())
            assert "version" in symbols
            assert "files" in symbols


# ==================== Integration Tests ====================


class TestIndexerIntegration:
    """Integration tests with real Kelpie codebase."""

    @pytest.mark.skipif(
        not (Path(__file__).parent.parent.parent.parent.parent / "crates").exists(),
        reason="Kelpie codebase not found",
    )
    def test_index_kelpie_workspace(self):
        """Test indexing the actual Kelpie workspace."""
        kelpie_root = Path(__file__).parent.parent.parent.parent.parent
        indexer = Indexer(kelpie_root)

        # Just verify it runs without error
        crates = indexer.find_workspace_crates()
        assert len(crates) > 0

        # Build symbol index for a subset
        symbol_index = indexer.build_symbol_index()
        assert len(symbol_index.files) > 0
