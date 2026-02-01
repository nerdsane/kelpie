//! AgentActor implementation
//!
//! TigerStyle: Single actor type for all agent types, state-based configuration.

use super::llm_trait::{LlmClient, LlmMessage, LlmToolCall};
use super::state::AgentActorState;
use crate::models::{
    AgentState, CreateAgentRequest, LettaToolCall, Message, MessageRole, ToolCall,
    UpdateAgentRequest, UsageStats,
};
use crate::security::audit::SharedAuditLog;
use crate::tools::{parse_pause_signal, UnifiedToolRegistry};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::{Actor, ActorContext};
use kelpie_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// AgentActor - implements virtual actor for agents
///
/// Single actor type handles all agent types (MemgptAgent, LettaV1Agent, ReactAgent).
/// Agent-specific behavior is determined by configuration in state, not by actor type.
///
/// TigerStyle: Explicit operations, serializable state, assertions for invariants.
#[derive(Clone)]
pub struct AgentActor {
    /// LLM client for message handling
    llm: Arc<dyn LlmClient>,
    /// Unified tool registry for tool execution
    tool_registry: Arc<UnifiedToolRegistry>,
    /// Optional dispatcher for inter-actor communication (e.g., RegistryActor registration)
    /// If None, self-registration is skipped (backward compatible)
    dispatcher: Option<kelpie_runtime::DispatcherHandle<kelpie_core::TokioRuntime>>,
    /// Audit log for recording tool executions
    /// If None, audit logging is disabled for this actor
    audit_log: Option<SharedAuditLog>,
}

impl AgentActor {
    /// Create a new AgentActor with LLM client
    pub fn new(llm: Arc<dyn LlmClient>, tool_registry: Arc<UnifiedToolRegistry>) -> Self {
        Self {
            llm,
            tool_registry,
            dispatcher: None,
            audit_log: None,
        }
    }

    /// Create AgentActor with dispatcher for self-registration
    pub fn with_dispatcher(
        mut self,
        dispatcher: kelpie_runtime::DispatcherHandle<kelpie_core::TokioRuntime>,
    ) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Create AgentActor with audit logging enabled
    pub fn with_audit_log(mut self, audit_log: SharedAuditLog) -> Self {
        self.audit_log = Some(audit_log);
        self
    }

    // =========================================================================
    // Message Storage Helpers (DRY - avoid duplicate code)
    // =========================================================================

