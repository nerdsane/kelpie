"""
Tests for MCP tools module.

Tests cover:
- Tool definitions
- Handler routing
- Individual tool handlers
"""

import asyncio
import pytest
import tempfile
from pathlib import Path

from mcp_kelpie.tools import ALL_TOOLS, ToolHandlers
from mcp_kelpie.tools.definitions import (
    REPL_TOOLS,
    AGENTFS_TOOLS,
    INDEX_TOOLS,
    VERIFICATION_TOOLS,
    DST_TOOLS,
    CODEBASE_TOOLS,
    EXAMINATION_TOOLS,
)


# ==================== Tool Definitions Tests ====================


class TestToolDefinitions:
    """Test tool definition schemas."""

    def test_all_tools_count(self):
        """Verify total tool count."""
        expected = (
            len(REPL_TOOLS)
            + len(AGENTFS_TOOLS)
            + len(INDEX_TOOLS)
            + len(VERIFICATION_TOOLS)
            + len(DST_TOOLS)
            + len(CODEBASE_TOOLS)
            + len(EXAMINATION_TOOLS)
        )
        assert len(ALL_TOOLS) == expected

    def test_repl_tools_names(self):
        """Verify REPL tool names."""
        names = {t["name"] for t in REPL_TOOLS}
        assert "repl_load" in names
        assert "repl_exec" in names
        assert "repl_query" in names
        assert "repl_state" in names
        assert "repl_clear" in names

    def test_vfs_tools_names(self):
        """Verify VFS/AgentFS tool names."""
        names = {t["name"] for t in AGENTFS_TOOLS}
        assert "vfs_init" in names
        assert "vfs_fact_add" in names
        assert "vfs_status" in names
        assert "vfs_tool_start" in names

    def test_index_tools_names(self):
        """Verify index tool names."""
        names = {t["name"] for t in INDEX_TOOLS}
        assert "index_symbols" in names
        assert "index_tests" in names
        assert "index_refresh" in names

    def test_verify_tools_names(self):
        """Verify verification tools were removed."""
        # Verification tools removed - use Claude's Bash tool instead
        assert len(VERIFICATION_TOOLS) == 0

    def test_dst_tools_names(self):
        """Verify DST tools were removed."""
        # DST tools removed - use RLM/REPL instead
        assert len(DST_TOOLS) == 0

    def test_codebase_tools_names(self):
        """Verify codebase tools were removed."""
        # Codebase tools removed - use Claude's built-in tools (Read, Grep, Glob) instead
        assert len(CODEBASE_TOOLS) == 0

    def test_examination_tools_names(self):
        """Verify examination tool names."""
        names = {t["name"] for t in EXAMINATION_TOOLS}
        assert "exam_start" in names
        assert "exam_record" in names
        assert "exam_status" in names
        assert "exam_complete" in names
        assert "exam_export" in names
        assert "issue_list" in names
        assert len(EXAMINATION_TOOLS) == 6

    def test_tool_schema_format(self):
        """Verify all tools have required schema fields."""
        for tool in ALL_TOOLS:
            assert "name" in tool, f"Tool missing name: {tool}"
            assert "description" in tool, f"Tool missing description: {tool}"
            assert "inputSchema" in tool, f"Tool missing inputSchema: {tool}"
            assert tool["inputSchema"]["type"] == "object"


# ==================== Handler Routing Tests ====================


@pytest.fixture
def temp_codebase():
    """Create a temporary codebase for testing."""
    with tempfile.TemporaryDirectory() as tmpdir:
        root = Path(tmpdir)

        # Create a minimal Rust project structure
        src_dir = root / "src"
        src_dir.mkdir()

        # Create lib.rs
        (src_dir / "lib.rs").write_text("""
//! Test library

pub fn test_function() -> i32 {
    42
}

#[test]
fn test_it_works() {
    assert_eq!(test_function(), 42);
}
""")

        # Create Cargo.toml
        (root / "Cargo.toml").write_text("""
[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
""")

        yield root


