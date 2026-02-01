//! Service layer for agent operations
//!
//! TigerStyle: Service wraps dispatcher, provides clean API, handles errors.

mod teleport_service;

pub use teleport_service::{
    TeleportInRequest, TeleportInResponse, TeleportOutRequest, TeleportOutResponse,
    TeleportPackageInfo, TeleportService,
};

use crate::actor::{
    AgentContinuation, ArchivalDeleteRequest, ArchivalInsertRequest, ArchivalInsertResponse,
    ArchivalSearchRequest, ArchivalSearchResponse, ContinueWithToolResultsRequest,
    ConversationSearchDateRequest, ConversationSearchRequest, ConversationSearchResponse,
    CoreMemoryReplaceRequest, GetBlockRequest, GetBlockResponse, HandleMessageFullRequest,
    HandleMessageFullResponse, HandleMessageResult, ListMessagesRequest, ListMessagesResponse,
    PendingToolCall, StreamChunk, ToolResult,
};
use crate::models::{
    AgentState, ArchivalEntry, Block, CreateAgentRequest, Message, StreamEvent, UpdateAgentRequest,
};
use crate::security::audit::SharedAuditLog;
use crate::tools::{ToolExecutionContext, UnifiedToolRegistry};
use bytes::Bytes;
use futures::stream::Stream;
use kelpie_core::actor::ActorId;
use kelpie_core::{Error, Result};
use kelpie_runtime::DispatcherHandle;
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

/// AgentService - service layer for agent operations
///
/// Wraps DispatcherHandle and provides high-level agent operations.
/// Handles serialization/deserialization and error mapping.
///
/// TigerStyle: Clean abstraction, explicit error handling, testable.
#[derive(Clone)]
pub struct AgentService<R: kelpie_core::Runtime> {
    /// Dispatcher handle for actor invocations
    dispatcher: DispatcherHandle<R>,
    /// Tool registry for executing tools outside actor context (continuation-based execution)
    tool_registry: Option<Arc<UnifiedToolRegistry>>,
    /// Audit log for recording tool executions
    audit_log: Option<SharedAuditLog>,
}

impl<R: kelpie_core::Runtime> AgentService<R> {
    /// Create a new AgentService
    pub fn new(dispatcher: DispatcherHandle<R>) -> Self {
        Self {
            dispatcher,
            tool_registry: None,
            audit_log: None,
        }
    }

    /// Create a new AgentService with tool registry for continuation-based execution
    ///
    /// The tool registry is used to execute tools outside actor invocations,
    /// which is required for the continuation-based architecture that avoids
    /// reentrant deadlock.
    pub fn with_tool_registry(
        dispatcher: DispatcherHandle<R>,
        tool_registry: Arc<UnifiedToolRegistry>,
    ) -> Self {
        Self {
            dispatcher,
            tool_registry: Some(tool_registry),
            audit_log: None,
        }
    }

    /// Create a new AgentService with tool registry and audit log
    pub fn with_tool_registry_and_audit(
        dispatcher: DispatcherHandle<R>,
        tool_registry: Arc<UnifiedToolRegistry>,
        audit_log: SharedAuditLog,
    ) -> Self {
        Self {
            dispatcher,
            tool_registry: Some(tool_registry),
            audit_log: Some(audit_log),
        }
    }