    /// Store an assistant message in the conversation history
    fn store_assistant_message(ctx: &mut ActorContext<AgentActorState>, content: &str) {
        let msg = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: ctx.id.id().to_string(),
            message_type: "assistant_message".to_string(),
            role: MessageRole::Assistant,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };
        ctx.state.add_message(msg);
    }

    /// Store tool call messages for each tool call in the response
    fn store_tool_call_messages(
        ctx: &mut ActorContext<AgentActorState>,
        tool_calls: &[LlmToolCall],
        response_content: &str,
    ) {
        for tool_call in tool_calls {
            let msg = Message {
                id: uuid::Uuid::new_v4().to_string(),
                agent_id: ctx.id.id().to_string(),
                message_type: "tool_call_message".to_string(),
                role: MessageRole::Assistant,
                content: response_content.to_string(),
                tool_call_id: None,
                tool_calls: vec![ToolCall {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    arguments: tool_call.input.clone(),
                }],
                tool_call: Some(LettaToolCall {
                    name: tool_call.name.clone(),
                    arguments: serde_json::to_string(&tool_call.input).unwrap_or_else(|e| {
                        tracing::warn!(
                            tool_call_id = %tool_call.id,
                            tool_name = %tool_call.name,
                            error = %e,
                            "Failed to serialize tool call input, using empty object"
                        );
                        "{}".to_string()
                    }),
                    tool_call_id: tool_call.id.clone(),
                }),
                tool_return: None,
                status: None,
                created_at: chrono::Utc::now(),
            };
            ctx.state.add_message(msg);
        }
    }

    /// Store a tool result message
    fn store_tool_result_message(
        ctx: &mut ActorContext<AgentActorState>,
        tool_call_id: &str,
        output: &str,
        success: bool,
    ) {
        let msg = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: ctx.id.id().to_string(),
            message_type: "tool_return_message".to_string(),
            role: MessageRole::Tool,
            content: output.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: vec![],
            tool_call: None,
            tool_return: Some(output.to_string()),
            status: Some(if success { "success" } else { "error" }.to_string()),
            created_at: chrono::Utc::now(),
        };
        ctx.state.add_message(msg);
    }

    // =========================================================================
    // Response Building Helpers (DRY - reduce cognitive complexity)
    // =========================================================================

    /// Build pending tool calls from LLM response
    ///
    /// TigerStyle: Single responsibility - converts LlmToolCall to PendingToolCall
    fn build_pending_tool_calls(tool_calls: &[LlmToolCall]) -> Vec<PendingToolCall> {
        tool_calls
            .iter()
            .map(|tc| PendingToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            })
            .collect()
    }

    /// Build a Done response with usage stats
    ///
    /// TigerStyle: Single responsibility - constructs final response
    fn build_done_response(
        ctx: &ActorContext<AgentActorState>,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> HandleMessageResult {
        HandleMessageResult::Done(HandleMessageFullResponse {
            messages: ctx.state.all_messages().to_vec(),
            usage: UsageStats {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
        })
    }

    /// Handle "create" operation - initialize agent from request
    ///
    /// Returns the created AgentState directly to avoid timing window
    /// between state creation and persistence (BUG-001 fix)
    async fn handle_create(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: CreateAgentRequest,
    ) -> Result<AgentState> {
        // TigerStyle: Assertions for preconditions
        assert!(ctx.state.agent.is_none(), "Agent already created");

        // Create agent state from request
        let mut agent_state = AgentState::from_request(request);

        // Set agent ID to match actor ID (just the id part, not the full namespace:id)
        agent_state.id = ctx.id.id().to_string();

        // Store in actor state
        ctx.state.agent = Some(agent_state.clone());

        // Return the created state directly (BUG-001 fix)
        Ok(agent_state)
    }

    /// Handle "get_state" operation - return current agent state
    async fn handle_get_state(&self, ctx: &ActorContext<AgentActorState>) -> Result<AgentState> {
        ctx.state.agent().cloned().ok_or_else(|| Error::Internal {
            message: "Agent not created".to_string(),
        })
    }

    /// Handle "update_block" operation - update a memory block
    async fn handle_update_block(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        update: BlockUpdate,
    ) -> Result<()> {
        let updated = ctx.state.update_block(&update.label, |block| {
            block.value = update.value;
            block.updated_at = chrono::Utc::now();
        });

        if !updated {
            return Err(Error::Internal {
                message: format!("Block '{}' not found", update.label),
            });
        }

        Ok(())
    }

    /// Handle "core_memory_append" operation - append to a memory block (or create if doesn't exist)
    ///
    /// Matches the behavior of `append_or_create_block_by_label` - creates the block if it
    /// doesn't exist, otherwise appends to existing content.
    async fn handle_core_memory_append(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        append: CoreMemoryAppend,
    ) -> Result<()> {
        let updated = ctx.state.update_block(&append.label, |block| {
            block.value.push_str(&append.content);
            block.updated_at = chrono::Utc::now();
        });

        if !updated {
            // Block doesn't exist - create it with the content
            ctx.state.create_block(&append.label, &append.content);
        }

        Ok(())
    }

    /// Handle "update_agent" operation - update agent metadata
    async fn handle_update_agent(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        update: UpdateAgentRequest,
    ) -> Result<AgentState> {
        // Get mutable agent state
        let agent = ctx.state.agent_mut().ok_or_else(|| Error::Internal {
            message: "Agent not created".to_string(),
        })?;

        // Apply update
        agent.apply_update(update);

        // Return updated state
        ctx.state.agent().cloned().ok_or_else(|| Error::Internal {
            message: "Agent not created".to_string(),
        })
    }

    /// Handle "delete_agent" operation - mark agent as deleted
    async fn handle_delete_agent(&self, ctx: &mut ActorContext<AgentActorState>) -> Result<()> {
        // Clear agent state to mark as deleted
        ctx.state.agent = None;

        // Delete state from storage
        ctx.kv_delete(b"agent_state").await?;

        Ok(())
    }

    /// Handle "handle_message" operation - process user message with LLM
    async fn handle_handle_message(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: HandleMessageRequest,
    ) -> Result<HandleMessageResponse> {
        // TigerStyle: Validate agent exists
        let agent = ctx.state.agent().ok_or_else(|| Error::Internal {
            message: "Agent not created".to_string(),
        })?;

        // Build prompt from agent blocks
        let mut messages = Vec::new();

        // Add system prompt if present
        if let Some(system) = &agent.system {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: system.clone(),
            });
        }

        // Add memory blocks as system context
        for block in &agent.blocks {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: format!("[{}]\n{}", block.label, block.value),
            });
        }

        // Add user message
        messages.push(LlmMessage {
            role: request.role,
            content: request.content,
        });

        // Call LLM
        let response = self.llm.complete(messages).await?;

        Ok(HandleMessageResponse {
            role: "assistant".to_string(),
            content: response.content,
        })
    }

    /// Handle message with full agent loop (Phase 6.8)
    ///
    /// Implements complete agent behavior:
    /// 1. Add user message to history
    /// 2. Build LLM prompt from agent blocks + history
    /// 3. Call LLM with tools
    /// 4. Execute tool calls (loop up to 5 iterations)
    /// 5. Add assistant response to history
    /// 6. Return all messages + usage stats
    ///
    /// Handle message full - returns HandleMessageResult for continuation-based execution
    ///
    /// CONTINUATION-BASED ARCHITECTURE:
    /// Instead of executing tools inline (which causes reentrant deadlock), this method
    /// returns `NeedTools` when tools are required. The caller (AgentService) executes
    /// tools outside the actor invocation and then calls `continue_with_tool_results`.
    ///
    /// This avoids the deadlock where tools calling dispatcher.invoke() wait on the
    /// same actor that's blocked waiting for those tools to complete.
    async fn handle_message_full(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: HandleMessageFullRequest,
    ) -> Result<HandleMessageResult> {
        // TigerStyle: Validate preconditions
        assert!(
            !request.content.is_empty(),
            "message content must not be empty"
        );

        // Clone agent data before any mutable operations
        let agent = ctx
            .state
            .agent()
            .ok_or_else(|| Error::Internal {
                message: "Agent not created".to_string(),
            })?
            .clone();

        // 1. Create and store user message
        let user_msg = Message {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: ctx.id.id().to_string(),
            message_type: "user_message".to_string(),
            role: MessageRole::User,
            content: request.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            tool_call: None,
            tool_return: None,
            status: None,
            created_at: chrono::Utc::now(),
        };

        // Store user message
        ctx.state.add_message(user_msg.clone());

        // 2. Build LLM messages
        let mut llm_messages = Vec::new();

        // System prompt
        if let Some(system) = &agent.system {
            llm_messages.push(LlmMessage {
                role: "system".to_string(),
                content: system.clone(),
            });
        }

        // Memory blocks as system context
        for block in &agent.blocks {
            llm_messages.push(LlmMessage {
                role: "system".to_string(),
                content: format!("[{}]\n{}", block.label, block.value),
            });
        }

        // Recent message history (last 20)
        for msg in ctx.state.recent_messages(20) {
            // Skip tool and system messages - Claude API doesn't support role "tool"
            if msg.role == MessageRole::Tool || msg.role == MessageRole::System {
                continue;
            }
            let role = match msg.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system", // Won't reach
                MessageRole::Tool => "user",     // Won't reach
            };
            llm_messages.push(LlmMessage {
                role: role.to_string(),
                content: msg.content.clone(),
            });
        }

        // 3. Get tool definitions (filtered by agent capabilities + tool_ids)
        let capabilities = agent.agent_type.capabilities();
        let all_tools = self.tool_registry.get_tool_definitions().await;

        tracing::debug!(
            agent_id = %ctx.id.id(),
            agent_tool_ids = ?agent.tool_ids,
            all_tool_names = ?all_tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
            "Tool filtering inputs"
        );

        // TigerStyle: Tools allowed if in static capabilities OR in agent's tool_ids
        let tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| {
                capabilities.allowed_tools.contains(&t.name) || agent.tool_ids.contains(&t.name)
            })
            .collect();

        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();

        tracing::debug!(
            agent_id = %ctx.id.id(),
            total_tools = tools.len(),
            tool_names = ?tool_names,
            "Loaded tools for LLM prompt"
        );

        // 4. Call LLM with tools
        let llm_start = std::time::Instant::now();
        tracing::info!(
            agent_id = %ctx.id.id(),
            message_count = llm_messages.len(),
            "Starting LLM call"
        );
        let response = self
            .llm
            .complete_with_tools(llm_messages.clone(), tools)
            .await?;
        tracing::info!(
            agent_id = %ctx.id.id(),
            elapsed_ms = llm_start.elapsed().as_millis() as u64,
            prompt_tokens = response.prompt_tokens,
            completion_tokens = response.completion_tokens,
            "LLM call completed"
        );

        let total_prompt_tokens = response.prompt_tokens;
        let total_completion_tokens = response.completion_tokens;

        // 5. Check if tools are needed - if so, return NeedTools for external execution
        if !response.tool_calls.is_empty() {
            tracing::info!(
                agent_id = %ctx.id.id(),
                tool_count = response.tool_calls.len(),
                tool_names = ?response.tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>(),
                "Returning NeedTools - tools will be executed outside actor"
            );

            // Store messages using helpers
            Self::store_assistant_message(ctx, &response.content);
            Self::store_tool_call_messages(ctx, &response.tool_calls, &response.content);

            // Build continuation state
            let continuation = AgentContinuation {
                llm_messages,
                tool_names,
                total_prompt_tokens,
                total_completion_tokens,
                iterations: 0,
                pending_response_content: response.content.clone(),
                call_context: request.call_context.clone(),
                supports_heartbeats: capabilities.supports_heartbeats,
            };

            return Ok(HandleMessageResult::NeedTools {
                tool_calls: Self::build_pending_tool_calls(&response.tool_calls),
                continuation,
            });
        }

        // 6. No tools needed - store final response and return Done
        let final_content = self.extract_send_message_content(&response, ctx).await?;
        Self::store_assistant_message(ctx, &final_content);

        Ok(Self::build_done_response(
            ctx,
            total_prompt_tokens,
            total_completion_tokens,
        ))
    }

    /// Continue processing after tool execution
    ///
    /// This is called by AgentService after executing tools outside the actor invocation.
    /// Takes the tool results and continuation state, continues the LLM conversation,
    /// and may return NeedTools again or Done.
    async fn handle_continue_with_tool_results(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: ContinueWithToolResultsRequest,
    ) -> Result<HandleMessageResult> {
        let agent = ctx
            .state
            .agent()
            .ok_or_else(|| Error::Internal {
                message: "Agent not created".to_string(),
            })?
            .clone();

        let _capabilities = agent.agent_type.capabilities();
        let continuation = request.continuation;

        // Get tool definitions again (needed for LLM call)
        let all_tools = self.tool_registry.get_tool_definitions().await;
        let tools: Vec<_> = all_tools
            .into_iter()
            .filter(|t| continuation.tool_names.contains(&t.name))
            .collect();

        let mut total_prompt_tokens = continuation.total_prompt_tokens;
        let mut total_completion_tokens = continuation.total_completion_tokens;
        let iterations = continuation.iterations + 1;
        const MAX_ITERATIONS: u32 = 5;

        // Store tool result messages and check for pause signals
        let mut should_break = false;
        let mut tool_results_for_llm = Vec::new();

        for tool_result in &request.tool_results {
            // Store tool result message using helper
            Self::store_tool_result_message(
                ctx,
                &tool_result.tool_call_id,
                &tool_result.output,
                tool_result.success,
            );

            tool_results_for_llm
                .push((tool_result.tool_call_id.clone(), tool_result.output.clone()));

            // Check for pause signal
            if let Some((minutes, pause_until_ms)) = parse_pause_signal(&tool_result.output) {
                if continuation.supports_heartbeats {
                    ctx.state.is_paused = true;
                    ctx.state.pause_until_ms = Some(pause_until_ms);
                    should_break = true;
                    tracing::info!(
                        agent_id = %ctx.id.id(),
                        pause_minutes = minutes,
                        pause_until_ms = pause_until_ms,
                        "Agent requested heartbeat pause"
                    );
                }
            }
        }

        // If pause was requested or max iterations reached, return current state
        if should_break || iterations >= MAX_ITERATIONS {
            let final_content = continuation.pending_response_content.clone();
            Self::store_assistant_message(ctx, &final_content);

            return Ok(HandleMessageResult::Done(HandleMessageFullResponse {
                messages: ctx.state.all_messages().to_vec(),
                usage: UsageStats {
                    prompt_tokens: total_prompt_tokens,
                    completion_tokens: total_completion_tokens,
                    total_tokens: total_prompt_tokens + total_completion_tokens,
                },
            }));
        }

        // Build assistant content blocks for LLM continuation
        let mut assistant_blocks = Vec::new();
        if !continuation.pending_response_content.is_empty() {
            assistant_blocks.push(crate::llm::ContentBlock::Text {
                text: continuation.pending_response_content.clone(),
            });
        }
        // Add tool use blocks from the previous response (reconstructed from tool results)
        for tool_result in &request.tool_results {
            // Find the input for this tool call from stored messages
            let input = ctx
                .state
                .all_messages()
                .iter()
                .rev()
                .find_map(|m| {
                    m.tool_calls
                        .iter()
                        .find(|tc| tc.id == tool_result.tool_call_id)
                        .map(|tc| tc.arguments.clone())
                })
                .unwrap_or_else(|| {
                    tracing::warn!(
                        tool_call_id = %tool_result.tool_call_id,
                        tool_name = %tool_result.tool_name,
                        "Tool call input not found in message history, using empty object"
                    );
                    serde_json::json!({})
                });

            assistant_blocks.push(crate::llm::ContentBlock::ToolUse {
                id: tool_result.tool_call_id.clone(),
                name: tool_result.tool_name.clone(),
                input,
            });
        }

        // Continue conversation with LLM
        tracing::info!(
            agent_id = %ctx.id.id(),
            tool_results_count = tool_results_for_llm.len(),
            iteration = iterations,
            "Continuing LLM conversation after tool execution"
        );

        let response = self
            .llm
            .continue_with_tool_result(
                continuation.llm_messages.clone(),
                tools.clone(),
                assistant_blocks,
                tool_results_for_llm,
            )
            .await?;

        total_prompt_tokens += response.prompt_tokens;
        total_completion_tokens += response.completion_tokens;

        tracing::info!(
            agent_id = %ctx.id.id(),
            prompt_tokens = response.prompt_tokens,
            completion_tokens = response.completion_tokens,
            has_tool_calls = !response.tool_calls.is_empty(),
            "LLM continuation completed"
        );

        // Check if more tools are needed
        if !response.tool_calls.is_empty() {
            tracing::info!(
                agent_id = %ctx.id.id(),
                tool_count = response.tool_calls.len(),
                tool_names = ?response.tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>(),
                "Returning NeedTools again - more tools required"
            );

            // Store messages using helpers
            Self::store_assistant_message(ctx, &response.content);
            Self::store_tool_call_messages(ctx, &response.tool_calls, &response.content);

            let new_continuation = AgentContinuation {
                llm_messages: continuation.llm_messages,
                tool_names: continuation.tool_names,
                total_prompt_tokens,
                total_completion_tokens,
                iterations,
                pending_response_content: response.content.clone(),
                call_context: continuation.call_context,
                supports_heartbeats: continuation.supports_heartbeats,
            };

            return Ok(HandleMessageResult::NeedTools {
                tool_calls: Self::build_pending_tool_calls(&response.tool_calls),
                continuation: new_continuation,
            });
        }

        // Done - no more tools needed
        let final_content = self.extract_send_message_content(&response, ctx).await?;
        Self::store_assistant_message(ctx, &final_content);

        Ok(Self::build_done_response(
            ctx,
            total_prompt_tokens,
            total_completion_tokens,
        ))
    }

    /// Extract send_message content for dual-mode support
    ///
    /// If the LLM response includes send_message tool calls, extract and return
    /// the message content from those calls. Supports multiple send_message calls
    /// in one turn (concatenates them). If no send_message calls, returns the
    /// direct LLM response content as fallback.
    ///
    /// This implements Letta's dual-mode messaging:
    /// - Agent calls send_message("text") -> use that text
    /// - Agent doesn't call send_message -> use LLM's direct response
    async fn extract_send_message_content(
        &self,
        response: &super::llm_trait::LlmResponse,
        _ctx: &ActorContext<AgentActorState>,
    ) -> Result<String> {
        // Find all send_message tool calls
        let send_message_calls: Vec<&LlmToolCall> = response
            .tool_calls
            .iter()
            .filter(|tc| tc.name == "send_message")
            .collect();

        // If no send_message calls, use direct LLM response (fallback mode)
        if send_message_calls.is_empty() {
            return Ok(response.content.clone());
        }

        // Extract message content from send_message calls
        let mut messages = Vec::new();
        for tool_call in send_message_calls {
            if let Some(message) = tool_call.input.get("message").and_then(|v| v.as_str()) {
                messages.push(message.to_string());
            }
        }

        // Concatenate multiple send_message calls with newlines
        if messages.is_empty() {
            // send_message was called but no valid message parameter - return direct response
            Ok(response.content.clone())
        } else {
            // Use send_message content (join multiple calls with newlines)
            Ok(messages.join("\n\n"))
        }
    }

    // =========================================================================
    // New handlers for single source of truth operations
    // =========================================================================

    /// Handle "archival_insert" operation - insert into archival memory
    async fn handle_archival_insert(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: ArchivalInsertRequest,
    ) -> Result<ArchivalInsertResponse> {
        let entry = ctx
            .state
            .add_archival_entry(request.content, request.metadata)
            .map_err(|e| Error::Internal { message: e })?;

        Ok(ArchivalInsertResponse { entry_id: entry.id })
    }

    /// Handle "archival_search" operation - search archival memory
    async fn handle_archival_search(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: ArchivalSearchRequest,
    ) -> Result<ArchivalSearchResponse> {
        let entries = ctx
            .state
            .search_archival(Some(&request.query), request.limit);
        Ok(ArchivalSearchResponse { entries })
    }

    /// Handle "archival_delete" operation - delete from archival memory
    async fn handle_archival_delete(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: ArchivalDeleteRequest,
    ) -> Result<()> {
        ctx.state
            .delete_archival_entry(&request.entry_id)
            .map_err(|e| Error::Internal { message: e })
    }

    /// Handle "conversation_search" operation - search messages
    async fn handle_conversation_search(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: ConversationSearchRequest,
    ) -> Result<ConversationSearchResponse> {
        let messages = ctx.state.search_messages(&request.query, request.limit);
        Ok(ConversationSearchResponse { messages })
    }

    /// Handle "conversation_search_date" operation - search messages with date filter
    async fn handle_conversation_search_date(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: ConversationSearchDateRequest,
    ) -> Result<ConversationSearchResponse> {
        // Parse dates
        let start_date = request
            .start_date
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let end_date = request
            .end_date
            .as_ref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let messages = ctx.state.search_messages_with_date(
            &request.query,
            start_date,
            end_date,
            request.limit,
        );

        Ok(ConversationSearchResponse { messages })
    }

    /// Handle "core_memory_replace" operation - replace content in a memory block
    async fn handle_core_memory_replace(
        &self,
        ctx: &mut ActorContext<AgentActorState>,
        request: CoreMemoryReplaceRequest,
    ) -> Result<()> {
        ctx.state
            .replace_block_content(&request.label, &request.old_content, &request.new_content)
            .map_err(|e| Error::Internal { message: e })
    }

    /// Handle "get_block" operation - get a memory block by label
    async fn handle_get_block(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: GetBlockRequest,
    ) -> Result<GetBlockResponse> {
        let block = ctx.state.get_block(&request.label).cloned();
        Ok(GetBlockResponse { block })
    }

    /// Handle "list_messages" operation - list messages with pagination
    async fn handle_list_messages(
        &self,
        ctx: &ActorContext<AgentActorState>,
        request: ListMessagesRequest,
    ) -> Result<ListMessagesResponse> {
        let messages = ctx
            .state
            .list_messages_paginated(request.limit, request.before.as_deref());
        Ok(ListMessagesResponse { messages })
    }
}

