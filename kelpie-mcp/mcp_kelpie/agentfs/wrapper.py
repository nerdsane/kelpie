"""
VerificationFS - AgentFS wrapper with verification semantics

Based on QuickHouse VDE implementation (VDE.md section 3.4, 4.4).
Extends Turso AgentFS SDK with verification-specific APIs.

Key features:
- Verified facts with evidence
- Invariant tracking
- TLA+ spec reading tracking
- Exploration logging
- Query caching with TTL
- Tool call trajectory (via AgentFS built-in)
"""

import hashlib
import time
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, AsyncIterator

from agentfs_sdk import AgentFS, AgentFSOptions


def _utcnow() -> datetime:
    """Get current UTC time (timezone-aware)."""
    return datetime.now(timezone.utc)


class VerificationFS:
    """
    Verification-driven extension of Turso AgentFS.

    Provides structured storage for:
    - Verified facts (claims + evidence + provenance)
    - Invariant verification status
    - TLA+ specs read
    - Exploration logs
    - Cached query results
    - Tool call trajectory (via AgentFS SDK)
    """

    # KV store prefixes for different verification entities
    PREFIX_SESSION = "vfs:session:"
    PREFIX_FACT = "vfs:fact:"
    PREFIX_INVARIANT = "vfs:invariant:"
    PREFIX_SPEC = "vfs:spec:"
    PREFIX_CACHE = "vfs:cache:"
    PREFIX_EXPLORATION = "vfs:exploration:"

    @classmethod
    @asynccontextmanager
    async def open(
        cls, session_id: str, task: str, project_root: str
    ) -> AsyncIterator["VerificationFS"]:
        """
        Open or resume a verification session.

        Args:
            session_id: Unique session identifier
            task: Description of current task
            project_root: Path to project root

        Yields:
            VerificationFS instance
        """
        # Store in .agentfs/ (not .claude/ like VDE - Kelpie uses .agentfs)
        db_path = Path(project_root) / ".agentfs" / f"agentfs-{session_id}.db"
        db_path.parent.mkdir(parents=True, exist_ok=True)

        # Open Turso AgentFS with SQLite backend
        afs = await AgentFS.open(AgentFSOptions(id=session_id, path=str(db_path)))

        vfs = cls(afs, session_id, task, project_root)

        # Initialize session metadata
        await vfs._init_session()

        try:
            yield vfs
        finally:
            # AgentFS handles cleanup
            await afs.close()

    def __init__(self, afs: AgentFS, session_id: str, task: str, project_root: str):
        """
        Initialize VerificationFS wrapper.

        Args:
            afs: AgentFS instance
            session_id: Session identifier
            task: Task description
            project_root: Project root path
        """
        self.afs = afs
        self.session_id = session_id
        self.task = task
        self.project_root = project_root

    async def _init_session(self):
        """Initialize session metadata."""
        session_key = f"{self.PREFIX_SESSION}current"
        session_data = {
            "id": self.session_id,
            "task": self.task,
            "started_at": _utcnow().isoformat(),
            "project_root": str(self.project_root),
        }
        await self.afs.kv.set(session_key, session_data)

    # ==================== Verified Facts ====================

    async def add_fact(
        self,
        claim: str,
        evidence: str,
        source: str,
        command: str | None = None,
        query: str | None = None,
    ) -> str:
        """
        Record a verified fact with evidence.

        Args:
            claim: The claim being verified
            evidence: Evidence supporting the claim
            source: Source of verification (e.g., "dst", "code_review", "datadog")
            command: Command executed to produce evidence (optional)
            query: Query used to produce evidence (optional)

        Returns:
            Fact ID
        """
        fact_id = f"{int(time.time() * 1000)}"
        fact = {
            "id": fact_id,
            "claim": claim,
            "evidence": evidence,
            "source": source,
            "timestamp": _utcnow().isoformat(),
            "command": command,
            "query": query,
        }
        await self.afs.kv.set(f"{self.PREFIX_FACT}{fact_id}", fact)
        return fact_id

    async def check_fact(self, claim_pattern: str) -> list[dict[str, Any]]:
        """
        Check if a claim has been verified.

        Args:
            claim_pattern: Pattern to search for in claims

        Returns:
            List of matching facts
        """
        # kv.list returns List[Dict] with 'key' and 'value' fields
        items = await self.afs.kv.list(self.PREFIX_FACT)
        facts = []
        for item in items:
            fact = item.get("value")
            if fact and claim_pattern.lower() in fact.get("claim", "").lower():
                facts.append(fact)
        return facts

    async def list_facts(self) -> list[dict[str, Any]]:
        """
        List all verified facts in chronological order.

        Returns:
            List of all facts
        """
        items = await self.afs.kv.list(self.PREFIX_FACT)
        facts = [item["value"] for item in items if item.get("value")]

        # Sort by timestamp
        facts.sort(key=lambda f: f.get("timestamp", ""), reverse=True)
        return facts

    # ==================== Invariant Tracking ====================

    async def verify_invariant(
        self,
        name: str,
        component: str,
        method: str = "dst",
        evidence: str | None = None,
    ):
        """
        Mark an invariant as verified.

        Args:
            name: Invariant name (e.g., "AtomicVisibility")
            component: Component name (e.g., "compaction")
            method: Verification method ("dst", "stateright", "kani", "manual")
            evidence: Evidence of verification (e.g., "23 tests passed")
        """
        inv = {
            "name": name,
            "component": component,
            "method": method,
            "verified_at": _utcnow().isoformat(),
            "evidence": evidence,
        }
        key = f"{self.PREFIX_INVARIANT}{component}:{name}"
        await self.afs.kv.set(key, inv)

    async def invariant_status(self, component: str) -> dict[str, Any]:
        """
        Check which invariants have been verified for a component.

        Args:
            component: Component name

        Returns:
            Dict with "verified" and "verified_count"
        """
        # Get all verified invariants for this component
        prefix = f"{self.PREFIX_INVARIANT}{component}:"
        items = await self.afs.kv.list(prefix)
        verified = [item["value"] for item in items if item.get("value")]

        return {
            "component": component,
            "verified": verified,
            "verified_count": len(verified),
        }

    # ==================== TLA+ Spec Tracking ====================

    async def spec_read(
        self, name: str, path: str, description: str | None = None, invariants: str | None = None
    ):
        """
        Record that a TLA+ spec was read.

        Args:
            name: Spec name (e.g., "CompactionProtocol")
            path: Path to spec file
            description: Brief description
            invariants: Comma-separated list of invariant names
        """
        spec = {
            "name": name,
            "path": path,
            "description": description,
            "invariants": invariants,
            "read_at": _utcnow().isoformat(),
        }
        await self.afs.kv.set(f"{self.PREFIX_SPEC}{name}", spec)

    async def list_specs(self) -> list[dict[str, Any]]:
        """List all TLA+ specs that have been read."""
        items = await self.afs.kv.list(self.PREFIX_SPEC)
        return [item["value"] for item in items if item.get("value")]

    # ==================== Exploration Logging ====================

    async def exploration_log(self, action: str, target: str, result: str | None = None):
        """
        Log an exploration action.

        Args:
            action: Action type ("read", "execute", "query")
            target: Target of action (file path, query, etc.)
            result: Result summary
        """
        log_id = f"{int(time.time() * 1000)}"
        log = {
            "id": log_id,
            "action": action,
            "target": target,
            "result": result,
            "timestamp": _utcnow().isoformat(),
        }
        await self.afs.kv.set(f"{self.PREFIX_EXPLORATION}{log_id}", log)

    async def list_explorations(self) -> list[dict[str, Any]]:
        """List all exploration logs."""
        items = await self.afs.kv.list(self.PREFIX_EXPLORATION)
        logs = [item["value"] for item in items if item.get("value")]

        # Sort by timestamp
        logs.sort(key=lambda l: l.get("timestamp", ""), reverse=True)
        return logs

    # ==================== Query Caching ====================

    async def cache_query(
        self, query: str, result: dict[str, Any], ttl_minutes: int = 30, query_type: str = "sql"
    ):
        """
        Cache a query result with TTL.

        Args:
            query: Query string
            result: Query result
            ttl_minutes: Time to live in minutes
            query_type: Type of query ("sql", "ddsql", "api")
        """
        cache_key = hashlib.sha256(query.encode()).hexdigest()[:16]
        entry = {
            "query": query,
            "query_type": query_type,
            "result": result,
            "timestamp": _utcnow().isoformat(),
            "ttl_minutes": ttl_minutes,
        }
        await self.afs.kv.set(f"{self.PREFIX_CACHE}{cache_key}", entry)

    async def get_cached_query(self, query: str) -> dict[str, Any] | None:
        """
        Get cached query result if not expired.

        Args:
            query: Query string

        Returns:
            Cached result or None if not found/expired
        """
        cache_key = hashlib.sha256(query.encode()).hexdigest()[:16]
        entry = await self.afs.kv.get(f"{self.PREFIX_CACHE}{cache_key}")

        if not entry:
            return None

        # Check TTL
        timestamp = datetime.fromisoformat(entry["timestamp"].replace("Z", "+00:00"))
        ttl_minutes = entry.get("ttl_minutes", 30)
        age_minutes = (_utcnow() - timestamp).total_seconds() / 60

        if age_minutes > ttl_minutes:
            # Expired
            return None

        return entry.get("result")

    # ==================== Session Status ====================

    async def status(self) -> dict[str, Any]:
        """
        Get current session status.

        Returns:
            Dict with counts of facts, invariants, specs, explorations, tool calls
        """
        # Count facts
        fact_items = await self.afs.kv.list(self.PREFIX_FACT)
        fact_count = len(fact_items)

        # Count invariants
        inv_items = await self.afs.kv.list(self.PREFIX_INVARIANT)
        inv_count = len(inv_items)

        # Count specs
        spec_items = await self.afs.kv.list(self.PREFIX_SPEC)
        spec_count = len(spec_items)

        # Count explorations
        expl_items = await self.afs.kv.list(self.PREFIX_EXPLORATION)
        expl_count = len(expl_items)

        # Get tool call count from AgentFS
        # get_recent(since=0) gets all tool calls
        tool_calls = await self.afs.tools.get_recent(0)
        tool_count = len(tool_calls)

        return {
            "session_id": self.session_id,
            "task": self.task,
            "facts": fact_count,
            "invariants": inv_count,
            "specs_read": spec_count,
            "explorations": expl_count,
            "tool_calls": tool_count,
        }

    async def export(self) -> dict[str, Any]:
        """
        Export entire session for replay/analysis.

        Returns:
            Dict with all session data
        """
        tool_calls = await self.afs.tools.get_recent(0)
        # Convert ToolCall objects to dicts if needed
        tool_calls_data = [
            tc if isinstance(tc, dict) else {"id": tc.id, "name": tc.name, "parameters": tc.parameters}
            for tc in tool_calls
        ]

        return {
            "session_id": self.session_id,
            "task": self.task,
            "facts": await self.list_facts(),
            "specs": await self.list_specs(),
            "explorations": await self.list_explorations(),
            "tool_calls": tool_calls_data,
            "export_time": _utcnow().isoformat(),
        }

    # ==================== Tool Call Trajectory (AgentFS SDK) ====================
    # These are direct pass-throughs to AgentFS's built-in tool tracking

    async def tool_start(self, name: str, args: dict[str, Any]) -> int:
        """
        Start tracking a tool call.

        Args:
            name: Tool name
            args: Tool arguments

        Returns:
            Call ID (integer) for later reference
        """
        return await self.afs.tools.start(name, args)

    async def tool_success(self, call_id: int, result: Any):
        """
        Mark tool call as successful.

        Args:
            call_id: Call ID from tool_start (integer)
            result: Tool result
        """
        await self.afs.tools.success(call_id, result)

    async def tool_error(self, call_id: int, error: str):
        """
        Mark tool call as failed.

        Args:
            call_id: Call ID from tool_start (integer)
            error: Error message
        """
        await self.afs.tools.error(call_id, error)

    async def tool_list(self) -> list[dict[str, Any]]:
        """
        List all tool calls with timing.

        Returns:
            List of tool calls with timestamps and durations
        """
        tool_calls = await self.afs.tools.get_recent(0)
        # Convert ToolCall objects to dicts
        return [
            tc if isinstance(tc, dict) else {"id": tc.id, "name": tc.name, "parameters": tc.parameters}
            for tc in tool_calls
        ]

    # ==================== Generic KV Operations ====================
    # For examination tools and other use cases that need raw KV access

    async def kv_set(self, key: str, value: Any):
        """
        Set a key-value pair in the KV store.

        Args:
            key: Key name
            value: Value (string or JSON-serializable)
        """
        await self.afs.kv.set(key, value)

    async def kv_get(self, key: str) -> Any | None:
        """
        Get a value from the KV store.

        Args:
            key: Key name

        Returns:
            Value or None if not found
        """
        return await self.afs.kv.get(key)
