"""
Session management for VerificationFS

Handles session lifecycle, tracking active sessions, and cleanup.
"""

import uuid
from typing import Any, Optional

from .wrapper import VerificationFS


class SessionManager:
    """
    Manage VerificationFS sessions.

    Provides:
    - Session creation with unique IDs
    - Active session tracking
    - Session resumption
    - Cleanup
    """

    def __init__(self, project_root: str):
        """
        Initialize session manager.

        Args:
            project_root: Path to project root
        """
        self.project_root = project_root
        self._active_session: Optional[VerificationFS] = None
        self._session_id: Optional[str] = None
        self._context_manager: Optional[Any] = None

    def generate_session_id(self) -> str:
        """
        Generate a unique session ID.

        Returns:
            Session ID (short hex string)
        """
        return uuid.uuid4().hex[:12]

    async def init_session(self, task: str, session_id: Optional[str] = None) -> VerificationFS:
        """
        Initialize a new session or resume existing one.

        Args:
            task: Task description
            session_id: Existing session ID (if resuming), or None for new

        Returns:
            VerificationFS instance
        """
        # Close any existing session first
        if self._active_session:
            await self.close_session()

        if not session_id:
            session_id = self.generate_session_id()

        self._session_id = session_id

        # Open VerificationFS (creates or resumes session)
        # Store the context manager to properly call __aexit__ later
        self._context_manager = VerificationFS.open(session_id, task, self.project_root)
        vfs = await self._context_manager.__aenter__()
        self._active_session = vfs

        return vfs

    def get_active_session(self) -> Optional[VerificationFS]:
        """
        Get the currently active session.

        Returns:
            Active VerificationFS instance or None
        """
        return self._active_session

    def get_session_id(self) -> Optional[str]:
        """
        Get the current session ID.

        Returns:
            Session ID or None
        """
        return self._session_id

    async def close_session(self):
        """Close the active session."""
        if self._context_manager:
            # Properly call __aexit__ to clean up
            await self._context_manager.__aexit__(None, None, None)
            self._context_manager = None
            self._active_session = None
            self._session_id = None
