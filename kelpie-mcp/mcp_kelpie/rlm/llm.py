"""
Sub-LLM integration for RLM environment.

VDE insight: Sub-models can analyze loaded variables server-side without
using the primary model's context window. This enables cost-efficient
analysis using Haiku for routine queries while preserving Opus for reasoning.

Tools exposed:
- repl_sub_llm: Have sub-model analyze a variable IN THE SERVER
"""

import os
from typing import Any

from anthropic import Anthropic

from .types import LoadedVariable, SubLLMResult

# TigerStyle: Explicit constants
DEFAULT_MAX_TOKENS = 4096
MAX_CONTEXT_CHARS = 100_000  # ~25K tokens for context


def _get_model_from_env() -> str:
    """Get the model from KELPIE_SUB_LLM_MODEL environment variable.

    Raises:
        ValueError: If KELPIE_SUB_LLM_MODEL is not set
    """
    model = os.environ.get("KELPIE_SUB_LLM_MODEL")
    if not model:
        raise ValueError("KELPIE_SUB_LLM_MODEL environment variable must be set in .mcp.json")
    return model


class SubLLM:
    """Sub-LLM caller for analyzing loaded variables.

    VDE insight: The sub-model analyzes variables server-side, not in the
    primary model's context. This enables:
    1. Cost efficiency (Haiku vs Opus)
    2. Parallel analysis of multiple partitions
    3. Specialized prompts for different analysis tasks

    Model must be configured via KELPIE_SUB_LLM_MODEL environment variable in .mcp.json.
    """

    def __init__(self, api_key: str | None = None, default_model: str | None = None):
        """Initialize SubLLM client.

        Args:
            api_key: Anthropic API key (defaults to ANTHROPIC_API_KEY env var)
            default_model: Model override (defaults to KELPIE_SUB_LLM_MODEL env var, lazily loaded)
        """
        self._api_key_override = api_key
        self._model_override = default_model
        self._client: Anthropic | None = None

    @property
    def api_key(self) -> str | None:
        """Get API key (lazily from env if not provided)."""
        return self._api_key_override or os.environ.get("ANTHROPIC_API_KEY")

    @property
    def default_model(self) -> str:
        """Get default model (lazily from env if not provided)."""
        return self._model_override or _get_model_from_env()

    def _get_client(self) -> Anthropic:
        """Get or create Anthropic client."""
        if self._client is None:
            if not self.api_key:
                raise ValueError("ANTHROPIC_API_KEY not set and no API key provided")
            self._client = Anthropic(api_key=self.api_key)
        return self._client

    async def analyze_variable(
        self,
        variable: LoadedVariable,
        query: str,
        model: str | None = None,
        max_tokens: int = DEFAULT_MAX_TOKENS,
    ) -> SubLLMResult:
        """Have a sub-LLM analyze a loaded variable.

        Args:
            variable: LoadedVariable to analyze
            query: Question to ask about the variable
            model: Model override (defaults to KELPIE_SUB_LLM_MODEL env var)
            max_tokens: Maximum response tokens

        Returns:
            SubLLMResult with response

        VDE pattern: The variable content becomes the context for the sub-LLM,
        which can be much larger than what fits in the primary model's context.
        """
        model = model or self.default_model

        # Build context from variable files
        context_parts = []
        total_chars = 0

        for path, content in variable.files.items():
            # Check if adding this file would exceed limit
            file_content = f"=== {path} ===\n{content}\n"
            if total_chars + len(file_content) > MAX_CONTEXT_CHARS:
                context_parts.append(f"\n[Truncated: {len(variable.files) - len(context_parts)} more files]")
                break
            context_parts.append(file_content)
            total_chars += len(file_content)

        context = "\n".join(context_parts)

        # Build prompt
        system_prompt = f"""You are analyzing code files loaded into a variable named '{variable.name}'.
The variable contains {variable.file_count} files ({variable.total_bytes / 1024:.1f}KB) matching pattern: {variable.glob_pattern}

Your task is to answer questions about this code. Be specific and reference file names and line numbers when relevant.
Keep your response focused and concise."""

        user_message = f"""Here are the files:

{context}

Question: {query}"""

        try:
            client = self._get_client()

            # Synchronous call (wrapped in async for consistency)
            response = client.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system_prompt,
                messages=[{"role": "user", "content": user_message}],
            )

            # Extract response text
            response_text = ""
            for block in response.content:
                if hasattr(block, "text"):
                    response_text += block.text

            return SubLLMResult(
                success=True,
                response=response_text,
                model=model,
                input_tokens=response.usage.input_tokens,
                output_tokens=response.usage.output_tokens,
            )

        except Exception as e:
            return SubLLMResult(
                success=False,
                error=f"Sub-LLM call failed: {type(e).__name__}: {e}",
                model=model,
            )

    async def analyze_content(
        self,
        content: str,
        query: str,
        context_name: str = "content",
        model: str | None = None,
        max_tokens: int = DEFAULT_MAX_TOKENS,
    ) -> SubLLMResult:
        """Have a sub-LLM analyze arbitrary content.

        Args:
            content: Content to analyze
            query: Question to ask
            context_name: Name for the content (for prompt)
            model: Model override (defaults to KELPIE_SUB_LLM_MODEL env var)
            max_tokens: Maximum response tokens

        Returns:
            SubLLMResult with response
        """
        model = model or self.default_model

        # Truncate if needed
        if len(content) > MAX_CONTEXT_CHARS:
            content = content[:MAX_CONTEXT_CHARS] + f"\n\n[Truncated: {len(content) - MAX_CONTEXT_CHARS} more chars]"

        system_prompt = f"""You are analyzing {context_name}. Be specific and concise in your response."""

        user_message = f"""Here is the content:

{content}

Question: {query}"""

        try:
            client = self._get_client()

            response = client.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system_prompt,
                messages=[{"role": "user", "content": user_message}],
            )

            response_text = ""
            for block in response.content:
                if hasattr(block, "text"):
                    response_text += block.text

            return SubLLMResult(
                success=True,
                response=response_text,
                model=model,
                input_tokens=response.usage.input_tokens,
                output_tokens=response.usage.output_tokens,
            )

        except Exception as e:
            return SubLLMResult(
                success=False,
                error=f"Sub-LLM call failed: {type(e).__name__}: {e}",
                model=model,
            )

    def analyze_variable_sync(
        self,
        variable: LoadedVariable,
        query: str,
        model: str | None = None,
        max_tokens: int = DEFAULT_MAX_TOKENS,
    ) -> SubLLMResult:
        """Synchronous version of analyze_variable.

        For use in contexts where async is not available.
        """
        import asyncio

        return asyncio.get_event_loop().run_until_complete(
            self.analyze_variable(variable, query, model, max_tokens)
        )

    def analyze_content_sync(
        self,
        content: str,
        query: str,
        context_name: str = "content",
        model: str | None = None,
        max_tokens: int = DEFAULT_MAX_TOKENS,
    ) -> SubLLMResult:
        """Truly synchronous analyze_content for use inside REPL execution.

        This is the TRUE RLM pattern: sub_llm() callable from within repl_exec code.
        No asyncio involvement - calls the Anthropic SDK directly (which is sync).

        This enables symbolic recursion - LLM calls embedded in code logic:
            for path, content in files.items():
                analysis = sub_llm(content, "What does this do?")
        """
        model = model or self.default_model

        # Truncate if needed
        if len(content) > MAX_CONTEXT_CHARS:
            content = content[:MAX_CONTEXT_CHARS] + f"\n\n[Truncated: {len(content) - MAX_CONTEXT_CHARS} more chars]"

        system_prompt = f"""You are analyzing {context_name}. Be specific and concise in your response."""

        user_message = f"""Here is the content:

{content}

Question: {query}"""

        try:
            client = self._get_client()

            # Direct sync call - no asyncio needed
            response = client.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system_prompt,
                messages=[{"role": "user", "content": user_message}],
            )

            response_text = ""
            for block in response.content:
                if hasattr(block, "text"):
                    response_text += block.text

            return SubLLMResult(
                success=True,
                response=response_text,
                model=model,
                input_tokens=response.usage.input_tokens,
                output_tokens=response.usage.output_tokens,
            )

        except Exception as e:
            return SubLLMResult(
                success=False,
                error=f"Sub-LLM call failed: {type(e).__name__}: {e}",
                model=model,
            )