/// Block update request
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlockUpdate {
    label: String,
    value: String,
}

/// Core memory append request
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoreMemoryAppend {
    label: String,
    content: String,
}

/// Request for full message handling (Phase 6.8)
///
/// TigerStyle (Issue #75 fix): Includes optional call context for nested agent calls.
/// When Agent A calls Agent B, A's call context is propagated to B for:
/// - Cycle detection (prevent A→B→A deadlock)
/// - Depth tracking (limit nested calls)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleMessageFullRequest {
    pub content: String,
    /// Optional call context for nested agent-to-agent calls
    /// None for top-level calls (from API), Some for nested calls (from call_agent tool)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_context: Option<CallContextInfo>,
}

/// Call context information propagated through agent-to-agent calls
///
/// TigerStyle: Explicit state for cycle detection and depth limiting.
/// TLA+ invariants: NoDeadlock, DepthBounded (see KelpieMultiAgentInvocation.tla)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CallContextInfo {
    /// Current call depth (0 = top level, increments with each nested call)
    pub call_depth: u32,
    /// Chain of agent IDs in the current call stack (for cycle detection)
    pub call_chain: Vec<String>,
}

/// Response from full message handling (Phase 6.8)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandleMessageFullResponse {
    pub messages: Vec<Message>,
    pub usage: UsageStats,
}

