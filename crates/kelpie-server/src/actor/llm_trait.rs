//! LLM client trait for AgentActor
//!
//! TigerStyle: Trait abstraction allows both real LLM and SimLlmClient

use async_trait::async_trait;
use futures::stream::{self, Stream, StreamExt};
use kelpie_core::Result;
use serde_json::Value;
use std::pin::Pin;

/// Chat message for LLM
#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

// Manual implementation for serialization in AgentContinuation
impl serde::Serialize for LlmMessage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("LlmMessage", 2)?;
        state.serialize_field("role", &self.role)?;
        state.serialize_field("content", &self.content)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for LlmMessage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct LlmMessageHelper {
            role: String,
            content: String,
        }
        let helper = LlmMessageHelper::deserialize(deserializer)?;
        Ok(LlmMessage {
            role: helper.role,
            content: helper.content,
        })
    }
}

/// Response from LLM completion
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<LlmToolCall>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub stop_reason: String,
}

/// Tool call from LLM
#[derive(Debug, Clone)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Stream chunk from LLM streaming (Phase 7.7)
///
/// Represents incremental pieces of LLM response as they arrive.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Content delta (token or partial token)
    ContentDelta { delta: String },

    /// Tool call starting
    ToolCallStart {
        id: String,
        name: String,
        input: Value,
    },

    /// Tool call argument delta (streaming tool input)
    ToolCallDelta { id: String, delta: String },

    /// Stream complete
    Done { stop_reason: String },
}

/// LLM client trait - abstraction over real and simulated LLM
///
/// Phase 7.7: Extended with streaming support via stream_complete()
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Complete a chat conversation (batch mode) with tool support
    async fn complete_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<crate::llm::ToolDefinition>,
    ) -> Result<LlmResponse>;

    /// Continue a conversation after tool execution
    async fn continue_with_tool_result(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<crate::llm::ToolDefinition>,
        assistant_blocks: Vec<crate::llm::ContentBlock>,
        tool_results: Vec<(String, String)>,
    ) -> Result<LlmResponse>;

    /// Complete a chat conversation (batch mode) without tools
    async fn complete(&self, messages: Vec<LlmMessage>) -> Result<LlmResponse> {
        self.complete_with_tools(messages, vec![]).await
    }

    /// Complete with streaming (Phase 7.7)
    ///
    /// Returns stream of chunks as they arrive from LLM.
    /// Default implementation converts batch complete() to stream.
    ///
    /// # Arguments
    /// * `messages` - Conversation history
    ///
    /// # Returns
    /// Stream of StreamChunk items
    ///
    /// # Errors
    /// Returns error if LLM call fails
    async fn stream_complete(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        // Default: convert batch to stream for backward compatibility
        let response = self.complete(messages).await?;

        // Create stream from batch response
        let mut chunks = Vec::new();

        // Add content as single delta
        if !response.content.is_empty() {
            chunks.push(Ok(StreamChunk::ContentDelta {
                delta: response.content,
            }));
        }

        // Add tool calls as ToolCallStart chunks
        for tool_call in response.tool_calls {
            chunks.push(Ok(StreamChunk::ToolCallStart {
                id: tool_call.id,
                name: tool_call.name,
                input: tool_call.input,
            }));
        }

        // Add done chunk
        chunks.push(Ok(StreamChunk::Done {
            stop_reason: response.stop_reason,
        }));

        Ok(Box::pin(stream::iter(chunks)))
    }
}

/// Adapter to use crate::llm::LlmClient with the actor LlmClient trait
///
/// Phase 6.4: Enables production AppState to use real LLM with actor service
pub struct RealLlmAdapter {
    client: crate::llm::LlmClient,
}

