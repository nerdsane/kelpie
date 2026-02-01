//! Memory Editing Tools for Letta Compatibility
//!
//! TigerStyle: Thin wrappers around AppState memory operations.
//!
//! Implements the 9 Letta memory tools:
//! - core_memory_append: Append to a core memory block
//! - core_memory_replace: Replace content in a core memory block
//! - archival_memory_insert: Insert into archival memory
//! - archival_memory_search: Search archival memory
//! - conversation_search: Search conversation history
//!
//! BUG-002 FIX: All memory tools now use ContextAwareToolHandler to get the
//! agent_id from ToolExecutionContext instead of requiring the LLM to provide it.
//! The LLM only knows its name (e.g., "Tama"), not its UUID, so passing agent_id
//! as an input parameter caused "agent not found" errors.

use crate::state::AppState;
use crate::tools::{
    ContextAwareToolHandler, ToolExecutionContext, ToolExecutionResult, UnifiedToolRegistry,
};
use kelpie_core::io::{TimeProvider, WallClockTime};
use serde_json::{json, Value};
use std::sync::Arc;

/// Helper to compute elapsed time since start_ms using WallClockTime.
#[inline]
fn elapsed_ms(start_ms: u64) -> u64 {
    WallClockTime::new().monotonic_ms().saturating_sub(start_ms)
}

/// Extract agent_id from context, falling back to input for backwards compatibility
fn get_agent_id(context: &ToolExecutionContext, input: &Value) -> Result<String, String> {
    // Prefer context.agent_id (the correct UUID)
    if let Some(agent_id) = &context.agent_id {
        return Ok(agent_id.clone());
    }

    // Fall back to input parameter (for backwards compatibility)
    input
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Error: agent_id not available in context or input".to_string())
}

/// Register all memory tools with the unified registry
pub async fn register_memory_tools<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // core_memory_append
    register_core_memory_append(registry, state.clone()).await;

    // core_memory_replace
    register_core_memory_replace(registry, state.clone()).await;

    // archival_memory_insert
    register_archival_memory_insert(registry, state.clone()).await;

    // archival_memory_search
    register_archival_memory_search(registry, state.clone()).await;

    // conversation_search
    register_conversation_search(registry, state.clone()).await;

    // conversation_search_date
    register_conversation_search_date(registry, state).await;

    tracing::info!(
        "Registered memory tools: core_memory_append, core_memory_replace, archival_memory_insert, archival_memory_search, conversation_search, conversation_search_date"
    );
}

async fn register_core_memory_append<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let label = match input.get("label").and_then(|v| v.as_str()) {
                    Some(l) => l.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'label'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let content = match input.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'content'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                // BUG-001 FIX: Use atomic append_or_create to eliminate TOCTOU race
                // Phase 6.11: Use async version that works with actor system
                match state
                    .append_or_create_block_by_label_async(&agent_id, &label, &content)
                    .await
                {
                    Ok(_) => ToolExecutionResult::success(
                        format!("Successfully updated memory block '{}'", label),
                        elapsed_ms(start_ms),
                    ),
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "core_memory_append",
            "Append content to a core memory block. The block will be created if it doesn't exist. Core memory blocks are always visible in the LLM context window. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Block label (e.g., 'persona', 'human', 'facts', 'goals', 'scratch')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to append to the block"
                    }
                },
                "required": ["label", "content"]
            }),
            handler,
        )
        .await;
}

async fn register_core_memory_replace<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    // Single source of truth: AgentService required (no HashMap fallback)
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                // Require AgentService (no fallback)
                let service = match state.agent_service() {
                    Some(s) => s,
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: AgentService not configured",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let label = match input.get("label").and_then(|v| v.as_str()) {
                    Some(l) => l.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'label'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let old_content = match input.get("old_content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'old_content'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let new_content = input
                    .get("new_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                match service
                    .core_memory_replace(&agent_id, &label, &old_content, &new_content)
                    .await
                {
                    Ok(()) => ToolExecutionResult::success(
                        format!("Successfully replaced content in memory block '{}'", label),
                        elapsed_ms(start_ms),
                    ),
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "core_memory_replace",
            "Replace content in a core memory block. The old content must exist in the block. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Block label"
                    },
                    "old_content": {
                        "type": "string",
                        "description": "Content to find and replace"
                    },
                    "new_content": {
                        "type": "string",
                        "description": "Replacement content (can be empty to delete)"
                    }
                },
                "required": ["label", "old_content", "new_content"]
            }),
            handler,
        )
        .await;
}