class TestHandlerRouting:
    """Test handler routing logic."""

    def test_handlers_init(self, temp_codebase):
        """Test handlers initialization."""
        handlers = ToolHandlers(temp_codebase)
        # Resolve both paths to handle macOS /var -> /private/var symlink
        assert handlers.codebase_path.resolve() == temp_codebase.resolve()

    @pytest.mark.asyncio
    async def test_handle_unknown_tool(self, temp_codebase):
        """Test handling of unknown tool."""
        handlers = ToolHandlers(temp_codebase)

        with pytest.raises(ValueError, match="Unknown tool"):
            await handlers.handle_tool("nonexistent_tool", {})

    @pytest.mark.asyncio
    async def test_handle_repl_state(self, temp_codebase):
        """Test repl_state handler."""
        handlers = ToolHandlers(temp_codebase)
        result = await handlers.handle_tool("repl_state", {})

        assert "variables" in result
        assert "total_size_bytes" in result

    @pytest.mark.asyncio
    async def test_handle_vfs_init(self, temp_codebase):
        """Test vfs_init handler."""
        handlers = ToolHandlers(temp_codebase)
        result = await handlers.handle_tool("vfs_init", {"task": "Test task"})

        assert "session_id" in result
        assert result["task"] == "Test task"

    @pytest.mark.asyncio
    async def test_handle_index_status(self, temp_codebase):
        """Test index_status handler."""
        handlers = ToolHandlers(temp_codebase)
        result = await handlers.handle_tool("index_status", {})

        assert result["success"]
        assert "status" in result


# ==================== REPL Tools Tests ====================


class TestReplTools:
    """Test REPL-related tools."""

    @pytest.mark.asyncio
    async def test_repl_load_and_query(self, temp_codebase):
        """Test repl_load and repl_query handlers."""
        handlers = ToolHandlers(temp_codebase)

        # Load files
        result = await handlers.handle_tool("repl_load", {
            "pattern": "**/*.rs",
            "var_name": "code"
        })
        assert result["success"]

        # Query loaded files
        result = await handlers.handle_tool("repl_query", {
            "expression": "len(code)"
        })
        assert result["success"]
        assert result["result"] >= 1

    @pytest.mark.asyncio
    async def test_repl_exec(self, temp_codebase):
        """Test repl_exec handler."""
        handlers = ToolHandlers(temp_codebase)

        # Load files first
        await handlers.handle_tool("repl_load", {
            "pattern": "**/*.rs",
            "var_name": "code"
        })

        # Execute code
        result = await handlers.handle_tool("repl_exec", {
            "code": "result = sum(len(c) for c in code.values())"
        })
        assert result["success"]
        assert result["result"] > 0

    @pytest.mark.asyncio
    async def test_repl_clear(self, temp_codebase):
        """Test repl_clear handler."""
        handlers = ToolHandlers(temp_codebase)

        # Load and clear
        await handlers.handle_tool("repl_load", {
            "pattern": "**/*.rs",
            "var_name": "code"
        })
        result = await handlers.handle_tool("repl_clear", {"var_name": "code"})

        assert result["success"]
        assert "freed" in result["message"]


# ==================== VFS Tools Tests ====================


class TestVfsTools:
    """Test VFS/AgentFS-related tools."""

    @pytest.mark.asyncio
    async def test_vfs_workflow(self, temp_codebase):
        """Test VFS workflow: init -> add fact -> check fact."""
        handlers = ToolHandlers(temp_codebase)

        # Initialize session
        result = await handlers.handle_tool("vfs_init", {"task": "Test workflow"})
        assert "session_id" in result

        # Add a fact
        result = await handlers.handle_tool("vfs_fact_add", {
            "claim": "Tests pass",
            "evidence": "cargo test output",
            "source": "test"
        })
        assert result["success"]

        # Check the fact
        result = await handlers.handle_tool("vfs_fact_check", {
            "claim_pattern": "Tests"
        })
        assert result["success"]
        assert result["count"] >= 1
        assert "facts" in result

    @pytest.mark.asyncio
    async def test_vfs_tool_tracking(self, temp_codebase):
        """Test VFS tool call tracking."""
        handlers = ToolHandlers(temp_codebase)

        # Initialize session
        await handlers.handle_tool("vfs_init", {"task": "Tool tracking test"})

        # Start a tool call
        result = await handlers.handle_tool("vfs_tool_start", {
            "name": "test_tool",
            "args": {"key": "value"}
        })
        assert "call_id" in result
        call_id = result["call_id"]

        # Mark success
        result = await handlers.handle_tool("vfs_tool_success", {
            "call_id": call_id,
            "result": "success"
        })
        assert result["success"]

        # List tool calls
        result = await handlers.handle_tool("vfs_tool_list", {})
        assert result["success"]
        assert "tools" in result
        assert len(result["tools"]) >= 1


# ==================== DST Tools Tests ====================