    /// Create a new agent
    ///
    /// # Arguments
    /// * `request` - Agent creation request
    ///
    /// # Returns
    /// Created agent state with ID
    ///
    /// # Errors
    /// Returns error if actor creation fails
    pub async fn create_agent(&self, request: CreateAgentRequest) -> Result<AgentState> {
        // Generate actor ID from agent name
        let actor_id = ActorId::new("agents", uuid::Uuid::new_v4().to_string())?;

        // Serialize request
        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize CreateAgentRequest: {}", e),
        })?;

        // Invoke create operation - returns AgentState directly (BUG-001 fix)
        let response = self
            .dispatcher
            .invoke(actor_id.clone(), "create".to_string(), Bytes::from(payload))
            .await?;

        // Deserialize and return the created agent state
        serde_json::from_slice(&response).map_err(|e| Error::Internal {
            message: format!(
                "Failed to deserialize AgentState from create response: {}",
                e
            ),
        })
    }

    /// Send message to agent
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `message` - Message as JSON value
    ///
    /// # Returns
    /// Response as JSON value
    pub async fn send_message(&self, agent_id: &str, message: Value) -> Result<Value> {
        // Extract content from message (Phase 6.8: support multiple formats)
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Internal {
                message: "Message must have 'content' field".to_string(),
            })?;

        // Use send_message_full which handles continuation-based tool execution
        let response = self
            .send_message_full(agent_id, content.to_string())
            .await?;

        // Convert typed response to JSON
        serde_json::to_value(&response).map_err(|e| Error::Internal {
            message: format!("Failed to serialize message response: {}", e),
        })
    }

    /// Send message with full agent loop (Phase 6.9)
    ///
    /// Typed API for agent message handling. Returns structured response
    /// with full conversation history and usage stats.
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `content` - Message content from user
    ///
    /// # Returns
    /// HandleMessageFullResponse with messages and usage stats
    ///
    /// # Errors
    /// - Invalid agent_id
    /// - Agent not found/created
    /// - Actor invocation failure
    /// - Serialization/deserialization error
    ///
    /// # TigerStyle
    /// - Explicit typed API (not JSON Value)
    /// - Clear error messages
    /// - No unwrap()
    ///
    /// # Continuation-Based Architecture
    /// This method implements the continuation-based tool execution pattern:
    /// 1. Call handle_message_full on actor (returns HandleMessageResult)
    /// 2. If NeedTools: execute tools OUTSIDE actor, then call continue_with_tool_results
    /// 3. Loop until Done
    ///
    /// This avoids reentrant deadlock where tools calling dispatcher.invoke() would
    /// wait on an actor that's blocked waiting for those tools to complete.
    pub async fn send_message_full(
        &self,
        agent_id: &str,
        content: String,
    ) -> Result<HandleMessageFullResponse> {
        // TigerStyle: Validate preconditions
        assert!(!agent_id.is_empty(), "agent_id must not be empty");
        assert!(!content.is_empty(), "content must not be empty");

        let actor_id = ActorId::new("agents", agent_id)?;

        // Build typed request (no call context for top-level API calls)
        let request = HandleMessageFullRequest {
            content,
            call_context: None,
        };

        // Serialize request
        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize HandleMessageFullRequest: {}", e),
        })?;

        // Invoke handle_message_full operation
        let response_bytes = self
            .dispatcher
            .invoke(
                actor_id.clone(),
                "handle_message_full".to_string(),
                Bytes::from(payload),
            )
            .await?;

        // Deserialize result - now returns HandleMessageResult
        let mut result: HandleMessageResult =
            serde_json::from_slice(&response_bytes).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize HandleMessageResult: {}", e),
            })?;

        // Continuation loop: execute tools outside actor, then continue
        const MAX_CONTINUATION_LOOPS: u32 = 10;
        let mut loop_count = 0u32;

        loop {
            match result {
                HandleMessageResult::Done(response) => {
                    tracing::info!(
                        agent_id = %agent_id,
                        loop_count = loop_count,
                        "send_message_full completed successfully"
                    );
                    return Ok(response);
                }
                HandleMessageResult::NeedTools {
                    tool_calls,
                    continuation,
                } => {
                    loop_count += 1;
                    if loop_count > MAX_CONTINUATION_LOOPS {
                        return Err(Error::Internal {
                            message: format!(
                                "Max continuation loops ({}) exceeded",
                                MAX_CONTINUATION_LOOPS
                            ),
                        });
                    }

                    tracing::info!(
                        agent_id = %agent_id,
                        tool_count = tool_calls.len(),
                        loop_count = loop_count,
                        "Executing tools outside actor context"
                    );

                    // Execute tools outside actor context
                    let tool_results = self
                        .execute_tools_external(&tool_calls, agent_id, &continuation)
                        .await?;

                    // Build continuation request
                    let continue_request = ContinueWithToolResultsRequest {
                        tool_results,
                        continuation,
                    };

                    // Serialize and invoke
                    let continue_payload =
                        serde_json::to_vec(&continue_request).map_err(|e| Error::Internal {
                            message: format!(
                                "Failed to serialize ContinueWithToolResultsRequest: {}",
                                e
                            ),
                        })?;

                    let continue_response = self
                        .dispatcher
                        .invoke(
                            actor_id.clone(),
                            "continue_with_tool_results".to_string(),
                            Bytes::from(continue_payload),
                        )
                        .await?;

                    // Deserialize result
                    result = serde_json::from_slice(&continue_response).map_err(|e| {
                        Error::Internal {
                            message: format!("Failed to deserialize HandleMessageResult: {}", e),
                        }
                    })?;
                }
            }
        }
    }

    /// Execute tools outside actor context
    ///
    /// This is the key part of the continuation-based architecture - tools are executed
    /// here in the service layer, outside any actor invocation, so they can freely
    /// call the dispatcher without causing reentrant deadlock.
    async fn execute_tools_external(
        &self,
        tool_calls: &[PendingToolCall],
        agent_id: &str,
        continuation: &AgentContinuation,
    ) -> Result<Vec<ToolResult>> {
        let tool_registry = self.tool_registry.as_ref().ok_or_else(|| Error::Internal {
            message: "Tool registry not configured - cannot execute tools".to_string(),
        })?;

        let mut results = Vec::with_capacity(tool_calls.len());

        for tool_call in tool_calls {
            tracing::info!(
                agent_id = %agent_id,
                tool_name = %tool_call.name,
                tool_id = %tool_call.id,
                "Executing tool externally"
            );

            // Build tool execution context
            let (call_depth, mut call_chain) = match &continuation.call_context {
                Some(ctx_info) => (ctx_info.call_depth, ctx_info.call_chain.clone()),
                None => (0, vec![]),
            };

            if !call_chain.contains(&agent_id.to_string()) {
                call_chain.push(agent_id.to_string());
            }

            // NOTE: Dispatcher not passed to tools here. The call_agent tool for
            // agent-to-agent communication will need a different approach (possibly
            // having AgentService implement AgentDispatcher directly). For now, tools
            // that need to call other agents won't work from this path.
            // TODO: Issue #XX - Wire up dispatcher for call_agent tool
            let context = ToolExecutionContext {
                agent_id: Some(agent_id.to_string()),
                project_id: None, // Could be passed through continuation if needed
                call_depth,
                call_chain,
                dispatcher: None,
                audit_log: self.audit_log.clone(),
            };

            let exec_result = tool_registry
                .execute_with_context(&tool_call.name, &tool_call.input, Some(&context))
                .await;

            tracing::info!(
                agent_id = %agent_id,
                tool_name = %tool_call.name,
                success = exec_result.success,
                "Tool execution completed"
            );

            results.push(ToolResult {
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                output: exec_result.output,
                success: exec_result.success,
            });
        }

        Ok(results)
    }

    /// Send message to agent with streaming
    ///
    /// Phase 7: Streams events (tokens, tool calls) via channel as agent processes message
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `message` - Message as JSON value
    /// * `tx` - Channel sender for streaming events
    ///
    /// # Returns
    /// Ok(()) on success, Err if processing fails
    ///
    /// # Errors
    /// - Invalid agent_id
    /// - Actor invocation failure
    /// - Channel send failure (client disconnected)
    pub async fn send_message_stream(
        &self,
        agent_id: &str,
        message: Value,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        // Phase 7.4: Simplified implementation that emits synthetic events
        // Full streaming will require dispatcher.invoke_stream() in future phases

        // Emit a message chunk (simulated streaming)
        if tx
            .send(StreamEvent::MessageChunk {
                content: "Processing your message...".to_string(),
            })
            .await
            .is_err()
        {
            // Client disconnected - stop processing
            return Ok(());
        }

        // Call the regular send_message (non-streaming)
        let response = self.send_message(agent_id, message).await?;

        // Parse response and emit events
        if let Some(content) = response.get("content").and_then(|v| v.as_str()) {
            // Emit content as chunk
            if tx
                .send(StreamEvent::MessageChunk {
                    content: content.to_string(),
                })
                .await
                .is_err()
            {
                return Ok(());
            }
        }

        // Check for tool calls in response
        if let Some(tool_calls) = response.get("tool_calls").and_then(|v| v.as_array()) {
            for tool_call in tool_calls {
                let tool_call_id = tool_call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let tool_name = tool_call
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                // Emit tool call start
                if tx
                    .send(StreamEvent::ToolCallStart {
                        tool_call_id: tool_call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        input: tool_call.get("arguments").cloned(),
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }

                // Emit tool call complete (simulated)
                if tx
                    .send(StreamEvent::ToolCallComplete {
                        tool_call_id: tool_call_id.to_string(),
                        result: "Tool executed".to_string(),
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
        }

        // Emit completion event
        let message_id = response
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("msg_generated");

        if tx
            .send(StreamEvent::MessageComplete {
                message_id: message_id.to_string(),
            })
            .await
            .is_err()
        {
            return Ok(());
        }

        Ok(())
    }

    /// Get agent state
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    ///
    /// # Returns
    /// Current agent state
    pub async fn get_agent(&self, agent_id: &str) -> Result<AgentState> {
        let actor_id = ActorId::new("agents", agent_id)?;

        // Invoke get_state operation
        let response = self
            .dispatcher
            .invoke(actor_id, "get_state".to_string(), Bytes::new())
            .await?;

        // Deserialize response
        serde_json::from_slice(&response).map_err(|e| Error::Internal {
            message: format!("Failed to deserialize AgentState: {}", e),
        })
    }

    /// Update agent
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `update` - Update as JSON value
    ///
    /// # Returns
    /// Updated agent state
    pub async fn update_agent(&self, agent_id: &str, update: Value) -> Result<AgentState> {
        let actor_id = ActorId::new("agents", agent_id)?;

        // Convert JSON value to UpdateAgentRequest
        let update_request: UpdateAgentRequest =
            serde_json::from_value(update).map_err(|e| Error::Internal {
                message: format!("Failed to parse update request: {}", e),
            })?;

        // For now, update individual fields via update_block if needed
        // This is simplified - a full implementation would have an update_agent operation
        // For this phase, we'll just get the current state and return it
        // (Real update would invoke an update operation on the actor)

        // Serialize update
        let payload = serde_json::to_vec(&update_request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize UpdateAgentRequest: {}", e),
        })?;

        // Invoke update_agent operation (we'll need to add this to AgentActor)
        let response = self
            .dispatcher
            .invoke(
                actor_id.clone(),
                "update_agent".to_string(),
                Bytes::from(payload),
            )
            .await?;

        // Deserialize response
        serde_json::from_slice(&response).map_err(|e| Error::Internal {
            message: format!("Failed to deserialize updated AgentState: {}", e),
        })
    }

    /// Delete agent
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    pub async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        let actor_id = ActorId::new("agents", agent_id)?;

        // Invoke delete operation to clear state
        self.dispatcher
            .invoke(actor_id.clone(), "delete_agent".to_string(), Bytes::new())
            .await?;

        // Then deactivate the actor
        self.dispatcher.deactivate(actor_id).await?;

        Ok(())
    }

    /// Update a memory block by label
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `label` - Block label
    /// * `value` - New block value
    ///
    /// # Returns
    /// Ok(()) on success
    pub async fn update_block_by_label(
        &self,
        agent_id: &str,
        label: &str,
        value: String,
    ) -> Result<()> {
        let actor_id = ActorId::new("agents", agent_id)?;

        // Build update request (using internal BlockUpdate struct format)
        let request = serde_json::json!({
            "label": label,
            "value": value,
        });

        // Serialize request
        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize BlockUpdate: {}", e),
        })?;

        // Invoke update_block operation
        self.dispatcher
            .invoke(actor_id, "update_block".to_string(), Bytes::from(payload))
            .await?;

        Ok(())
    }

    /// Append content to a memory block by label
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `label` - Block label (e.g., "persona", "human")
    /// * `content` - Content to append
    ///
    /// # Returns
    /// Ok(()) on success
    pub async fn core_memory_append(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<()> {
        let actor_id = ActorId::new("agents", agent_id)?;

        // Build append request (matches CoreMemoryAppend struct in agent_actor)
        let request = serde_json::json!({
            "label": label,
            "content": content,
        });

        // Serialize request
        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize CoreMemoryAppend: {}", e),
        })?;

        // Invoke core_memory_append operation
        self.dispatcher
            .invoke(
                actor_id,
                "core_memory_append".to_string(),
                Bytes::from(payload),
            )
            .await?;

        Ok(())
    }

    /// Stream message with LLM token streaming (Phase 7.7)
    ///
    /// Returns stream of chunks as LLM generates response.
    /// Currently uses default stream_complete which converts batch to stream.
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `content` - Message content from user
    ///
    /// # Returns
    /// Stream of StreamChunk items
    ///
    /// # Errors
    /// - Invalid agent_id
    /// - Agent not found/created
    /// - LLM call failure
    ///
    /// # TigerStyle
    /// - For now, uses send_message_full then converts to stream
    /// - Phase 7.8 will add true streaming via actor operation
    pub async fn stream_message(
        &self,
        agent_id: &str,
        content: String,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        // Phase 7.7: Simple implementation using batch then convert to stream
        // This makes tests pass while we implement true streaming in Phase 7.8

        // Call batch send_message_full
        let response = self.send_message_full(agent_id, content).await?;

        // Convert response to stream chunks
        let mut chunks = Vec::new();

        // Add each message's content as deltas
        for message in response.messages {
            if !message.content.is_empty() {
                chunks.push(Ok(StreamChunk::ContentDelta {
                    delta: message.content,
                }));
            }

            // Add tool calls if present
            for tool_call in message.tool_calls {
                chunks.push(Ok(StreamChunk::ToolCallStart {
                    id: tool_call.id,
                    name: tool_call.name,
                    input: tool_call.arguments,
                }));
            }
        }

        // Add done chunk
        chunks.push(Ok(StreamChunk::Done {
            stop_reason: "end_turn".to_string(),
        }));

        Ok(Box::pin(futures::stream::iter(chunks)))
    }

    // =========================================================================
    // New methods for single source of truth (HashMap removal)
    // =========================================================================

    /// Insert into archival memory
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `content` - Content to store
    /// * `metadata` - Optional metadata
    ///
    /// # Returns
    /// The entry ID of the created archival entry
    pub async fn archival_insert(
        &self,
        agent_id: &str,
        content: &str,
        metadata: Option<Value>,
    ) -> Result<String> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ArchivalInsertRequest {
            content: content.to_string(),
            metadata,
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ArchivalInsertRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(
                actor_id,
                "archival_insert".to_string(),
                Bytes::from(payload),
            )
            .await?;

        let result: ArchivalInsertResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize ArchivalInsertResponse: {}", e),
            })?;

        Ok(result.entry_id)
    }

    /// Search archival memory
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `query` - Search query
    /// * `limit` - Maximum results to return
    ///
    /// # Returns
    /// Matching archival entries
    pub async fn archival_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ArchivalSearchRequest {
            query: query.to_string(),
            limit,
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ArchivalSearchRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(
                actor_id,
                "archival_search".to_string(),
                Bytes::from(payload),
            )
            .await?;

        let result: ArchivalSearchResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize ArchivalSearchResponse: {}", e),
            })?;

        Ok(result.entries)
    }

    /// Delete an archival entry
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `entry_id` - ID of the entry to delete
    pub async fn archival_delete(&self, agent_id: &str, entry_id: &str) -> Result<()> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ArchivalDeleteRequest {
            entry_id: entry_id.to_string(),
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ArchivalDeleteRequest: {}", e),
        })?;

        self.dispatcher
            .invoke(
                actor_id,
                "archival_delete".to_string(),
                Bytes::from(payload),
            )
            .await?;

        Ok(())
    }

    /// Search conversation messages
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `query` - Search query
    /// * `limit` - Maximum results to return
    ///
    /// # Returns
    /// Matching messages
    pub async fn conversation_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ConversationSearchRequest {
            query: query.to_string(),
            limit,
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ConversationSearchRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(
                actor_id,
                "conversation_search".to_string(),
                Bytes::from(payload),
            )
            .await?;

        let result: ConversationSearchResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize ConversationSearchResponse: {}", e),
            })?;

        Ok(result.messages)
    }

    /// Search conversation messages with date filter
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `query` - Search query
    /// * `start_date` - Optional start date (RFC 3339 format)
    /// * `end_date` - Optional end date (RFC 3339 format)
    /// * `limit` - Maximum results to return
    ///
    /// # Returns
    /// Matching messages within date range
    pub async fn conversation_search_date(
        &self,
        agent_id: &str,
        query: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ConversationSearchDateRequest {
            query: query.to_string(),
            start_date: start_date.map(|s| s.to_string()),
            end_date: end_date.map(|s| s.to_string()),
            limit,
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ConversationSearchDateRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(
                actor_id,
                "conversation_search_date".to_string(),
                Bytes::from(payload),
            )
            .await?;

        let result: ConversationSearchResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize ConversationSearchResponse: {}", e),
            })?;

        Ok(result.messages)
    }

    /// Replace content in a memory block
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `label` - Block label
    /// * `old_content` - Content to find and replace
    /// * `new_content` - Replacement content
    pub async fn core_memory_replace(
        &self,
        agent_id: &str,
        label: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<()> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = CoreMemoryReplaceRequest {
            label: label.to_string(),
            old_content: old_content.to_string(),
            new_content: new_content.to_string(),
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize CoreMemoryReplaceRequest: {}", e),
        })?;

        self.dispatcher
            .invoke(
                actor_id,
                "core_memory_replace".to_string(),
                Bytes::from(payload),
            )
            .await?;

        Ok(())
    }

    /// Get a memory block by label
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `label` - Block label to find
    ///
    /// # Returns
    /// The block if found, None otherwise
    pub async fn get_block_by_label(&self, agent_id: &str, label: &str) -> Result<Option<Block>> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = GetBlockRequest {
            label: label.to_string(),
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize GetBlockRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(actor_id, "get_block".to_string(), Bytes::from(payload))
            .await?;

        let result: GetBlockResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize GetBlockResponse: {}", e),
            })?;

        Ok(result.block)
    }

    /// List messages with pagination
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID string
    /// * `limit` - Maximum messages to return
    /// * `before` - Optional message ID to return messages before
    ///
    /// # Returns
    /// List of messages
    pub async fn list_messages(
        &self,
        agent_id: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<Vec<Message>> {
        let actor_id = ActorId::new("agents", agent_id)?;

        let request = ListMessagesRequest {
            limit,
            before: before.map(|s| s.to_string()),
        };

        let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
            message: format!("Failed to serialize ListMessagesRequest: {}", e),
        })?;

        let response = self
            .dispatcher
            .invoke(actor_id, "list_messages".to_string(), Bytes::from(payload))
            .await?;

        let result: ListMessagesResponse =
            serde_json::from_slice(&response).map_err(|e| Error::Internal {
                message: format!("Failed to deserialize ListMessagesResponse: {}", e),
            })?;

        Ok(result.messages)
    }
}
