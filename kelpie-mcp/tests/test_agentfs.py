"""
Tests for AgentFS wrapper (VerificationFS)

Validates:
- Session initialization
- Fact recording and retrieval
- Invariant tracking
- Spec tracking
- Exploration logging
- Query caching
- Tool call tracking
"""

import asyncio
import tempfile
from pathlib import Path

import pytest

from mcp_kelpie.agentfs import SessionManager, VerificationFS


@pytest.fixture
async def temp_project():
    """Create temporary project directory."""
    with tempfile.TemporaryDirectory() as tmpdir:
        yield tmpdir


@pytest.fixture
async def vfs(temp_project):
    """Create VerificationFS instance for testing."""
    async with VerificationFS.open(
        session_id="test_session", task="test task", project_root=temp_project
    ) as vfs:
        yield vfs


class TestVerificationFS:
    """Test VerificationFS wrapper."""

    async def test_session_initialization(self, temp_project):
        """Test session creation and metadata."""
        async with VerificationFS.open(
            session_id="test_init", task="test initialization", project_root=temp_project
        ) as vfs:
            assert vfs.session_id == "test_init"
            assert vfs.task == "test initialization"

            status = await vfs.status()
            assert status["session_id"] == "test_init"
            assert status["task"] == "test initialization"

    async def test_add_and_list_facts(self, vfs):
        """Test fact recording and retrieval."""
        # Add a fact
        fact_id = await vfs.add_fact(
            claim="Test claim", evidence="Test evidence", source="test", command="test command"
        )

        assert fact_id is not None

        # List facts
        facts = await vfs.list_facts()
        assert len(facts) == 1
        assert facts[0]["claim"] == "Test claim"
        assert facts[0]["evidence"] == "Test evidence"
        assert facts[0]["source"] == "test"
        assert facts[0]["command"] == "test command"

    async def test_check_fact(self, vfs):
        """Test fact searching."""
        # Add facts
        await vfs.add_fact(claim="First claim", evidence="Evidence 1", source="test")
        await vfs.add_fact(claim="Second claim", evidence="Evidence 2", source="test")

        # Check for pattern
        results = await vfs.check_fact("First")
        assert len(results) == 1
        assert results[0]["claim"] == "First claim"

    async def test_invariant_tracking(self, vfs):
        """Test invariant verification tracking."""
        # Verify invariant
        await vfs.verify_invariant(
            name="TestInvariant", component="test_component", method="dst", evidence="All tests passed"
        )

        # Check status
        status = await vfs.invariant_status("test_component")
        assert status["verified_count"] == 1
        assert len(status["verified"]) == 1
        assert status["verified"][0]["name"] == "TestInvariant"
        assert status["verified"][0]["evidence"] == "All tests passed"

    async def test_spec_tracking(self, vfs):
        """Test TLA+ spec reading tracking."""
        # Record spec reading
        await vfs.spec_read(
            name="TestSpec",
            path="specs/tla/Test.tla",
            description="Test specification",
            invariants="Inv1, Inv2",
        )

        # List specs
        specs = await vfs.list_specs()
        assert len(specs) == 1
        assert specs[0]["name"] == "TestSpec"
        assert specs[0]["invariants"] == "Inv1, Inv2"

    async def test_exploration_logging(self, vfs):
        """Test exploration action logging."""
        # Log exploration
        await vfs.exploration_log(action="read", target="test.rs", result="Found 3 functions")

        # List explorations
        logs = await vfs.list_explorations()
        assert len(logs) == 1
        assert logs[0]["action"] == "read"
        assert logs[0]["target"] == "test.rs"
        assert logs[0]["result"] == "Found 3 functions"

    async def test_query_caching(self, vfs):
        """Test query result caching with TTL."""
        query = "SELECT * FROM test"
        result = {"data": [1, 2, 3]}

        # Cache query
        await vfs.cache_query(query, result, ttl_minutes=30)

        # Retrieve cached
        cached = await vfs.get_cached_query(query)
        assert cached is not None
        assert cached == result

        # Different query returns None
        other = await vfs.get_cached_query("SELECT * FROM other")
        assert other is None

    async def test_tool_call_tracking(self, vfs):
        """Test tool call trajectory tracking."""
        # Start tool call
        call_id = await vfs.tool_start("test_tool", {"arg1": "value1"})
        assert call_id is not None

        # Mark success
        await vfs.tool_success(call_id, {"result": "success"})

        # List tool calls
        calls = await vfs.tool_list()
        assert len(calls) > 0

    async def test_status(self, vfs):
        """Test session status reporting."""
        # Add some data
        await vfs.add_fact("Claim", "Evidence", "test")
        await vfs.verify_invariant("Inv1", "comp1")
        await vfs.spec_read("Spec1", "path/to/spec.tla")
        await vfs.exploration_log("read", "file.rs")

        # Check status
        status = await vfs.status()
        assert status["facts"] == 1
        assert status["invariants"] == 1
        assert status["specs_read"] == 1
        assert status["explorations"] == 1

    async def test_export(self, vfs):
        """Test session export."""
        # Add data
        await vfs.add_fact("Test claim", "Test evidence", "test")

        # Export
        export = await vfs.export()
        assert export["session_id"] == vfs.session_id
        assert export["task"] == vfs.task
        assert len(export["facts"]) == 1
        assert "export_time" in export


class TestSessionManager:
    """Test SessionManager."""

    async def test_generate_session_id(self, temp_project):
        """Test session ID generation."""
        manager = SessionManager(temp_project)
        sid1 = manager.generate_session_id()
        sid2 = manager.generate_session_id()

        assert sid1 != sid2
        assert len(sid1) == 12
        assert len(sid2) == 12

    async def test_init_session(self, temp_project):
        """Test session initialization."""
        manager = SessionManager(temp_project)
        vfs = await manager.init_session("test task")

        assert vfs is not None
        assert manager.get_session_id() is not None
        assert manager.get_active_session() == vfs

        await manager.close_session()

    async def test_resume_session(self, temp_project):
        """Test session resumption."""
        manager = SessionManager(temp_project)

        # Create session
        vfs1 = await manager.init_session("task 1", session_id="test_resume")
        await vfs1.add_fact("Fact 1", "Evidence 1", "test")
        await manager.close_session()

        # Resume session
        vfs2 = await manager.init_session("task 1 resumed", session_id="test_resume")
        facts = await vfs2.list_facts()
        assert len(facts) == 1
        assert facts[0]["claim"] == "Fact 1"

        await manager.close_session()