async fn register_archival_memory_insert<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    // Single source of truth: AgentService required (no HashMap fallback)
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                // Require AgentService (no fallback)
                let service = match state.agent_service() {
                    Some(s) => s,
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: AgentService not configured",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let content = match input.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'content'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                match service.archival_insert(&agent_id, &content, None).await {
                    Ok(entry_id) => ToolExecutionResult::success(
                        format!(
                            "Successfully inserted into archival memory. Entry ID: {}",
                            entry_id
                        ),
                        elapsed_ms(start_ms),
                    ),
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "archival_memory_insert",
            "Insert content into archival memory with embedding for semantic search. Use this for long-term knowledge that doesn't need to be in the main context window. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Content to store in archival memory"
                    }
                },
                "required": ["content"]
            }),
            handler,
        )
        .await;
}

async fn register_archival_memory_search<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    // Single source of truth: AgentService required (no HashMap fallback)
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                // Require AgentService (no fallback)
                let service = match state.agent_service() {
                    Some(s) => s,
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: AgentService not configured",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let query = match input.get("query").and_then(|v| v.as_str()) {
                    Some(q) => q.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'query'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let page = input.get("page").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let page_size = 10;

                // Helper function to format results
                fn format_results(
                    entries: Vec<crate::models::ArchivalEntry>,
                    page: usize,
                    elapsed: u64,
                ) -> ToolExecutionResult {
                    if entries.is_empty() {
                        ToolExecutionResult::success("No results found", elapsed)
                    } else {
                        let results: Vec<String> = entries
                            .iter()
                            .map(|e| format!("[{}] {}", e.id, e.content))
                            .collect();
                        ToolExecutionResult::success(
                            format!(
                                "Found {} results (page {}):\n{}",
                                results.len(),
                                page,
                                results.join("\n---\n")
                            ),
                            elapsed,
                        )
                    }
                }

                let total_needed = (page + 1) * page_size;
                match service
                    .archival_search(&agent_id, &query, total_needed)
                    .await
                {
                    Ok(entries) => {
                        let offset = page * page_size;
                        let page_entries: Vec<_> =
                            entries.into_iter().skip(offset).take(page_size).collect();
                        format_results(page_entries, page, elapsed_ms(start_ms))
                    }
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "archival_memory_search",
            "Search archival memory using semantic search. Returns paginated results. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "page": {
                        "type": "integer",
                        "description": "Page number (0-indexed)",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
            handler,
        )
        .await;
}

async fn register_conversation_search<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    // Single source of truth: AgentService required (no HashMap fallback)
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                // Require AgentService (no fallback)
                let service = match state.agent_service() {
                    Some(s) => s,
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: AgentService not configured",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let query = match input.get("query").and_then(|v| v.as_str()) {
                    Some(q) => q.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'query'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let page = input.get("page").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let page_size = 10;

                // Helper function to format results
                fn format_message_results(
                    messages: Vec<crate::models::Message>,
                    page: usize,
                    elapsed: u64,
                ) -> ToolExecutionResult {
                    if messages.is_empty() {
                        ToolExecutionResult::success("No matching conversations found", elapsed)
                    } else {
                        let results: Vec<String> = messages
                            .iter()
                            .map(|m| format!("[{:?}]: {}", m.role, m.content))
                            .collect();
                        ToolExecutionResult::success(
                            format!(
                                "Found {} results (page {}):\n{}",
                                results.len(),
                                page,
                                results.join("\n---\n")
                            ),
                            elapsed,
                        )
                    }
                }

                let total_needed = (page + 1) * page_size;
                match service
                    .conversation_search(&agent_id, &query, total_needed)
                    .await
                {
                    Ok(messages) => {
                        let offset = page * page_size;
                        let page_messages: Vec<_> =
                            messages.into_iter().skip(offset).take(page_size).collect();
                        format_message_results(page_messages, page, elapsed_ms(start_ms))
                    }
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "conversation_search",
            "Search past conversation messages. Returns paginated results matching the query. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "page": {
                        "type": "integer",
                        "description": "Page number (0-indexed)",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
            handler,
        )
        .await;
}