// =============================================================================
// Continuation-Based Tool Execution Types
// =============================================================================
//
// These types enable tool execution OUTSIDE actor invocations, avoiding the
// reentrant deadlock that occurs when tools call the dispatcher from within
// an active invocation.
//
// Flow:
// 1. AgentService calls "handle_message_full" or "start_message"
// 2. Actor returns NeedTools { tool_calls, continuation } if tools needed
// 3. AgentService executes tools OUTSIDE actor (can call dispatcher freely)
// 4. AgentService calls "continue_with_tool_results" with results
// 5. Actor continues, may return NeedTools again or Done

/// Result from handle_message_full - either done or needs tools executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandleMessageResult {
    /// Processing complete, here's the final response
    Done(HandleMessageFullResponse),
    /// Need tools executed before continuing
    NeedTools {
        /// Tools to execute (outside actor invocation)
        tool_calls: Vec<PendingToolCall>,
        /// State needed to resume after tool execution
        continuation: AgentContinuation,
    },
}

/// A tool call that needs to be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// State captured to resume agent processing after tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContinuation {
    /// LLM messages built so far (system + history)
    pub llm_messages: Vec<LlmMessage>,
    /// Tool definitions available
    pub tool_names: Vec<String>,
    /// Running token counts
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    /// Current iteration count
    pub iterations: u32,
    /// The LLM response that requested tools
    pub pending_response_content: String,
    /// Call context for nested agent calls
    pub call_context: Option<CallContextInfo>,
    /// Agent capabilities (for pause detection)
    pub supports_heartbeats: bool,
}

