"""
Kelpie MCP Server - VDE-aligned single Python server

Combines RLM, AgentFS, Indexer, and Verification tools in one MCP server.
Based on QuickHouse VDE implementation (.progress/VDE.md).

Architecture:
- RLM: Python REPL with recursive LLM calls
- AgentFS: Persistent state via Turso AgentFS SDK
- Indexer: tree-sitter structural analysis
- Verification: CLI execution tracking
"""

import asyncio
import json
import logging
import os
import sys
from pathlib import Path
from typing import Any

# Load environment variables from .env file
from dotenv import load_dotenv
load_dotenv()  # Load from .env in current directory
load_dotenv(Path(__file__).parent.parent.parent / ".env")  # Also try project root (kelpie/.env)

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

from .tools import ALL_TOOLS, ToolHandlers

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    stream=sys.stderr,
)
logger = logging.getLogger("mcp-kelpie")


def create_server() -> tuple[Server, ToolHandlers]:
    """Create and configure MCP server.

    Returns:
        Tuple of (server, handlers)
    """
    # Get codebase path from environment or current directory
    codebase_path = Path(os.getenv("KELPIE_CODEBASE_PATH", os.getcwd())).resolve()

    logger.info(f"Kelpie MCP Server initializing")
    logger.info(f"Codebase: {codebase_path}")

    # Initialize handlers
    handlers = ToolHandlers(codebase_path)

    # Create MCP server
    server = Server("kelpie-mcp")

    @server.list_tools()
    async def list_tools() -> list[Tool]:
        """List available tools."""
        return [
            Tool(
                name=tool["name"],
                description=tool["description"],
                inputSchema=tool["inputSchema"],
            )
            for tool in ALL_TOOLS
        ]

    @server.call_tool()
    async def call_tool(name: str, arguments: dict[str, Any]) -> list[TextContent]:
        """Handle tool invocation."""
        logger.info(f"Tool called: {name}")

        try:
            result = await handlers.handle_tool(name, arguments or {})
            return [TextContent(type="text", text=json.dumps(result, indent=2, default=str))]
        except Exception as e:
            logger.error(f"Tool error: {e}", exc_info=True)
            return [TextContent(type="text", text=json.dumps({"error": str(e)}))]

    return server, handlers


async def main():
    """Main entry point for MCP server."""
    logger.info("Starting Kelpie MCP Server...")

    server, handlers = create_server()

    logger.info(f"Registered {len(ALL_TOOLS)} tools")

    async with stdio_server() as (read_stream, write_stream):
        logger.info("Server running on stdio")
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options(),
        )


def cli_main():
    """CLI entry point."""
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logger.info("Server stopped by user")
    except Exception as e:
        logger.error(f"Server error: {e}", exc_info=True)
        sys.exit(1)


if __name__ == "__main__":
    cli_main()