async fn register_conversation_search_date<R: kelpie_core::Runtime + 'static>(
    registry: &UnifiedToolRegistry,
    state: AppState<R>,
) {
    // BUG-002 FIX: Use ContextAwareToolHandler to get agent_id from context
    // Single source of truth: AgentService required (no HashMap fallback)
    let handler: ContextAwareToolHandler =
        Arc::new(move |input: &Value, context: &ToolExecutionContext| {
            let state = state.clone();
            let input = input.clone();
            let context = context.clone();
            Box::pin(async move {
                let start_ms = WallClockTime::new().monotonic_ms();

                // Require AgentService (no fallback)
                let service = match state.agent_service() {
                    Some(s) => s,
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: AgentService not configured",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                let agent_id = match get_agent_id(&context, &input) {
                    Ok(id) => id,
                    Err(e) => return ToolExecutionResult::failure(e, elapsed_ms(start_ms)),
                };

                let query = match input.get("query").and_then(|v| v.as_str()) {
                    Some(q) => q.to_string(),
                    None => {
                        return ToolExecutionResult::failure(
                            "Error: missing required parameter 'query'",
                            elapsed_ms(start_ms),
                        )
                    }
                };

                // Parse start_date (optional)
                let start_date = match input.get("start_date") {
                    Some(val) => match parse_date_param(val) {
                        Ok(dt) => Some(dt),
                        Err(e) => {
                            return ToolExecutionResult::failure(
                                format!("Error parsing start_date: {}", e),
                                elapsed_ms(start_ms),
                            )
                        }
                    },
                    None => None,
                };

                // Parse end_date (optional)
                let end_date = match input.get("end_date") {
                    Some(val) => match parse_date_param(val) {
                        Ok(dt) => Some(dt),
                        Err(e) => {
                            return ToolExecutionResult::failure(
                                format!("Error parsing end_date: {}", e),
                                elapsed_ms(start_ms),
                            )
                        }
                    },
                    None => None,
                };

                // Validate date range
                if let (Some(start), Some(end)) = (start_date, end_date) {
                    if start > end {
                        return ToolExecutionResult::failure(
                            "Error: start_date must be before end_date",
                            elapsed_ms(start_ms),
                        );
                    }
                }

                let page = input.get("page").and_then(|v| v.as_i64()).unwrap_or(0) as usize;
                let page_size = 10;

                // Helper function to format results with dates
                fn format_date_results(
                    messages: Vec<crate::models::Message>,
                    page: usize,
                    elapsed: u64,
                ) -> ToolExecutionResult {
                    if messages.is_empty() {
                        ToolExecutionResult::success(
                            "No matching conversations found in date range",
                            elapsed,
                        )
                    } else {
                        let results: Vec<String> = messages
                            .iter()
                            .map(|m| {
                                format!(
                                    "[{:?}] [{}]: {}",
                                    m.role,
                                    m.created_at.to_rfc3339(),
                                    m.content
                                )
                            })
                            .collect();
                        ToolExecutionResult::success(
                            format!(
                                "Found {} results (page {}):\n{}",
                                results.len(),
                                page,
                                results.join("\n---\n")
                            ),
                            elapsed,
                        )
                    }
                }

                let total_needed = (page + 1) * page_size;
                // Convert dates to RFC 3339 strings for service
                let start_str = start_date.map(|d| d.to_rfc3339());
                let end_str = end_date.map(|d| d.to_rfc3339());

                match service
                    .conversation_search_date(
                        &agent_id,
                        &query,
                        start_str.as_deref(),
                        end_str.as_deref(),
                        total_needed,
                    )
                    .await
                {
                    Ok(messages) => {
                        let offset = page * page_size;
                        let page_messages: Vec<_> =
                            messages.into_iter().skip(offset).take(page_size).collect();
                        format_date_results(page_messages, page, elapsed_ms(start_ms))
                    }
                    Err(e) => {
                        ToolExecutionResult::failure(format!("Error: {}", e), elapsed_ms(start_ms))
                    }
                }
            })
        });

    registry
        .register_context_aware_builtin(
            "conversation_search_date",
            "Search past conversation messages with date filtering. Returns paginated results matching the query within the specified date range. Supports ISO 8601, RFC 3339, and Unix timestamps. The agent_id is automatically provided from context.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "start_date": {
                        "type": ["string", "integer"],
                        "description": "Start date (ISO 8601/RFC 3339 string or Unix timestamp). Messages on or after this date will be included."
                    },
                    "end_date": {
                        "type": ["string", "integer"],
                        "description": "End date (ISO 8601/RFC 3339 string or Unix timestamp). Messages on or before this date will be included."
                    },
                    "page": {
                        "type": "integer",
                        "description": "Page number (0-indexed)",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
            handler,
        )
        .await;
}