class TestDstTools:
    """Test DST-related tools (removed - use RLM instead)."""

    def test_dst_tools_removed(self):
        """Verify DST tools were removed in favor of RLM primitives."""
        from mcp_kelpie.tools.definitions import DST_TOOLS

        # DST_TOOLS should be empty
        assert len(DST_TOOLS) == 0, "DST tools should be removed - use RLM/REPL instead"


# ==================== Examination Tools Tests ====================


class TestExaminationTools:
    """Test examination-related tools for thorough codebase understanding."""

    @pytest.mark.asyncio
    async def test_exam_start(self, temp_codebase):
        """Test exam_start handler."""
        handlers = ToolHandlers(temp_codebase)

        result = await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        assert result["success"]
        assert result["task"] == "Test examination"
        assert result["total_components"] == 2
        assert "component-a" in result["scope"]
        assert "component-b" in result["scope"]

    @pytest.mark.asyncio
    async def test_exam_start_requires_scope(self, temp_codebase):
        """Test exam_start with empty scope."""
        handlers = ToolHandlers(temp_codebase)

        result = await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": []
        })

        # Empty scope should still work but have 0 components
        assert result["success"]
        assert result["total_components"] == 0

    @pytest.mark.asyncio
    async def test_exam_record(self, temp_codebase):
        """Test exam_record handler."""
        handlers = ToolHandlers(temp_codebase)

        # Start examination first
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        # Record findings for a component
        result = await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A test component",
            "details": "Detailed explanation of component-a",
            "connections": ["component-b"],
            "issues": [
                {
                    "severity": "medium",
                    "description": "Missing tests",
                    "evidence": "No test files found"
                }
            ]
        })

        assert result["success"]
        assert result["component"] == "component-a"
        assert result["issues_found"] == 1
        assert "1/2" in result["progress"]
        assert result["remaining_count"] == 1

    @pytest.mark.asyncio
    async def test_exam_record_without_start(self, temp_codebase):
        """Test exam_record without starting examination."""
        handlers = ToolHandlers(temp_codebase)

        result = await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A test component"
        })

        assert not result["success"]
        assert "exam_start" in result["error"]

    @pytest.mark.asyncio
    async def test_exam_status(self, temp_codebase):
        """Test exam_status handler."""
        handlers = ToolHandlers(temp_codebase)

        # Start examination
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        # Check initial status
        result = await handlers.handle_tool("exam_status", {})

        assert result["success"]
        assert result["examined_count"] == 0
        assert result["remaining_count"] == 2
        assert not result["is_complete"]

        # Record one component
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A test component"
        })

        # Check updated status
        result = await handlers.handle_tool("exam_status", {})

        assert result["examined_count"] == 1
        assert result["remaining_count"] == 1
        assert "component-a" in result["examined"]
        assert "component-b" in result["remaining"]

    @pytest.mark.asyncio
    async def test_exam_complete_incomplete(self, temp_codebase):
        """Test exam_complete when examination is incomplete."""
        handlers = ToolHandlers(temp_codebase)

        # Start examination
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        # Record only one component
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A test component"
        })

        # Check completion
        result = await handlers.handle_tool("exam_complete", {})

        assert result["success"]
        assert not result["complete"]
        assert not result["can_answer"]
        assert "component-b" in result["remaining"]

    @pytest.mark.asyncio
    async def test_exam_complete_complete(self, temp_codebase):
        """Test exam_complete when examination is complete."""
        handlers = ToolHandlers(temp_codebase)

        # Start examination
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        # Record all components
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "Component A"
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-b",
            "summary": "Component B"
        })

        # Check completion
        result = await handlers.handle_tool("exam_complete", {})

        assert result["success"]
        assert result["complete"]
        assert result["can_answer"]
        assert result["examined_count"] == 2

    @pytest.mark.asyncio
    async def test_exam_export(self, temp_codebase):
        """Test exam_export handler."""
        handlers = ToolHandlers(temp_codebase)

        # Start and complete examination
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a"]
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A test component",
            "details": "Detailed explanation",
            "connections": [],
            "issues": [
                {"severity": "high", "description": "Issue 1", "evidence": "Found here"}
            ]
        })

        # Export
        result = await handlers.handle_tool("exam_export", {"include_details": True})

        assert result["success"]
        assert "output_dir" in result
        assert "files" in result
        assert result["component_count"] == 1
        assert result["issue_count"] == 1

        # Verify files were created
        output_dir = Path(result["output_dir"])
        assert output_dir.exists()
        assert (output_dir / "MAP.md").exists()
        assert (output_dir / "ISSUES.md").exists()
        assert (output_dir / "components").exists()

        # Check MAP.md content
        map_content = (output_dir / "MAP.md").read_text()
        assert "component-a" in map_content
        assert "A test component" in map_content

        # Check ISSUES.md content
        issues_content = (output_dir / "ISSUES.md").read_text()
        assert "Issue 1" in issues_content
        assert "HIGH" in issues_content

    @pytest.mark.asyncio
    async def test_issue_list(self, temp_codebase):
        """Test issue_list handler."""
        handlers = ToolHandlers(temp_codebase)

        # Start examination
        await handlers.handle_tool("exam_start", {
            "task": "Test examination",
            "scope": ["component-a", "component-b"]
        })

        # Record components with issues
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "Component A",
            "issues": [
                {"severity": "high", "description": "High issue"},
                {"severity": "medium", "description": "Medium issue"}
            ]
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-b",
            "summary": "Component B",
            "issues": [
                {"severity": "low", "description": "Low issue"}
            ]
        })

        # List all issues
        result = await handlers.handle_tool("issue_list", {})

        assert result["success"]
        assert result["count"] == 3
        assert result["by_severity"]["high"] == 1
        assert result["by_severity"]["medium"] == 1
        assert result["by_severity"]["low"] == 1

    @pytest.mark.asyncio
    async def test_issue_list_filter_by_component(self, temp_codebase):
        """Test issue_list with component filter."""
        handlers = ToolHandlers(temp_codebase)

        # Start and record
        await handlers.handle_tool("exam_start", {
            "task": "Test",
            "scope": ["component-a", "component-b"]
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A",
            "issues": [{"severity": "high", "description": "Issue A"}]
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-b",
            "summary": "B",
            "issues": [{"severity": "low", "description": "Issue B"}]
        })

        # Filter by component
        result = await handlers.handle_tool("issue_list", {
            "component": "component-a"
        })

        assert result["count"] == 1
        assert result["issues"][0]["description"] == "Issue A"

    @pytest.mark.asyncio
    async def test_issue_list_filter_by_severity(self, temp_codebase):
        """Test issue_list with severity filter."""
        handlers = ToolHandlers(temp_codebase)

        # Start and record
        await handlers.handle_tool("exam_start", {
            "task": "Test",
            "scope": ["component-a"]
        })
        await handlers.handle_tool("exam_record", {
            "component": "component-a",
            "summary": "A",
            "issues": [
                {"severity": "critical", "description": "Critical issue"},
                {"severity": "low", "description": "Low issue"}
            ]
        })

        # Filter by severity
        result = await handlers.handle_tool("issue_list", {
            "severity": "critical"
        })

        assert result["count"] == 1
        assert result["issues"][0]["severity"] == "critical"

    @pytest.mark.asyncio
    async def test_exam_full_workflow(self, temp_codebase):
        """Test complete examination workflow end-to-end."""
        handlers = ToolHandlers(temp_codebase)

        # Step 1: Start examination
        result = await handlers.handle_tool("exam_start", {
            "task": "Full workflow test",
            "scope": ["core", "storage"]
        })
        assert result["success"]
        assert result["total_components"] == 2

        # Step 2: Check initial status
        result = await handlers.handle_tool("exam_status", {})
        assert result["remaining_count"] == 2
        assert not result["is_complete"]

        # Step 3: Verify not complete
        result = await handlers.handle_tool("exam_complete", {})
        assert not result["can_answer"]

        # Step 4: Record findings for core
        result = await handlers.handle_tool("exam_record", {
            "component": "core",
            "summary": "Core types and traits",
            "details": "Defines ActorId, Error, Result types",
            "connections": ["storage"],
            "issues": []
        })
        assert result["progress"] == "1/2"

        # Step 5: Record findings for storage
        result = await handlers.handle_tool("exam_record", {
            "component": "storage",
            "summary": "Per-actor KV storage",
            "details": "SQLite-based storage with WAL mode",
            "connections": ["core"],
            "issues": [
                {"severity": "medium", "description": "Missing compaction tests"}
            ]
        })
        assert result["progress"] == "2/2"
        assert result["remaining_count"] == 0

        # Step 6: Verify complete
        result = await handlers.handle_tool("exam_complete", {})
        assert result["complete"]
        assert result["can_answer"]

        # Step 7: Export results
        result = await handlers.handle_tool("exam_export", {})
        assert result["success"]
        assert result["component_count"] == 2
        assert result["issue_count"] == 1

        # Step 8: Query issues
        result = await handlers.handle_tool("issue_list", {})
        assert result["count"] == 1
        assert result["by_severity"]["medium"] == 1
