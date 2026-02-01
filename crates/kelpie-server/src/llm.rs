//! LLM client for agent message processing
//!
//! TigerStyle: Explicit configuration, OpenAI-compatible API support.
//! Phase 7.8 REDO: Uses HttpClient trait for proper DST fault injection.

use crate::http::{HttpClient, HttpMethod, HttpRequest, ReqwestHttpClient};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::pin::Pin;
use std::sync::Arc;

/// LLM provider configuration
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// API base URL (OpenAI-compatible)
    pub base_url: String,
    /// API key
    pub api_key: String,
    /// Model to use
    pub model: String,
    /// Max tokens in response
    pub max_tokens: u32,
}

impl LlmConfig {
    /// Create config from environment variables
    ///
    /// Supports:
    /// - OPENAI_API_KEY + OPENAI_BASE_URL (default: api.openai.com)
    /// - ANTHROPIC_API_KEY (uses Anthropic's OpenAI-compatible endpoint)
    pub fn from_env() -> Option<Self> {
        // Try Anthropic first
        if let Ok(api_key) = env::var("ANTHROPIC_API_KEY") {
            return Some(Self {
                base_url: "https://api.anthropic.com/v1".to_string(),
                api_key,
                model: env::var("KELPIE_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
                max_tokens: 1024,
            });
        }

        // Try OpenAI
        if let Ok(api_key) = env::var("OPENAI_API_KEY") {
            return Some(Self {
                base_url: env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
                api_key,
                model: env::var("KELPIE_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
                max_tokens: 1024,
            });
        }

        None
    }

    /// Check if using Anthropic API
    pub fn is_anthropic(&self) -> bool {
        self.base_url.contains("anthropic.com")
    }
}

/// Chat message for LLM API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// OpenAI-compatible chat completion request
#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
}

/// OpenAI-compatible chat completion response
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

/// Anthropic messages API request
#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDefinition>,
}

/// Anthropic message (supports tool results)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicMessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

/// Anthropic messages API response
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseContent>,
    usage: AnthropicUsage,
    stop_reason: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponseContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
}

/// Tool definition for LLM
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl ToolDefinition {
    pub fn shell() -> Self {
        Self {
            name: "shell".to_string(),
            description: "Execute a shell command. Use this to run commands, check files, or perform system operations.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }
}

/// Tool call from LLM
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Response from LLM completion
#[derive(Debug)]
pub struct CompletionResponse {
    pub content: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: String,
}

/// Stream delta from LLM (Phase 7.8)
#[derive(Debug, Clone)]
pub enum StreamDelta {
    /// Text content delta
    ContentDelta { text: String },
    /// Tool call started
    ToolCallStart { id: String, name: String },
    /// Tool call input delta
    ToolCallDelta { delta: String },
    /// Stream completed
    Done { stop_reason: String },
}

/// LLM client (Phase 7.8 REDO: uses HttpClient trait for DST)
pub struct LlmClient {
    config: LlmConfig,
    http_client: Arc<dyn HttpClient>,
}

// Manual Clone implementation (Arc is Clone)
impl Clone for LlmClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            http_client: Arc::clone(&self.http_client),
        }
    }
}

impl LlmClient {
    /// Create a new LLM client with production HTTP client
    pub fn new(config: LlmConfig) -> Self {
        Self {
            config,
            http_client: Arc::new(ReqwestHttpClient::new()),
        }
    }

    /// Create LLM client with custom HTTP client (for DST)
    pub fn with_http_client(config: LlmConfig, http_client: Arc<dyn HttpClient>) -> Self {
        Self {
            config,
            http_client,
        }
    }

    /// Create from environment, returns None if no API key configured
    pub fn from_env() -> Option<Self> {
        LlmConfig::from_env().map(Self::new)
    }

    /// Complete a chat conversation (without tools)
    #[allow(dead_code)]
    pub async fn complete(&self, messages: Vec<ChatMessage>) -> Result<CompletionResponse, String> {
        self.complete_with_tools(messages, vec![]).await
    }

    /// Complete a chat conversation with tool support
    pub async fn complete_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<CompletionResponse, String> {
        if self.config.is_anthropic() {
            self.complete_anthropic(messages, tools).await
        } else {
            self.complete_openai(messages).await
        }
    }