/// Parse date parameter from JSON value
///
/// Supports:
/// - ISO 8601: "2024-01-15T10:00:00Z"
/// - RFC 3339: "2024-01-15T10:00:00+00:00"
/// - Unix timestamp: 1705315200 (seconds since epoch)
fn parse_date_param(val: &Value) -> Result<chrono::DateTime<chrono::Utc>, String> {
    use chrono::{DateTime, TimeZone, Utc};

    match val {
        // String: try ISO 8601 / RFC 3339
        Value::String(s) => {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    // Try ISO 8601 without timezone (assume UTC)
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                        .map(|ndt| Utc.from_utc_datetime(&ndt))
                })
                .or_else(|_| {
                    // Try date only (assume start of day UTC)
                    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map(|nd| {
                        Utc.from_utc_datetime(
                            &nd.and_hms_opt(0, 0, 0)
                                .expect("00:00:00 is always a valid time"),
                        )
                    })
                })
                .map_err(|e| {
                    format!(
                        "Invalid date format '{}'. Expected ISO 8601/RFC 3339 or Unix timestamp. Error: {}",
                        s, e
                    )
                })
        }
        // Number: treat as Unix timestamp (seconds)
        Value::Number(n) => {
            if let Some(ts) = n.as_i64() {
                Utc.timestamp_opt(ts, 0)
                    .single()
                    .ok_or_else(|| format!("Invalid Unix timestamp: {} (out of range)", ts))
            } else {
                Err(format!("Invalid timestamp: {}", n))
            }
        }
        _ => Err(format!(
            "Invalid date type: expected string or number, got {:?}",
            val
        )),
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::actor::{AgentActor, AgentActorState, LlmClient, LlmMessage, LlmResponse};
    use crate::models::{AgentType, CreateAgentRequest, CreateBlockRequest};
    use crate::service::AgentService;
    use crate::tools::ToolExecutionContext;
    use async_trait::async_trait;
    use kelpie_core::Runtime;
    use kelpie_dst::{DeterministicRng, FaultInjector, SimStorage};
    use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig};
    use std::sync::Arc;

    /// Mock LLM client for testing that returns simple responses
    struct MockLlmClient;

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn complete_with_tools(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Vec<crate::llm::ToolDefinition>,
        ) -> kelpie_core::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Test response".to_string(),
                tool_calls: vec![],
                prompt_tokens: 0,
                completion_tokens: 0,
                stop_reason: "end_turn".to_string(),
            })
        }

        async fn continue_with_tool_result(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Vec<crate::llm::ToolDefinition>,
            _assistant_blocks: Vec<crate::llm::ContentBlock>,
            _tool_results: Vec<(String, String)>,
        ) -> kelpie_core::Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Test response".to_string(),
                tool_calls: vec![],
                prompt_tokens: 0,
                completion_tokens: 0,
                stop_reason: "end_turn".to_string(),
            })
        }
    }

    /// Create a test AppState with AgentService (single source of truth)
    async fn create_test_state_with_service() -> AppState<kelpie_core::TokioRuntime> {
        let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient);
        let actor = AgentActor::new(llm, Arc::new(UnifiedToolRegistry::new()));
        let factory = Arc::new(CloneFactory::new(actor));

        let rng = DeterministicRng::new(42);
        let faults = Arc::new(FaultInjector::new(rng.fork()));
        let storage = SimStorage::new(rng.fork(), faults);
        let kv = Arc::new(storage);

        let runtime = kelpie_core::TokioRuntime;

        let mut dispatcher = Dispatcher::<AgentActor, AgentActorState, _>::new(
            factory,
            kv,
            DispatcherConfig::default(),
            runtime.clone(),
        );
        let handle = dispatcher.handle();

        drop(runtime.spawn(async move {
            dispatcher.run().await;
        }));

        let service = AgentService::new(handle.clone());
        AppState::with_agent_service(runtime, service, handle)
    }

    fn create_test_agent_request(name: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            name: name.to_string(),
            agent_type: AgentType::default(),
            model: None,
            embedding: None,
            system: None,
            description: None,
            memory_blocks: vec![CreateBlockRequest {
                label: "persona".to_string(),
                value: "I am a test agent".to_string(),
                description: None,
                limit: None,
            }],
            block_ids: vec![],
            tool_ids: vec![],
            tags: vec![],
            metadata: json!({}),
            project_id: None,
            user_id: None,
            org_id: None,
        }
    }

    #[tokio::test]
    async fn test_memory_tools_registration() {
        let state = AppState::new(kelpie_core::current_runtime());
        let registry = state.tool_registry();

        register_memory_tools(registry, state.clone()).await;

        // Verify tools are registered
        assert!(registry.has_tool("core_memory_append").await);
        assert!(registry.has_tool("core_memory_replace").await);
        assert!(registry.has_tool("archival_memory_insert").await);
        assert!(registry.has_tool("archival_memory_search").await);
        assert!(registry.has_tool("conversation_search").await);
    }

    #[tokio::test]
    async fn test_core_memory_append_integration() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Execute append - note: no agent_id in input since it comes from context
        let result = registry
            .execute_with_context(
                "core_memory_append",
                &json!({
                    "label": "facts",
                    "content": "User likes pizza"
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Append failed: {}", result.output);
        assert!(result.output.contains("Successfully"));

        // Verify block was created via AgentService
        let service = state.agent_service().unwrap();
        let block = service
            .get_block_by_label(&agent_id, "facts")
            .await
            .unwrap();
        assert!(block.is_some());
        assert!(block.unwrap().value.contains("pizza"));
    }

    #[tokio::test]
    async fn test_core_memory_replace_integration() {
        let state = create_test_state_with_service().await;

        // Create agent with existing block via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Execute replace on existing persona block
        let result = registry
            .execute_with_context(
                "core_memory_replace",
                &json!({
                    "label": "persona",
                    "old_content": "test agent",
                    "new_content": "helpful assistant"
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Replace failed: {}", result.output);

        // Verify replacement via AgentService
        let service = state.agent_service().unwrap();
        let block = service
            .get_block_by_label(&agent_id, "persona")
            .await
            .unwrap();
        assert!(block.is_some(), "Block should exist");
        assert!(block.as_ref().unwrap().value.contains("helpful assistant"));
        assert!(!block.as_ref().unwrap().value.contains("test agent"));
    }

    #[tokio::test]
    async fn test_archival_memory_integration() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Insert into archival
        let result = registry
            .execute_with_context(
                "archival_memory_insert",
                &json!({
                    "content": "User's favorite color is blue"
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Insert failed: {}", result.output);
        assert!(result.output.contains("Entry ID"));

        // Search archival
        let result = registry
            .execute_with_context(
                "archival_memory_search",
                &json!({
                    "query": "blue"
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Search failed: {}", result.output);
        assert!(result.output.contains("blue") || result.output.contains("Found"));
    }

    #[test]
    fn test_parse_date_iso8601() {
        // RFC 3339 with timezone
        let val = json!("2024-01-15T10:00:00Z");
        let dt = parse_date_param(&val).unwrap();
        assert_eq!(dt.timestamp(), 1705312800);

        // RFC 3339 with offset
        let val = json!("2024-01-15T10:00:00+00:00");
        let dt = parse_date_param(&val).unwrap();
        assert_eq!(dt.timestamp(), 1705312800);
    }

    #[test]
    fn test_parse_date_unix_timestamp() {
        use chrono::Datelike;
        // Unix timestamp (seconds)
        let val = json!(1705315200);
        let dt = parse_date_param(&val).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn test_parse_date_date_only() {
        use chrono::{Datelike, Timelike};
        // Date only (assumes 00:00:00 UTC)
        let val = json!("2024-01-15");
        let dt = parse_date_param(&val).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 0);
        assert_eq!(dt.minute(), 0);
    }

    #[test]
    fn test_parse_date_invalid() {
        // Invalid format
        let val = json!("not-a-date");
        assert!(parse_date_param(&val).is_err());

        // Invalid type
        let val = json!(true);
        assert!(parse_date_param(&val).is_err());

        // Invalid timestamp (too large)
        let val = json!(99999999999999i64);
        assert!(parse_date_param(&val).is_err());
    }

    #[tokio::test]
    async fn test_conversation_search_date() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Add test message (via send_message endpoint simulation)
        // Note: In real usage, messages are added through handle_message
        // For testing, we'll verify the tool executes without error

        // Search with valid date range
        let result = registry
            .execute_with_context(
                "conversation_search_date",
                &json!({
                    "query": "test",
                    "start_date": "2024-01-01T00:00:00Z",
                    "end_date": "2024-12-31T23:59:59Z"
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Search failed: {}", result.output);
        // Since agent has no messages yet, expect "No matching conversations"
        assert!(result.output.contains("No matching conversations"));
    }

    #[tokio::test]
    async fn test_conversation_search_date_unix_timestamp() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Search with Unix timestamps
        let result = registry
            .execute_with_context(
                "conversation_search_date",
                &json!({
                    "query": "test",
                    "start_date": 1704067200, // 2024-01-01
                    "end_date": 1735689599   // 2024-12-31
                }),
                Some(&context),
            )
            .await;

        assert!(result.success, "Search failed: {}", result.output);
    }

    #[tokio::test]
    async fn test_conversation_search_date_invalid_range() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Search with invalid range (start > end)
        let result = registry
            .execute_with_context(
                "conversation_search_date",
                &json!({
                    "query": "test",
                    "start_date": "2024-12-31T00:00:00Z",
                    "end_date": "2024-01-01T00:00:00Z"
                }),
                Some(&context),
            )
            .await;

        // Should fail with error message
        assert!(result
            .output
            .contains("Error: start_date must be before end_date"));
    }

    #[tokio::test]
    async fn test_conversation_search_date_invalid_format() {
        let state = create_test_state_with_service().await;

        // Create agent via AgentService
        let request = create_test_agent_request("test-agent");
        let agent = state.create_agent_async(request).await.unwrap();
        let agent_id = agent.id.clone();

        // Register memory tools
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Create context with agent_id (BUG-002 FIX: tools now get agent_id from context)
        let context = ToolExecutionContext {
            agent_id: Some(agent_id.clone()),
            ..Default::default()
        };

        // Search with invalid date format
        let result = registry
            .execute_with_context(
                "conversation_search_date",
                &json!({
                    "query": "test",
                    "start_date": "not-a-date"
                }),
                Some(&context),
            )
            .await;

        // Should fail with error message
        assert!(result.output.contains("Error parsing start_date"));
    }

    #[tokio::test]
    async fn test_conversation_search_date_missing_params() {
        let state = create_test_state_with_service().await;
        let registry = state.tool_registry();
        register_memory_tools(registry, state.clone()).await;

        // Missing agent_id in context (and no fallback in input)
        let context_without_agent = ToolExecutionContext::default();
        let result = registry
            .execute_with_context(
                "conversation_search_date",
                &json!({
                    "query": "test"
                }),
                Some(&context_without_agent),
            )
            .await;

        assert!(result.output.contains("agent_id not available"));

        // Create context with agent_id for next test
        let context = ToolExecutionContext {
            agent_id: Some("test-id".to_string()),
            ..Default::default()
        };

        // Missing query
        let result = registry
            .execute_with_context("conversation_search_date", &json!({}), Some(&context))
            .await;

        assert!(result
            .output
            .contains("Error: missing required parameter 'query'"));
    }
}