/// Request to continue after tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinueWithToolResultsRequest {
    /// Results from tool execution
    pub tool_results: Vec<ToolResult>,
    /// Continuation state from NeedTools
    pub continuation: AgentContinuation,
}

/// Result from a single tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub success: bool,
}

/// Handle message request
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandleMessageRequest {
    role: String,
    content: String,
}

/// Handle message response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandleMessageResponse {
    role: String,
    content: String,
}

// =========================================================================
// New operation request/response types for single source of truth
// =========================================================================

/// Archival memory insert request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalInsertRequest {
    pub content: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Archival memory insert response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalInsertResponse {
    pub entry_id: String,
}

/// Archival memory search request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalSearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    10
}

/// Archival memory search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalSearchResponse {
    pub entries: Vec<crate::models::ArchivalEntry>,
}

/// Archival memory delete request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivalDeleteRequest {
    pub entry_id: String,
}

/// Conversation search request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

/// Conversation search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSearchResponse {
    pub messages: Vec<Message>,
}

/// Conversation search with date filter request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSearchDateRequest {
    pub query: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

/// Core memory replace request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreMemoryReplaceRequest {
    pub label: String,
    pub old_content: String,
    pub new_content: String,
}

/// Get block by label request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockRequest {
    pub label: String,
}