    /// Continue conversation with tool result
    pub async fn continue_with_tool_result(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        assistant_content: Vec<AnthropicContentBlock>,
        tool_results: Vec<(String, String)>, // (tool_use_id, result)
    ) -> Result<CompletionResponse, String> {
        if !self.config.is_anthropic() {
            return Err("Tool use only supported with Anthropic API".to_string());
        }

        // Build messages with tool results
        let (system, mut anthropic_messages) = self.prepare_anthropic_messages(messages);

        // Add assistant message with tool use
        anthropic_messages.push(AnthropicMessage {
            role: "assistant".to_string(),
            content: AnthropicMessageContent::Blocks(assistant_content),
        });

        // Add tool results
        let tool_result_blocks: Vec<AnthropicContentBlock> = tool_results
            .into_iter()
            .map(|(id, result)| AnthropicContentBlock::ToolResult {
                tool_use_id: id,
                content: result,
            })
            .collect();

        anthropic_messages.push(AnthropicMessage {
            role: "user".to_string(),
            content: AnthropicMessageContent::Blocks(tool_result_blocks),
        });

        self.call_anthropic(anthropic_messages, system, tools).await
    }

    /// Stream a chat conversation with tool support (Phase 7.8)
    ///
    /// Returns stream of text deltas as they arrive from LLM.
    /// Supports both Anthropic and OpenAI APIs.
    pub async fn stream_complete_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<
        std::pin::Pin<Box<dyn futures::stream::Stream<Item = Result<StreamDelta, String>> + Send>>,
        String,
    > {
        if self.config.is_anthropic() {
            self.stream_anthropic(messages, tools).await
        } else {
            self.stream_openai(messages, tools).await
        }
    }