impl RealLlmAdapter {
    /// Create adapter from real LLM client
    pub fn new(client: crate::llm::LlmClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmClient for RealLlmAdapter {
    async fn complete_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<crate::llm::ToolDefinition>,
    ) -> Result<LlmResponse> {
        // Convert actor LlmMessage to llm::ChatMessage
        let chat_messages: Vec<crate::llm::ChatMessage> = messages
            .into_iter()
            .map(|m| crate::llm::ChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let response = self
            .client
            .complete_with_tools(chat_messages, tools)
            .await
            .map_err(|e| kelpie_core::Error::Internal {
                message: format!("LLM completion failed: {}", e),
            })?;

        // Convert CompletionResponse to LlmResponse
        Ok(LlmResponse {
            content: response.content,
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|tc| LlmToolCall {
                    id: tc.id,
                    name: tc.name,
                    input: tc.input,
                })
                .collect(),
            prompt_tokens: response.prompt_tokens,
            completion_tokens: response.completion_tokens,
            stop_reason: response.stop_reason,
        })
    }

    async fn continue_with_tool_result(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<crate::llm::ToolDefinition>,
        assistant_blocks: Vec<crate::llm::ContentBlock>,
        tool_results: Vec<(String, String)>,
    ) -> Result<LlmResponse> {
        let chat_messages: Vec<crate::llm::ChatMessage> = messages
            .into_iter()
            .map(|m| crate::llm::ChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let response = self
            .client
            .continue_with_tool_result(chat_messages, tools, assistant_blocks, tool_results)
            .await
            .map_err(|e| kelpie_core::Error::Internal {
                message: format!("LLM tool continuation failed: {}", e),
            })?;

        Ok(LlmResponse {
            content: response.content,
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|tc| LlmToolCall {
                    id: tc.id,
                    name: tc.name,
                    input: tc.input,
                })
                .collect(),
            prompt_tokens: response.prompt_tokens,
            completion_tokens: response.completion_tokens,
            stop_reason: response.stop_reason,
        })
    }

    /// Stream complete with real LLM streaming (Phase 7.8 REDO - TRUE DST)
    ///
    /// Uses crate::llm::LlmClient.stream_complete_with_tools() for true token streaming.
    /// Tested with HTTP mocking and fault injection.
    async fn stream_complete(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        // Convert actor LlmMessage to llm::ChatMessage
        let chat_messages: Vec<crate::llm::ChatMessage> = messages
            .into_iter()
            .map(|m| crate::llm::ChatMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        // Call real LLM streaming without tools (simplified for actor invocation)
        let stream = self
            .client
            .stream_complete_with_tools(chat_messages, vec![])
            .await
            .map_err(|e| kelpie_core::Error::Internal {
                message: format!("LLM streaming failed: {}", e),
            })?;

        // Convert StreamDelta to StreamChunk, tracking tool call ID across deltas
        // Use scan to maintain state (current tool call ID) across stream items
        let chunk_stream = stream.scan(String::new(), |current_tool_id, delta_result| {
            let result = delta_result
                .map_err(|e| kelpie_core::Error::Internal {
                    message: format!("Stream error: {}", e),
                })
                .map(|delta| match delta {
                    crate::llm::StreamDelta::ContentDelta { text } => {
                        StreamChunk::ContentDelta { delta: text }
                    }
                    crate::llm::StreamDelta::ToolCallStart { id, name } => {
                        // Track the tool call ID for subsequent deltas
                        *current_tool_id = id.clone();
                        StreamChunk::ToolCallStart {
                            id,
                            name,
                            input: serde_json::Value::Null, // Will be filled by deltas
                        }
                    }
                    crate::llm::StreamDelta::ToolCallDelta { delta } => {
                        // Use the tracked tool call ID
                        StreamChunk::ToolCallDelta {
                            id: current_tool_id.clone(),
                            delta,
                        }
                    }
                    crate::llm::StreamDelta::Done { stop_reason } => {
                        StreamChunk::Done { stop_reason }
                    }
                });
            async move { Some(result) }
        });

        Ok(Box::pin(chunk_stream))
    }
}