/// Get block response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlockResponse {
    pub block: Option<crate::models::Block>,
}

/// List messages request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMessagesRequest {
    #[serde(default = "default_messages_limit")]
    pub limit: usize,
    #[serde(default)]
    pub before: Option<String>,
}

fn default_messages_limit() -> usize {
    100
}

/// List messages response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMessagesResponse {
    pub messages: Vec<Message>,
}

#[async_trait]
impl Actor for AgentActor {
    type State = AgentActorState;

    async fn invoke(
        &self,
        ctx: &mut ActorContext<Self::State>,
        operation: &str,
        payload: Bytes,
    ) -> Result<Bytes> {
        // TigerStyle: Explicit operation routing with clear error messages
        match operation {
            "create" => {
                let request: CreateAgentRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize CreateAgentRequest: {}", e),
                    })?;
                let agent_state = self.handle_create(ctx, request).await?;
                let response = serde_json::to_vec(&agent_state).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize AgentState: {}", e),
                })?;
                Ok(Bytes::from(response))
            }
            "get_state" => {
                let state = self.handle_get_state(ctx).await?;
                let response = serde_json::to_vec(&state).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize AgentState: {}", e),
                })?;
                Ok(Bytes::from(response))
            }
            "update_block" => {
                let update: BlockUpdate =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize BlockUpdate: {}", e),
                    })?;
                self.handle_update_block(ctx, update).await?;
                Ok(Bytes::from("{}"))
            }
            "core_memory_append" => {
                let append: CoreMemoryAppend =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize CoreMemoryAppend: {}", e),
                    })?;
                self.handle_core_memory_append(ctx, append).await?;
                Ok(Bytes::from("{}"))
            }
            "update_agent" => {
                let update: UpdateAgentRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize UpdateAgentRequest: {}", e),
                    })?;
                let response = self.handle_update_agent(ctx, update).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize AgentState: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "handle_message" => {
                let request: HandleMessageRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize HandleMessageRequest: {}", e),
                    })?;
                let response = self.handle_handle_message(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize HandleMessageResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "handle_message_full" => {
                let request: HandleMessageFullRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize HandleMessageFullRequest: {}", e),
                    })?;
                let response = self.handle_message_full(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize HandleMessageResult: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "continue_with_tool_results" => {
                let request: ContinueWithToolResultsRequest = serde_json::from_slice(&payload)
                    .map_err(|e| Error::Internal {
                        message: format!(
                            "Failed to deserialize ContinueWithToolResultsRequest: {}",
                            e
                        ),
                    })?;
                let response = self.handle_continue_with_tool_results(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize HandleMessageResult: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "delete_agent" => {
                self.handle_delete_agent(ctx).await?;
                Ok(Bytes::from("{}"))
            }
            // =========================================================================
            // New operations for single source of truth
            // =========================================================================
            "archival_insert" => {
                let request: ArchivalInsertRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ArchivalInsertRequest: {}", e),
                    })?;
                let response = self.handle_archival_insert(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ArchivalInsertResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "archival_search" => {
                let request: ArchivalSearchRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ArchivalSearchRequest: {}", e),
                    })?;
                let response = self.handle_archival_search(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ArchivalSearchResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "archival_delete" => {
                let request: ArchivalDeleteRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ArchivalDeleteRequest: {}", e),
                    })?;
                self.handle_archival_delete(ctx, request).await?;
                Ok(Bytes::from("{}"))
            }
            "conversation_search" => {
                let request: ConversationSearchRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ConversationSearchRequest: {}", e),
                    })?;
                let response = self.handle_conversation_search(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ConversationSearchResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "conversation_search_date" => {
                let request: ConversationSearchDateRequest = serde_json::from_slice(&payload)
                    .map_err(|e| Error::Internal {
                        message: format!(
                            "Failed to deserialize ConversationSearchDateRequest: {}",
                            e
                        ),
                    })?;
                let response = self.handle_conversation_search_date(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ConversationSearchResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "core_memory_replace" => {
                let request: CoreMemoryReplaceRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize CoreMemoryReplaceRequest: {}", e),
                    })?;
                self.handle_core_memory_replace(ctx, request).await?;
                Ok(Bytes::from("{}"))
            }
            "get_block" => {
                let request: GetBlockRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize GetBlockRequest: {}", e),
                    })?;
                let response = self.handle_get_block(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize GetBlockResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            "list_messages" => {
                let request: ListMessagesRequest =
                    serde_json::from_slice(&payload).map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize ListMessagesRequest: {}", e),
                    })?;
                let response = self.handle_list_messages(ctx, request).await?;
                let response_bytes =
                    serde_json::to_vec(&response).map_err(|e| Error::Internal {
                        message: format!("Failed to serialize ListMessagesResponse: {}", e),
                    })?;
                Ok(Bytes::from(response_bytes))
            }
            _ => Err(Error::Internal {
                message: format!("Unknown operation: {}", operation),
            }),
        }
    }

    async fn on_activate(&self, ctx: &mut ActorContext<Self::State>) -> Result<()> {
        // TigerStyle: Log activation for debugging
        debug_assert!(!ctx.id.id().is_empty(), "ActorId must not be empty");

        // Try to load existing state from KV store
        let state_key = b"agent_state";
        if let Some(state_bytes) = ctx.kv_get(state_key).await? {
            let loaded_state: AgentActorState =
                serde_json::from_slice(&state_bytes).map_err(|e| Error::Internal {
                    message: format!("Failed to deserialize AgentActorState: {}", e),
                })?;
            ctx.state = loaded_state;
        }
        // If no state exists, that's OK - will be created on first "create" operation

        // Phase 2: Registry Gap Fix
        // Self-register in the global registry if we have state (i.e., we are a valid agent)
        if let Some(agent) = &ctx.state.agent {
            // We need to write to "system/agent_registry"
            // Since we don't have direct access to other actors' storage, we rely on the
            // storage layer's ability to access the registry keyspace if configured.
            // However, ActorContext only gives access to *our* storage.

            // In the current architecture, the Actor cannot write to the Registry directly
            // unless the Registry is part of its own storage or shared storage.
            // FdbAgentRegistry uses a shared FDB connection.
            // KvAdapter uses a shared ActorKV.

            // If we are running with KvAdapter, we are in a simulation or test.
            // We can try to write to a "well-known" key in our own storage that the Registry scanner looks for?
            // No, the Registry scanner looks at "system/agent_registry".

            // Ideally, we would send a message to the Registry actor:
            // ctx.send(registry_id, RegisterAgent(agent)).await?
            // But we don't have a Registry actor implemented as an Actor yet (it's just a storage abstraction).

            // Workaround: Write a "metadata" key in our own namespace.
            // And update KvAdapter to scan all actors? No, too expensive.

            // Correct Fix: The "Registry" should be an Actor.
            // But refactoring Registry to be an Actor is a larger task.

            // For now, we will rely on the Service to register the agent on creation (which it does via AppState).
            // But for Teleport/Recovery, the Service isn't involved.

            // Registry Gap: FIXED with RegistryActor (Option 1)
            //
            // Self-registration is now implemented via message passing to RegistryActor.
            // If dispatcher is available, agent registers itself on activation.
            //
            // Registration paths by scenario:
            // 1. Normal creation: API → AppState → storage.save_agent() → AgentActor.create() → on_activate self-registers
            // 2. Teleport in: TeleportService.teleport_in() → restore → on_activate self-registers
            // 3. Recovery: Actor restarts → on_activate self-registers (idempotent)
            //
            // Backward compatible: If dispatcher is None, registration is handled by service layer (Option 2)

            if let Some(ref dispatcher) = self.dispatcher {
                // Convert AgentState to AgentMetadata for registry
                use crate::storage::AgentMetadata;
                let metadata = AgentMetadata {
                    id: agent.id.clone(),
                    name: agent.name.clone(),
                    agent_type: agent.agent_type.clone(),
                    model: agent.model.clone(),
                    embedding: agent.embedding.clone(),
                    system: agent.system.clone(),
                    description: agent.description.clone(),
                    tool_ids: agent.tool_ids.clone(),
                    tags: agent.tags.clone(),
                    metadata: agent.metadata.clone(),
                    created_at: agent.created_at,
                    updated_at: agent.updated_at,
                };

                // Send register message to RegistryActor
                let registry_id = kelpie_core::actor::ActorId::new("system", "agent_registry")?;
                let request = super::registry_actor::RegisterRequest { metadata };
                let payload = serde_json::to_vec(&request).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize RegisterRequest: {}", e),
                })?;

                match dispatcher
                    .invoke(registry_id, "register".to_string(), Bytes::from(payload))
                    .await
                {
                    Ok(_) => {
                        tracing::info!(agent_id = %agent.id, "Agent self-registered via RegistryActor");
                    }
                    Err(e) => {
                        // Non-fatal: registration failure doesn't prevent actor activation
                        tracing::warn!(
                            agent_id = %agent.id,
                            error = %e,
                            "Failed to self-register with RegistryActor (non-fatal)"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    agent_id = %agent.id,
                    "Agent activated (no dispatcher, registry managed by service layer)"
                );
            }
        }

        Ok(())
    }

    async fn on_deactivate(&self, ctx: &mut ActorContext<Self::State>) -> Result<()> {
        // Phase 2: Storage Unification
        // Write granular keys for API compatibility AND the BLOB for fast recovery

        // TigerStyle: Keep agent_state BLOB for fast Actor recovery
        let state_key = b"agent_state";
        let state_bytes = serde_json::to_vec(&ctx.state).map_err(|e| Error::Internal {
            message: format!("Failed to serialize AgentActorState: {}", e),
        })?;
        ctx.kv_set(state_key, &state_bytes).await?;

        // Write granular keys for API (AgentStorage) compatibility
        // 1. Write memory blocks
        if let Some(agent) = &ctx.state.agent {
            let blocks_value = serde_json::to_vec(&agent.blocks).map_err(|e| Error::Internal {
                message: format!("Failed to serialize blocks: {}", e),
            })?;
            ctx.kv_set(b"blocks", &blocks_value).await?;
        }

        // 2. Write messages as individual keys (message:0, message:1, ...)
        let message_count = ctx.state.messages.len() as u64;
        for (idx, message) in ctx.state.messages.iter().enumerate() {
            let message_key = format!("message:{}", idx);
            let message_value = serde_json::to_vec(message).map_err(|e| Error::Internal {
                message: format!("Failed to serialize message {}: {}", idx, e),
            })?;
            ctx.kv_set(message_key.as_bytes(), &message_value).await?;
        }

        // 3. Write message count
        let count_value = Bytes::from(message_count.to_string());
        ctx.kv_set(b"message_count", &count_value).await?;

        // 4. Write archival entries as individual keys (archival:0, archival:1, ...)
        let archival_count = ctx.state.archival.len() as u64;
        for (idx, entry) in ctx.state.archival.iter().enumerate() {
            let archival_key = format!("archival:{}", idx);
            let archival_value = serde_json::to_vec(entry).map_err(|e| Error::Internal {
                message: format!("Failed to serialize archival entry {}: {}", idx, e),
            })?;
            ctx.kv_set(archival_key.as_bytes(), &archival_value).await?;
        }

        // 5. Write archival count
        let archival_count_value = Bytes::from(archival_count.to_string());
        ctx.kv_set(b"archival_count", &archival_count_value).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::llm_trait::{LlmResponse, LlmToolCall};
    use crate::tools::UnifiedToolRegistry;
    use kelpie_core::actor::{ActorId, NoOpKV};
    use serde_json::json;

    /// Test dual-mode: send_message tool call content is extracted
    #[tokio::test]
    async fn test_extract_send_message_content_single() {
        let actor = AgentActor::new(Arc::new(MockLlm), Arc::new(UnifiedToolRegistry::new()));

        let response = LlmResponse {
            content: "This should be ignored".to_string(),
            tool_calls: vec![LlmToolCall {
                id: "call_1".to_string(),
                name: "send_message".to_string(),
                input: json!({
                    "message": "Hello from send_message!"
                }),
            }],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        };

        let kv = Box::new(NoOpKV);
        let actor_id = ActorId::new("test", "agent1").unwrap();
        let state = AgentActorState::default();
        let ctx = ActorContext::new(actor_id, state, kv);

        let result = actor
            .extract_send_message_content(&response, &ctx)
            .await
            .unwrap();
        assert_eq!(result, "Hello from send_message!");
    }

    /// Test dual-mode: multiple send_message calls are concatenated
    #[tokio::test]
    async fn test_extract_send_message_content_multiple() {
        let actor = AgentActor::new(Arc::new(MockLlm), Arc::new(UnifiedToolRegistry::new()));

        let response = LlmResponse {
            content: "This should be ignored".to_string(),
            tool_calls: vec![
                LlmToolCall {
                    id: "call_1".to_string(),
                    name: "send_message".to_string(),
                    input: json!({
                        "message": "First message"
                    }),
                },
                LlmToolCall {
                    id: "call_2".to_string(),
                    name: "send_message".to_string(),
                    input: json!({
                        "message": "Second message"
                    }),
                },
            ],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        };

        let kv = Box::new(NoOpKV);
        let actor_id = ActorId::new("test", "agent1").unwrap();
        let state = AgentActorState::default();
        let ctx = ActorContext::new(actor_id, state, kv);

        let result = actor
            .extract_send_message_content(&response, &ctx)
            .await
            .unwrap();
        assert_eq!(result, "First message\n\nSecond message");
    }

    /// Test dual-mode: no send_message call uses direct LLM response (fallback)
    #[tokio::test]
    async fn test_extract_send_message_content_fallback() {
        let actor = AgentActor::new(Arc::new(MockLlm), Arc::new(UnifiedToolRegistry::new()));

        let response = LlmResponse {
            content: "Direct LLM response".to_string(),
            tool_calls: vec![LlmToolCall {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                input: json!({
                    "command": "ls"
                }),
            }],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        };

        let kv = Box::new(NoOpKV);
        let actor_id = ActorId::new("test", "agent1").unwrap();
        let state = AgentActorState::default();
        let ctx = ActorContext::new(actor_id, state, kv);

        let result = actor
            .extract_send_message_content(&response, &ctx)
            .await
            .unwrap();
        assert_eq!(result, "Direct LLM response");
    }

    /// Test dual-mode: empty tool calls uses direct response
    #[tokio::test]
    async fn test_extract_send_message_content_no_tools() {
        let actor = AgentActor::new(Arc::new(MockLlm), Arc::new(UnifiedToolRegistry::new()));

        let response = LlmResponse {
            content: "Direct response, no tools".to_string(),
            tool_calls: vec![],
            prompt_tokens: 0,
            completion_tokens: 0,
            stop_reason: "end_turn".to_string(),
        };

        let kv = Box::new(NoOpKV);
        let actor_id = ActorId::new("test", "agent1").unwrap();
        let state = AgentActorState::default();
        let ctx = ActorContext::new(actor_id, state, kv);

        let result = actor
            .extract_send_message_content(&response, &ctx)
            .await
            .unwrap();
        assert_eq!(result, "Direct response, no tools");
    }

    /// Mock LLM for testing
    struct MockLlm;

    #[async_trait]
    impl LlmClient for MockLlm {
        async fn complete_with_tools(
            &self,
            _messages: Vec<LlmMessage>,
            _tools: Vec<crate::llm::ToolDefinition>,
        ) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Mock response".to_string(),
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
        ) -> Result<LlmResponse> {
            Ok(LlmResponse {
                content: "Mock response".to_string(),
                tool_calls: vec![],
                prompt_tokens: 0,
                completion_tokens: 0,
                stop_reason: "end_turn".to_string(),
            })
        }
    }
}