    fn prepare_anthropic_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system = None;
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                system = Some(msg.content);
            } else {
                anthropic_messages.push(AnthropicMessage {
                    role: msg.role,
                    content: AnthropicMessageContent::Text(msg.content),
                });
            }
        }

        (system, anthropic_messages)
    }

    async fn complete_openai(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<CompletionResponse, String> {
        let request_body = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: self.config.max_tokens,
        };

        // Build HTTP request
        let http_request = HttpRequest::new(
            HttpMethod::Post,
            format!("{}/chat/completions", self.config.base_url),
        )
        .header("Authorization", format!("Bearer {}", self.config.api_key))
        .json(&request_body)?;

        // Send HTTP request
        let response = self.http_client.send(http_request).await?;

        // Check status
        if !response.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(format!("API error {}: {}", response.status, body));
        }

        // Parse JSON response
        let completion: ChatCompletionResponse = response.json()?;

        let choice = completion
            .choices
            .into_iter()
            .next()
            .ok_or("No completion choices returned")?;

        let usage = completion.usage.unwrap_or(ApiUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        });

        Ok(CompletionResponse {
            content: choice.message.content,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            tool_calls: vec![],
            stop_reason: "end_turn".to_string(),
        })
    }

    async fn complete_anthropic(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<CompletionResponse, String> {
        let (system, anthropic_messages) = self.prepare_anthropic_messages(messages);
        self.call_anthropic(anthropic_messages, system, tools).await
    }

    async fn call_anthropic(
        &self,
        messages: Vec<AnthropicMessage>,
        system: Option<String>,
        tools: Vec<ToolDefinition>,
    ) -> Result<CompletionResponse, String> {
        let request_body = AnthropicRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens: self.config.max_tokens,
            system,
            tools,
        };

        // Build HTTP request
        let http_request = HttpRequest::new(
            HttpMethod::Post,
            format!("{}/messages", self.config.base_url),
        )
        .header("x-api-key", &self.config.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)?;

        // Send HTTP request
        let response = self.http_client.send(http_request).await?;

        // Check status
        if !response.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(format!("API error {}: {}", response.status, body));
        }

        // Parse JSON response
        let completion: AnthropicResponse = response.json()?;

        // Extract text content
        let content = completion
            .content
            .iter()
            .filter(|c| c.content_type == "text")
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("");

        // Extract tool calls
        let tool_calls = completion
            .content
            .iter()
            .filter(|c| c.content_type == "tool_use")
            .filter_map(|c| {
                Some(ToolCall {
                    id: c.id.clone()?,
                    name: c.name.clone()?,
                    input: c.input.clone()?,
                })
            })
            .collect();

        Ok(CompletionResponse {
            content,
            prompt_tokens: completion.usage.input_tokens,
            completion_tokens: completion.usage.output_tokens,
            tool_calls,
            stop_reason: completion.stop_reason,
        })
    }

    async fn stream_anthropic(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta, String>> + Send>>, String> {
        let (system, anthropic_messages) = self.prepare_anthropic_messages(messages);

        // Build request with streaming enabled
        let mut request_json = serde_json::json!({
            "model": self.config.model,
            "messages": anthropic_messages,
            "max_tokens": self.config.max_tokens,
            "stream": true,
        });

        if let Some(system_msg) = system {
            request_json["system"] = serde_json::Value::String(system_msg);
        }

        if !tools.is_empty() {
            request_json["tools"] = serde_json::to_value(&tools).map_err(|e| e.to_string())?;
        }

        // Build HTTP request for streaming
        let http_request = HttpRequest::new(
            HttpMethod::Post,
            format!("{}/messages", self.config.base_url),
        )
        .header("x-api-key", &self.config.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&request_json)?;

        // Send streaming HTTP request
        let byte_stream = self.http_client.send_streaming(http_request).await?;

        // Parse SSE events and convert to StreamDelta
        let stream = parse_sse_stream(byte_stream);

        Ok(Box::pin(stream))
    }

    /// Stream OpenAI API response (Issue #76)
    ///
    /// OpenAI SSE format differs from Anthropic:
    /// - Content: `{"choices":[{"delta":{"content":"..."}}]}`
    /// - Completion: `{"choices":[{"finish_reason":"stop"}]}` then `data: [DONE]`
    ///
    /// Note: Tool calling in streaming is not yet supported for OpenAI.
    /// Tools will be logged as a warning and ignored.
    async fn stream_openai(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamDelta, String>> + Send>>, String> {
        // TigerStyle: No silent failures - warn if tools are passed but not supported
        if !tools.is_empty() {
            tracing::warn!(
                tool_count = tools.len(),
                "OpenAI streaming does not support tools yet - {} tools will be ignored",
                tools.len()
            );
        }

        // Build request with streaming enabled
        let request_json = serde_json::json!({
            "model": self.config.model,
            "messages": messages,
            "max_tokens": self.config.max_tokens,
            "stream": true,
        });

        // Build HTTP request for streaming
        let http_request = HttpRequest::new(
            HttpMethod::Post,
            format!("{}/chat/completions", self.config.base_url),
        )
        .header("Authorization", format!("Bearer {}", self.config.api_key))
        .json(&request_json)?;

        // Send streaming HTTP request
        let byte_stream = self.http_client.send_streaming(http_request).await?;

        // Parse OpenAI SSE events and convert to StreamDelta
        let stream = parse_openai_sse_stream(byte_stream);

        Ok(Box::pin(stream))
    }
}

/// Parse Server-Sent Events stream from Anthropic API (Phase 7.8 REDO)
///
/// Converts SSE events to StreamDelta items.
/// Handles events: content_block_delta, message_stop
///
/// Updated to accept Result<Bytes, String> for HttpClient trait compatibility.
fn parse_sse_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, String>> + Send + 'static,
) -> impl Stream<Item = Result<StreamDelta, String>> + Send {
    use futures::stream;

    // Use scan to maintain buffer state across chunks
    byte_stream
        .scan(String::new(), |buffer, chunk_result| {
            let result = match chunk_result {
                Ok(chunk) => {
                    // Add chunk to buffer
                    if let Ok(text) = std::str::from_utf8(&chunk) {
                        buffer.push_str(text);

                        // Process complete lines (ending with \n)
                        let mut deltas = Vec::new();

                        // Find all complete lines
                        while let Some(newline_idx) = buffer.find('\n') {
                            let line = buffer[..newline_idx].trim().to_string();

                            // Remove processed line from buffer
                            buffer.drain(..=newline_idx);

                            if let Some(data) = line.strip_prefix("data: ") {
                                // Skip "data: "

                                // Parse JSON
                                if let Ok(event) = serde_json::from_str::<Value>(data) {
                                    // Handle content_block_delta events
                                    if let Some(event_type) =
                                        event.get("type").and_then(|v| v.as_str())
                                    {
                                        match event_type {
                                            "content_block_delta" => {
                                                if let Some(text) = event
                                                    .get("delta")
                                                    .and_then(|d| d.get("text"))
                                                    .and_then(|t| t.as_str())
                                                {
                                                    deltas.push(Ok(StreamDelta::ContentDelta {
                                                        text: text.to_string(),
                                                    }));
                                                }
                                            }
                                            "message_stop" => {
                                                deltas.push(Ok(StreamDelta::Done {
                                                    stop_reason: "end_turn".to_string(),
                                                }));
                                            }
                                            _ => {
                                                // Ignore other event types (message_start, content_block_start, etc.)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Some(deltas)
                    } else {
                        Some(vec![])
                    }
                }
                Err(e) => Some(vec![Err(format!("Stream error: {}", e))]),
            };

            futures::future::ready(result)
        })
        .flat_map(stream::iter)
}

/// Parse Server-Sent Events stream from OpenAI API (Issue #76)
///
/// Converts OpenAI SSE events to StreamDelta items.
/// OpenAI format: `{"choices":[{"index":0,"delta":{"content":"..."},"finish_reason":null}]}`
/// Error format: `{"error":{"message":"...","type":"..."}}`
/// Stream ends with: `data: [DONE]`
fn parse_openai_sse_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, String>> + Send + 'static,
) -> impl Stream<Item = Result<StreamDelta, String>> + Send {
    use futures::stream;

    // Use scan to maintain buffer state across chunks
    // State: (buffer, seen_done) - track if we've already emitted Done
    byte_stream
        .scan(
            (String::new(), false),
            |(buffer, seen_done), chunk_result| {
                let result = match chunk_result {
                    Ok(chunk) => {
                        // Add chunk to buffer
                        if let Ok(text) = std::str::from_utf8(&chunk) {
                            buffer.push_str(text);

                            // Process complete lines (ending with \n)
                            let mut deltas = Vec::new();

                            // Find all complete lines
                            while let Some(newline_idx) = buffer.find('\n') {
                                let line = buffer[..newline_idx].trim().to_string();

                                // Remove processed line from buffer
                                buffer.drain(..=newline_idx);

                                if let Some(data) = line.strip_prefix("data: ") {
                                    // Handle [DONE] marker (OpenAI specific)
                                    // Only emit Done if we haven't already from finish_reason
                                    if data == "[DONE]" {
                                        if !*seen_done {
                                            *seen_done = true;
                                            deltas.push(Ok(StreamDelta::Done {
                                                stop_reason: "stop".to_string(),
                                            }));
                                        }
                                        continue;
                                    }

                                    // Parse JSON
                                    if let Ok(event) = serde_json::from_str::<Value>(data) {
                                        // Check for error events first
                                        if let Some(error) = event.get("error") {
                                            let message = error
                                                .get("message")
                                                .and_then(|m| m.as_str())
                                                .unwrap_or("Unknown error");
                                            let error_type = error
                                                .get("type")
                                                .and_then(|t| t.as_str())
                                                .unwrap_or("api_error");
                                            deltas.push(Err(format!(
                                                "OpenAI API error ({}): {}",
                                                error_type, message
                                            )));
                                            continue;
                                        }

                                        // OpenAI format: choices[0].delta.content
                                        if let Some(choices) =
                                            event.get("choices").and_then(|c| c.as_array())
                                        {
                                            if let Some(choice) = choices.first() {
                                                // Check for content delta
                                                if let Some(content) = choice
                                                    .get("delta")
                                                    .and_then(|d| d.get("content"))
                                                    .and_then(|c| c.as_str())
                                                {
                                                    if !content.is_empty() {
                                                        deltas.push(Ok(
                                                            StreamDelta::ContentDelta {
                                                                text: content.to_string(),
                                                            },
                                                        ));
                                                    }
                                                }

                                                // Check for finish_reason (signals completion)
                                                // Emit Done with actual reason before [DONE] marker
                                                if let Some(finish_reason) = choice
                                                    .get("finish_reason")
                                                    .and_then(|f| f.as_str())
                                                {
                                                    if !*seen_done {
                                                        *seen_done = true;
                                                        deltas.push(Ok(StreamDelta::Done {
                                                            stop_reason: finish_reason.to_string(),
                                                        }));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Some(deltas)
                        } else {
                            Some(vec![])
                        }
                    }
                    Err(e) => Some(vec![Err(format!("Stream error: {}", e))]),
                };

                futures::future::ready(result)
            },
        )
        .flat_map(stream::iter)
}

// Re-export for use in messages.rs
pub use self::AnthropicContentBlock as ContentBlock;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn test_config_detection() {
        // This test just verifies the code compiles and runs
        let _ = LlmConfig::from_env();
    }

    #[test]
    fn test_is_anthropic() {
        let anthropic_config = LlmConfig {
            base_url: "https://api.anthropic.com/v1".to_string(),
            api_key: "test".to_string(),
            model: "claude-3".to_string(),
            max_tokens: 1024,
        };
        assert!(anthropic_config.is_anthropic());

        let openai_config = LlmConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "test".to_string(),
            model: "gpt-4".to_string(),
            max_tokens: 1024,
        };
        assert!(!openai_config.is_anthropic());
    }

    #[tokio::test]
    async fn test_parse_openai_sse_stream_content() {
        // Simulate OpenAI SSE chunks
        let chunks = vec![
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")),
            Ok(bytes::Bytes::from("data: [DONE]\n\n")),
        ];

        let stream = futures::stream::iter(chunks);
        let mut parsed: Vec<_> = parse_openai_sse_stream(stream).collect().await;

        // Should have: "Hello", " world", Done (from finish_reason, [DONE] is deduplicated)
        assert_eq!(parsed.len(), 3);

        // First chunk: "Hello"
        match parsed.remove(0) {
            Ok(StreamDelta::ContentDelta { text }) => assert_eq!(text, "Hello"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }

        // Second chunk: " world"
        match parsed.remove(0) {
            Ok(StreamDelta::ContentDelta { text }) => assert_eq!(text, " world"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }

        // Third chunk: Done from finish_reason (not [DONE] marker)
        match parsed.remove(0) {
            Ok(StreamDelta::Done { stop_reason }) => assert_eq!(stop_reason, "stop"),
            other => panic!("Expected Done with stop_reason='stop', got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_openai_sse_stream_handles_done_marker() {
        // Test that [DONE] is properly handled when no finish_reason was seen
        let chunks = vec![Ok(bytes::Bytes::from("data: [DONE]\n\n"))];

        let stream = futures::stream::iter(chunks);
        let parsed: Vec<_> = parse_openai_sse_stream(stream).collect().await;

        assert_eq!(parsed.len(), 1);
        match &parsed[0] {
            Ok(StreamDelta::Done { stop_reason }) => assert_eq!(stop_reason, "stop"),
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_openai_sse_stream_uses_actual_finish_reason() {
        // Test that non-"stop" finish reasons are captured correctly
        let chunks = vec![
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"Partial\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n")),
            Ok(bytes::Bytes::from("data: [DONE]\n\n")),
        ];

        let stream = futures::stream::iter(chunks);
        let parsed: Vec<_> = parse_openai_sse_stream(stream).collect().await;

        // Should have: "Partial", Done with "length" reason
        assert_eq!(parsed.len(), 2);

        match &parsed[1] {
            Ok(StreamDelta::Done { stop_reason }) => assert_eq!(stop_reason, "length"),
            other => panic!("Expected Done with stop_reason='length', got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_openai_sse_stream_handles_error_events() {
        // Test that OpenAI error events are properly converted to errors
        let chunks = vec![
            Ok(bytes::Bytes::from("data: {\"error\":{\"message\":\"Rate limit exceeded\",\"type\":\"rate_limit_error\"}}\n\n")),
        ];

        let stream = futures::stream::iter(chunks);
        let parsed: Vec<_> = parse_openai_sse_stream(stream).collect().await;

        assert_eq!(parsed.len(), 1);
        match &parsed[0] {
            Err(e) => {
                assert!(e.contains("Rate limit exceeded"));
                assert!(e.contains("rate_limit_error"));
            }
            other => panic!("Expected error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_parse_openai_sse_stream_ignores_empty_content() {
        // OpenAI sometimes sends empty delta content
        let chunks = vec![
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n")),
            Ok(bytes::Bytes::from("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")),
            Ok(bytes::Bytes::from("data: [DONE]\n\n")),
        ];

        let stream = futures::stream::iter(chunks);
        let parsed: Vec<_> = parse_openai_sse_stream(stream).collect().await;

        // Should only have "Hi" and Done (empty content ignored)
        assert_eq!(parsed.len(), 2);
        match &parsed[0] {
            Ok(StreamDelta::ContentDelta { text }) => assert_eq!(text, "Hi"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }
        match &parsed[1] {
            Ok(StreamDelta::Done { stop_reason }) => assert_eq!(stop_reason, "stop"),
            other => panic!("Expected Done, got {:?}", other),
        }
    }
}
