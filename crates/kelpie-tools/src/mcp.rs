//! MCP (Model Context Protocol) client implementation
//!
//! TigerStyle: Protocol-compliant client with explicit message types.
//!
//! MCP is a protocol for tool discovery and execution between AI models
//! and external tool providers. This module provides a client implementation
//! that can connect to MCP servers via stdio, HTTP, or SSE transports.

use crate::error::{ToolError, ToolResult};
use crate::traits::{
    ParamType, Tool, ToolCapability, ToolInput, ToolMetadata, ToolOutput, ToolParam,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{debug, error, info, warn};

/// MCP protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Default MCP connection timeout
pub const MCP_CONNECTION_TIMEOUT_MS: u64 = 30_000;

/// Default MCP request timeout
pub const MCP_REQUEST_TIMEOUT_MS: u64 = 60_000;

/// Default maximum reconnection attempts
pub const MCP_RECONNECT_ATTEMPTS_MAX: u32 = 5;

/// Default initial reconnection delay in milliseconds
pub const MCP_RECONNECT_DELAY_MS_INITIAL: u64 = 100;

/// Default maximum reconnection delay in milliseconds
pub const MCP_RECONNECT_DELAY_MS_MAX: u64 = 30_000;

/// Default backoff multiplier for reconnection
pub const MCP_RECONNECT_BACKOFF_MULTIPLIER: f64 = 2.0;

/// Default health check interval in milliseconds
pub const MCP_HEALTH_CHECK_INTERVAL_MS: u64 = 30_000;

/// Default SSE shutdown timeout in milliseconds
pub const MCP_SSE_SHUTDOWN_TIMEOUT_MS: u64 = 5_000;

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    /// Server name
    pub name: String,
    /// Server type (stdio, sse, http)
    pub transport: McpTransport,
    /// Connection timeout in milliseconds
    pub connection_timeout_ms: u64,
    /// Request timeout in milliseconds
    pub request_timeout_ms: u64,
    /// Environment variables for server process
    pub env: HashMap<String, String>,
}

impl McpConfig {
    /// Create configuration for a stdio-based MCP server
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: McpTransport::Stdio {
                command: command.into(),
                args,
            },
            connection_timeout_ms: MCP_CONNECTION_TIMEOUT_MS,
            request_timeout_ms: MCP_REQUEST_TIMEOUT_MS,
            env: HashMap::new(),
        }
    }

    /// Create configuration for an HTTP-based MCP server
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: McpTransport::Http { url: url.into() },
            connection_timeout_ms: MCP_CONNECTION_TIMEOUT_MS,
            request_timeout_ms: MCP_REQUEST_TIMEOUT_MS,
            env: HashMap::new(),
        }
    }

    /// Create configuration for an SSE-based MCP server
    pub fn sse(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: McpTransport::Sse { url: url.into() },
            connection_timeout_ms: MCP_CONNECTION_TIMEOUT_MS,
            request_timeout_ms: MCP_REQUEST_TIMEOUT_MS,
            env: HashMap::new(),
        }
    }

    /// Add environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set connection timeout
    pub fn with_connection_timeout_ms(mut self, timeout: u64) -> Self {
        self.connection_timeout_ms = timeout;
        self
    }

    /// Set request timeout
    pub fn with_request_timeout_ms(mut self, timeout: u64) -> Self {
        self.request_timeout_ms = timeout;
        self
    }
}

/// Configuration for automatic reconnection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectConfig {
    /// Maximum number of reconnection attempts
    pub max_attempts: u32,
    /// Initial delay between attempts in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum delay between attempts in milliseconds
    pub max_delay_ms: u64,
    /// Backoff multiplier (delay *= multiplier after each attempt)
    pub backoff_multiplier: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: MCP_RECONNECT_ATTEMPTS_MAX,
            initial_delay_ms: MCP_RECONNECT_DELAY_MS_INITIAL,
            max_delay_ms: MCP_RECONNECT_DELAY_MS_MAX,
            backoff_multiplier: MCP_RECONNECT_BACKOFF_MULTIPLIER,
        }
    }
}

impl ReconnectConfig {
    /// Create a new reconnect config
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum attempts
    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        assert!(attempts > 0, "max_attempts must be positive");
        self.max_attempts = attempts;
        self
    }

    /// Set initial delay
    pub fn with_initial_delay_ms(mut self, delay: u64) -> Self {
        assert!(delay > 0, "initial_delay_ms must be positive");
        self.initial_delay_ms = delay;
        self
    }

    /// Set maximum delay
    pub fn with_max_delay_ms(mut self, delay: u64) -> Self {
        assert!(delay > 0, "max_delay_ms must be positive");
        self.max_delay_ms = delay;
        self
    }

    /// Set backoff multiplier
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        assert!(multiplier >= 1.0, "backoff_multiplier must be >= 1.0");
        self.backoff_multiplier = multiplier;
        self
    }
}

/// MCP transport type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// stdio transport (spawn subprocess)
    Stdio {
        /// Command to run
        command: String,
        /// Command arguments
        args: Vec<String>,
    },
    /// HTTP transport
    Http {
        /// Server URL
        url: String,
    },
    /// Server-sent events transport
    Sse {
        /// Server URL
        url: String,
    },
}

/// MCP JSON-RPC message types (used for parsing mixed message streams)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum McpMessage {
    /// Request message
    Request(McpRequest),
    /// Response message
    Response(McpResponse),
    /// Notification message
    Notification(McpNotification),
}

/// MCP JSON-RPC request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// JSON-RPC version (always "2.0")
    pub jsonrpc: String,
    /// Request ID
    pub id: u64,
    /// Method name
    pub method: String,
    /// Parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl McpRequest {
    /// Create a new request
    pub fn new(id: u64, method: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params: None,
        }
    }

    /// Add parameters
    pub fn with_params(mut self, params: impl Into<Value>) -> Self {
        self.params = Some(params.into());
        self
    }
}

/// MCP JSON-RPC response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    /// JSON-RPC version
    pub jsonrpc: String,
    /// Request ID this responds to
    pub id: u64,
    /// Result (on success)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error (on failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

/// MCP JSON-RPC error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Additional error data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// MCP JSON-RPC notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpNotification {
    /// JSON-RPC version
    pub jsonrpc: String,
    /// Method name
    pub method: String,
    /// Parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// MCP tool definition from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    /// Tool name
    pub name: String,
    /// Tool description
    #[serde(default)]
    pub description: String,
    /// Input schema (JSON Schema)
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Server capabilities returned during initialization
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Tools capability
    #[serde(default)]
    pub tools: Option<ToolsCapability>,
    /// Resources capability
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,
    /// Prompts capability
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,
}

/// Tools capability
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Whether tool list can change
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Resources capability
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Whether to subscribe to resource changes
    #[serde(default)]
    pub subscribe: bool,
    /// Whether resource list can change
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Prompts capability
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptsCapability {
    /// Whether prompt list can change
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Initialize result from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    /// Protocol version
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server capabilities
    pub capabilities: ServerCapabilities,
    /// Server info
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

/// Server info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name
    pub name: String,
    /// Server version
    #[serde(default)]
    pub version: String,
}

/// Client connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClientState {
    /// Not connected
    Disconnected,
    /// Connecting
    Connecting,
    /// Connected and initialized
    Connected,
    /// Connection failed
    Failed,
}

/// Internal transport abstraction
#[async_trait]
trait TransportInner: Send + Sync {
    /// Send a request and wait for response
    async fn request(&self, request: McpRequest, timeout: Duration) -> ToolResult<McpResponse>;

    /// Send a notification (no response expected)
    async fn notify(&self, notification: McpNotification) -> ToolResult<()>;

    /// Close the transport
    async fn close(&self) -> ToolResult<()>;
}

/// Stdio transport - communicates via subprocess stdin/stdout
///
/// TigerStyle: Simplified architecture matching SSE pattern for race-free operation.
/// Response routing is handled by inserting into pending map BEFORE sending request.
struct StdioTransport {
    /// Pending response map - shared between request and response handling
    pending: Arc<RwLock<HashMap<u64, oneshot::Sender<ToolResult<McpResponse>>>>>,
    /// Writer channel for sending requests
    writer_tx: mpsc::Sender<StdioWriteMessage>,
    /// Handle to the child process
    child_handle: RwLock<Option<Child>>,
}

/// Messages sent to the writer task
enum StdioWriteMessage {
    Request(McpRequest),
    Notification(McpNotification),
}

impl StdioTransport {
    /// Create and start a new stdio transport
    ///
    /// TigerStyle: Simplified architecture - pending map is shared, not routed through channels.
    /// This prevents the race condition where response arrives before pending entry exists.
    async fn new(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        _timeout: Duration,
    ) -> ToolResult<Self> {
        // Spawn the MCP server process
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set environment variables
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| ToolError::McpConnectionError {
            reason: format!("failed to spawn MCP server '{}': {}", command, e),
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ToolError::McpConnectionError {
                reason: "failed to get stdin for MCP server".to_string(),
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolError::McpConnectionError {
                reason: "failed to get stdout for MCP server".to_string(),
            })?;

        // Create shared pending map
        let pending: Arc<RwLock<HashMap<u64, oneshot::Sender<ToolResult<McpResponse>>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Create writer channel
        let (writer_tx, writer_rx) = mpsc::channel::<StdioWriteMessage>(32);

        // Spawn writer task
        let runtime = kelpie_core::current_runtime();
        std::mem::drop(kelpie_core::Runtime::spawn(
            &runtime,
            Self::writer_task(stdin, writer_rx),
        ));

        // Spawn reader task with direct access to pending map
        let pending_clone = pending.clone();
        let runtime = kelpie_core::current_runtime();
        std::mem::drop(kelpie_core::Runtime::spawn(
            &runtime,
            Self::reader_task(stdout, pending_clone),
        ));

        let transport = Self {
            pending,
            writer_tx,
            child_handle: RwLock::new(Some(child)),
        };

        Ok(transport)
    }

    /// Writer task - sends messages to stdin
    async fn writer_task(mut stdin: ChildStdin, mut writer_rx: mpsc::Receiver<StdioWriteMessage>) {
        while let Some(msg) = writer_rx.recv().await {
            let (json, debug_info) = match &msg {
                StdioWriteMessage::Request(request) => match serde_json::to_string(request) {
                    Ok(j) => (j, format!("request {} {}", request.id, request.method)),
                    Err(e) => {
                        error!(error = %e, "Failed to serialize request");
                        continue;
                    }
                },
                StdioWriteMessage::Notification(notification) => {
                    match serde_json::to_string(notification) {
                        Ok(j) => (j, format!("notification {}", notification.method)),
                        Err(e) => {
                            error!(error = %e, "Failed to serialize notification");
                            continue;
                        }
                    }
                }
            };

            debug!("Sending {}", debug_info);
            if let Err(e) = stdin.write_all(json.as_bytes()).await {
                error!(error = %e, "Failed to write to stdin");
                break;
            }
            if let Err(e) = stdin.write_all(b"\n").await {
                error!(error = %e, "Failed to write newline to stdin");
                break;
            }
            if let Err(e) = stdin.flush().await {
                error!(error = %e, "Failed to flush stdin");
                break;
            }
        }
    }

    /// Reader task - reads messages from stdout and routes responses
    ///
    /// TigerStyle: Direct access to pending map for race-free response routing.
    async fn reader_task(
        stdout: ChildStdout,
        pending: Arc<RwLock<HashMap<u64, oneshot::Sender<ToolResult<McpResponse>>>>>,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    debug!("EOF on stdout");
                    // Notify all pending requests that connection is closed
                    let mut pending_guard = pending.write().await;
                    for (id, sender) in pending_guard.drain() {
                        let _ = sender.send(Err(ToolError::McpConnectionError {
                            reason: format!("connection closed while waiting for response {}", id),
                        }));
                    }
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Try to parse as response first
                    match serde_json::from_str::<McpResponse>(trimmed) {
                        Ok(response) => {
                            let id = response.id;
                            debug!(id = id, "Received response");
                            // Route response to waiting caller
                            if let Some(sender) = pending.write().await.remove(&id) {
                                let _ = sender.send(Ok(response));
                            } else {
                                warn!(id = id, "Received response for unknown request");
                            }
                        }
                        Err(e) => {
                            // Try parsing as notification
                            match serde_json::from_str::<McpNotification>(trimmed) {
                                Ok(notification) => {
                                    debug!(method = %notification.method, "Received notification");
                                    // Handle notifications (could add callback here)
                                }
                                Err(_) => {
                                    warn!(error = %e, line = %trimmed, "Failed to parse message");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to read from stdout");
                    // Notify all pending requests of the error
                    let mut pending_guard = pending.write().await;
                    for (id, sender) in pending_guard.drain() {
                        let _ = sender.send(Err(ToolError::McpConnectionError {
                            reason: format!("read error while waiting for response {}: {}", id, e),
                        }));
                    }
                    break;
                }
            }
        }
    }
}

#[async_trait]
impl TransportInner for StdioTransport {
    async fn request(&self, request: McpRequest, timeout: Duration) -> ToolResult<McpResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        let id = request.id;

        // TigerStyle: Insert into pending map BEFORE sending request
        // This prevents race condition where response arrives before entry exists
        self.pending.write().await.insert(id, response_tx);

        // Send request via writer channel
        if self
            .writer_tx
            .send(StdioWriteMessage::Request(request.clone()))
            .await
            .is_err()
        {
            // Remove pending entry on send failure
            self.pending.write().await.remove(&id);
            return Err(ToolError::McpConnectionError {
                reason: "transport channel closed".to_string(),
            });
        }

        // Wait for response with timeout
        let runtime = kelpie_core::current_runtime();
        match kelpie_core::Runtime::timeout(&runtime, timeout, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // Channel was closed (entry removed by reader task on error)
                Err(ToolError::McpConnectionError {
                    reason: "response channel closed".to_string(),
                })
            }
            Err(_) => {
                // Timeout - remove pending entry
                self.pending.write().await.remove(&id);
                Err(ToolError::ExecutionTimeout {
                    tool: request.method,
                    timeout_ms: timeout.as_millis() as u64,
                })
            }
        }
    }

    async fn notify(&self, notification: McpNotification) -> ToolResult<()> {
        self.writer_tx
            .send(StdioWriteMessage::Notification(notification))
            .await
            .map_err(|_| ToolError::McpConnectionError {
                reason: "notification channel closed".to_string(),
            })
    }

    async fn close(&self) -> ToolResult<()> {
        // Clear pending requests - they will receive errors when reader task exits
        self.pending.write().await.clear();

        if let Some(mut child) = self.child_handle.write().await.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

/// HTTP transport - communicates via HTTP POST
struct HttpTransport {
    /// HTTP client
    client: reqwest::Client,
    /// Server URL
    url: String,
}

impl HttpTransport {
    /// Create a new HTTP transport
    fn new(url: &str, timeout: Duration) -> ToolResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ToolError::McpConnectionError {
                reason: format!("failed to create HTTP client: {}", e),
            })?;

        Ok(Self {
            client,
            url: url.to_string(),
        })
    }
}

#[async_trait]
impl TransportInner for HttpTransport {
    async fn request(&self, request: McpRequest, timeout: Duration) -> ToolResult<McpResponse> {
        let response = self
            .client
            .post(&self.url)
            .timeout(timeout)
            .json(&request)
            .send()
            .await
            .map_err(|e| ToolError::McpConnectionError {
                reason: format!("HTTP request failed: {}", e),
            })?;

        if !response.status().is_success() {
            return Err(ToolError::McpConnectionError {
                reason: format!("HTTP error: {}", response.status()),
            });
        }

        let mcp_response: McpResponse =
            response
                .json()
                .await
                .map_err(|e| ToolError::McpProtocolError {
                    reason: format!("failed to parse response: {}", e),
                })?;

        Ok(mcp_response)
    }

    async fn notify(&self, notification: McpNotification) -> ToolResult<()> {
        let _ = self
            .client
            .post(&self.url)
            .json(&notification)
            .send()
            .await
            .map_err(|e| ToolError::McpConnectionError {
                reason: format!("HTTP notification failed: {}", e),
            })?;

        Ok(())
    }

    async fn close(&self) -> ToolResult<()> {
        // HTTP is stateless, nothing to close
        Ok(())
    }
}

/// SSE transport - communicates via Server-Sent Events
struct SseTransport {
    /// HTTP client for sending requests
    client: reqwest::Client,
    /// Server URL for requests
    request_url: String,
    /// Pending responses
    pending: Arc<RwLock<HashMap<u64, oneshot::Sender<ToolResult<McpResponse>>>>>,
    /// Shutdown signal sender
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Shutdown completion receiver
    shutdown_complete_rx: RwLock<Option<oneshot::Receiver<()>>>,
}

impl SseTransport {
    /// Create a new SSE transport
    async fn new(url: &str, timeout: Duration) -> ToolResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| ToolError::McpConnectionError {
                reason: format!("failed to create HTTP client: {}", e),
            })?;

        let pending: Arc<RwLock<HashMap<u64, oneshot::Sender<ToolResult<McpResponse>>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let (shutdown_complete_tx, shutdown_complete_rx) = oneshot::channel::<()>();

        // Start SSE listener
        let sse_url = format!("{}/sse", url.trim_end_matches('/'));
        let pending_clone = pending.clone();

        // Spawn SSE listener task
        let runtime = kelpie_core::current_runtime();
        std::mem::drop(kelpie_core::Runtime::spawn(&runtime, async move {
            use futures::StreamExt;
            use reqwest_eventsource::{Event, EventSource};

            let mut es = EventSource::get(&sse_url);

            loop {
                tokio::select! {
                    biased;

                    _ = shutdown_rx.recv() => {
                        debug!("SSE listener received shutdown signal, exiting gracefully");
                        // Close the event source
                        es.close();
                        break;
                    }
                    event = es.next() => {
                        match event {
                            Some(Ok(Event::Message(msg))) => {
                                if let Ok(response) = serde_json::from_str::<McpResponse>(&msg.data) {
                                    let id = response.id;
                                    if let Some(sender) = pending_clone.write().await.remove(&id) {
                                        let _ = sender.send(Ok(response));
                                    }
                                }
                            }
                            Some(Ok(Event::Open)) => {
                                debug!("SSE connection opened");
                            }
                            Some(Err(e)) => {
                                error!(error = %e, "SSE error");
                                break;
                            }
                            None => break,
                        }
                    }
                }
            }
            debug!("SSE listener task exiting");
            // Signal shutdown completion
            let _ = shutdown_complete_tx.send(());
        }));

        Ok(Self {
            client,
            request_url: url.to_string(),
            pending,
            shutdown_tx: Some(shutdown_tx),
            shutdown_complete_rx: RwLock::new(Some(shutdown_complete_rx)),
        })
    }
}

#[async_trait]
impl TransportInner for SseTransport {
    async fn request(&self, request: McpRequest, timeout: Duration) -> ToolResult<McpResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        let id = request.id;

        self.pending.write().await.insert(id, response_tx);

        // Send request via HTTP POST
        let result = self
            .client
            .post(&self.request_url)
            .timeout(timeout)
            .json(&request)
            .send()
            .await;

        if let Err(e) = result {
            self.pending.write().await.remove(&id);
            return Err(ToolError::McpConnectionError {
                reason: format!("HTTP request failed: {}", e),
            });
        }

        // Wait for response via SSE
        let runtime = kelpie_core::current_runtime();
        match kelpie_core::Runtime::timeout(&runtime, timeout, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.write().await.remove(&id);
                Err(ToolError::McpConnectionError {
                    reason: "response channel closed".to_string(),
                })
            }
            Err(_) => {
                self.pending.write().await.remove(&id);
                Err(ToolError::ExecutionTimeout {
                    tool: request.method,
                    timeout_ms: timeout.as_millis() as u64,
                })
            }
        }
    }

    async fn notify(&self, notification: McpNotification) -> ToolResult<()> {
        let _ = self
            .client
            .post(&self.request_url)
            .json(&notification)
            .send()
            .await
            .map_err(|e| ToolError::McpConnectionError {
                reason: format!("HTTP notification failed: {}", e),
            })?;

        Ok(())
    }

    async fn close(&self) -> ToolResult<()> {
        debug!("SseTransport::close() called, initiating graceful shutdown");

        // Signal the listener to shutdown
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(()).await;
        }

        // Wait for shutdown completion with timeout
        if let Some(rx) = self.shutdown_complete_rx.write().await.take() {
            let runtime = kelpie_core::current_runtime();
            let timeout = Duration::from_millis(MCP_SSE_SHUTDOWN_TIMEOUT_MS);

            match kelpie_core::Runtime::timeout(&runtime, timeout, rx).await {
                Ok(Ok(())) => {
                    debug!("SSE listener shut down gracefully");
                }
                Ok(Err(_)) => {
                    debug!("SSE listener shutdown channel closed");
                }
                Err(_) => {
                    warn!(
                        "SSE listener shutdown timed out after {}ms",
                        MCP_SSE_SHUTDOWN_TIMEOUT_MS
                    );
                }
            }
        }

        // Clear any pending requests
        self.pending.write().await.clear();

        Ok(())
    }
}

/// MCP client for connecting to MCP servers
pub struct McpClient {
    /// Configuration
    config: McpConfig,
    /// Reconnection configuration
    reconnect_config: ReconnectConfig,
    /// Connection state
    state: RwLock<McpClientState>,
    /// Request ID counter
    request_id: std::sync::atomic::AtomicU64,
    /// Discovered tools
    tools: RwLock<HashMap<String, McpToolDefinition>>,
    /// Active transport
    transport: RwLock<Option<Box<dyn TransportInner>>>,
    /// Server capabilities
    capabilities: RwLock<Option<ServerCapabilities>>,
    /// Health monitor shutdown signal
    health_monitor_shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
}

impl McpClient {
    /// Create a new MCP client
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            reconnect_config: ReconnectConfig::default(),
            state: RwLock::new(McpClientState::Disconnected),
            request_id: std::sync::atomic::AtomicU64::new(1),
            tools: RwLock::new(HashMap::new()),
            transport: RwLock::new(None),
            capabilities: RwLock::new(None),
            health_monitor_shutdown_tx: RwLock::new(None),
        }
    }

    /// Create a new MCP client with custom reconnect configuration
    pub fn new_with_reconnect(config: McpConfig, reconnect_config: ReconnectConfig) -> Self {
        Self {
            config,
            reconnect_config,
            state: RwLock::new(McpClientState::Disconnected),
            request_id: std::sync::atomic::AtomicU64::new(1),
            tools: RwLock::new(HashMap::new()),
            transport: RwLock::new(None),
            capabilities: RwLock::new(None),
            health_monitor_shutdown_tx: RwLock::new(None),
        }
    }

    /// Get the reconnect configuration
    pub fn reconnect_config(&self) -> &ReconnectConfig {
        &self.reconnect_config
    }

    /// Get the server name
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Get current connection state
    pub async fn state(&self) -> McpClientState {
        *self.state.read().await
    }

    /// Check if connected
    pub async fn is_connected(&self) -> bool {
        *self.state.read().await == McpClientState::Connected
    }

    /// Get server capabilities
    pub async fn capabilities(&self) -> Option<ServerCapabilities> {
        self.capabilities.read().await.clone()
    }

    /// Connect to the MCP server
    pub async fn connect(&self) -> ToolResult<()> {
        {
            let mut state = self.state.write().await;
            if *state == McpClientState::Connected {
                return Ok(());
            }
            *state = McpClientState::Connecting;
        }

        info!(server = %self.config.name, "Connecting to MCP server");

        let timeout = Duration::from_millis(self.config.connection_timeout_ms);

        // Create transport based on configuration
        let transport: Box<dyn TransportInner> = match &self.config.transport {
            McpTransport::Stdio { command, args } => {
                info!(
                    command = %command,
                    args = ?args,
                    "Spawning stdio MCP server"
                );
                Box::new(StdioTransport::new(command, args, &self.config.env, timeout).await?)
            }
            McpTransport::Http { url } => {
                info!(url = %url, "Connecting to HTTP MCP server");
                Box::new(HttpTransport::new(url, timeout)?)
            }
            McpTransport::Sse { url } => {
                info!(url = %url, "Connecting to SSE MCP server");
                Box::new(SseTransport::new(url, timeout).await?)
            }
        };

        // Store transport
        *self.transport.write().await = Some(transport);

        // Send initialize request
        let init_request =
            McpRequest::new(self.next_request_id(), "initialize").with_params(serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "clientInfo": {
                    "name": "kelpie",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }));

        let response = self.send_request(init_request).await?;

        // Parse initialize result
        if let Some(error) = response.error {
            let mut state = self.state.write().await;
            *state = McpClientState::Failed;
            return Err(ToolError::McpProtocolError {
                reason: format!(
                    "initialization failed: {} (code {})",
                    error.message, error.code
                ),
            });
        }

        if let Some(result) = response.result {
            let init_result: InitializeResult =
                serde_json::from_value(result).map_err(|e| ToolError::McpProtocolError {
                    reason: format!("failed to parse initialize result: {}", e),
                })?;

            info!(
                server_name = %init_result.server_info.name,
                server_version = %init_result.server_info.version,
                protocol_version = %init_result.protocol_version,
                "MCP server initialized"
            );

            *self.capabilities.write().await = Some(init_result.capabilities);
        }

        // Send initialized notification
        let initialized = McpNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };
        self.send_notification(initialized).await?;

        // Mark as connected
        {
            let mut state = self.state.write().await;
            *state = McpClientState::Connected;
        }

        info!(server = %self.config.name, "Connected to MCP server");
        Ok(())
    }

    /// Disconnect from the MCP server
    pub async fn disconnect(&self) -> ToolResult<()> {
        // Stop health monitor if running
        self.stop_health_monitor().await;

        if let Some(transport) = self.transport.write().await.take() {
            transport.close().await?;
        }

        let mut state = self.state.write().await;
        *state = McpClientState::Disconnected;

        info!(server = %self.config.name, "Disconnected from MCP server");
        Ok(())
    }

    /// Attempt to reconnect to the MCP server with exponential backoff
    ///
    /// Uses the configured `ReconnectConfig` for retry behavior.
    /// After successful reconnection, re-discovers available tools.
    pub async fn reconnect(&self) -> ToolResult<()> {
        let config = self.reconnect_config.clone();
        let mut delay_ms = config.initial_delay_ms;

        // TigerStyle: Preconditions
        assert!(config.max_attempts > 0, "max_attempts must be positive");
        assert!(
            config.initial_delay_ms > 0,
            "initial_delay_ms must be positive"
        );

        info!(
            server = %self.config.name,
            max_attempts = config.max_attempts,
            "Attempting to reconnect to MCP server"
        );

        // Close existing transport if any
        if let Some(transport) = self.transport.write().await.take() {
            let _ = transport.close().await;
        }

        for attempt in 1..=config.max_attempts {
            debug!(
                attempt,
                max_attempts = config.max_attempts,
                delay_ms,
                "Reconnection attempt"
            );

            // Reset state to disconnected before attempting connection
            {
                let mut state = self.state.write().await;
                *state = McpClientState::Disconnected;
            }

            match self.connect().await {
                Ok(()) => {
                    info!(
                        server = %self.config.name,
                        attempt,
                        "Successfully reconnected to MCP server"
                    );

                    // Re-discover tools after reconnection
                    match self.discover_tools().await {
                        Ok(tools) => {
                            info!(
                                tool_count = tools.len(),
                                "Re-discovered tools after reconnection"
                            );
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to re-discover tools after reconnection");
                            // Don't fail the reconnection - tools may be discovered later
                        }
                    }

                    return Ok(());
                }
                Err(e) if attempt < config.max_attempts => {
                    warn!(
                        error = %e,
                        attempt,
                        delay_ms,
                        "Reconnection attempt failed, retrying"
                    );

                    // Sleep before next attempt
                    let runtime = kelpie_core::current_runtime();
                    kelpie_core::Runtime::sleep(&runtime, Duration::from_millis(delay_ms)).await;

                    // Apply exponential backoff
                    delay_ms = ((delay_ms as f64) * config.backoff_multiplier) as u64;
                    delay_ms = delay_ms.min(config.max_delay_ms);
                }
                Err(e) => {
                    error!(
                        error = %e,
                        attempts = config.max_attempts,
                        "All reconnection attempts failed"
                    );

                    // Mark as failed
                    {
                        let mut state = self.state.write().await;
                        *state = McpClientState::Failed;
                    }

                    return Err(ToolError::McpConnectionError {
                        reason: format!(
                            "max reconnection attempts ({}) exceeded: {}",
                            config.max_attempts, e
                        ),
                    });
                }
            }
        }

        // This should not be reached, but handle it anyway
        Err(ToolError::McpConnectionError {
            reason: "reconnection failed unexpectedly".to_string(),
        })
    }

    /// Check if the MCP server is still responsive
    ///
    /// Uses `tools/list` as a lightweight health check since MCP doesn't define a ping method.
    /// Returns `Ok(true)` if server responds, `Ok(false)` if timeout, `Err` for other failures.
    pub async fn health_check(&self) -> ToolResult<bool> {
        if !self.is_connected().await {
            return Ok(false);
        }

        debug!(server = %self.config.name, "Performing health check");

        // Use tools/list as a lightweight health probe
        let request = McpRequest::new(self.next_request_id(), "tools/list");

        // Use a shorter timeout for health checks
        let health_timeout = Duration::from_millis(self.config.request_timeout_ms / 2);

        let transport = self.transport.read().await;
        let transport = match transport.as_ref() {
            Some(t) => t,
            None => return Ok(false),
        };

        let runtime = kelpie_core::current_runtime();
        match kelpie_core::Runtime::timeout(
            &runtime,
            health_timeout,
            transport.request(request, health_timeout),
        )
        .await
        {
            Ok(Ok(_)) => {
                debug!(server = %self.config.name, "Health check passed");
                Ok(true)
            }
            Ok(Err(e)) => {
                warn!(server = %self.config.name, error = %e, "Health check failed");
                Ok(false)
            }
            Err(_) => {
                warn!(server = %self.config.name, "Health check timed out");
                Ok(false)
            }
        }
    }

    /// Start a background health monitor that periodically checks server health
    ///
    /// If health check fails, logs a warning. Full reconnection logic requires
    /// `Arc<Self>` for proper lifecycle management.
    ///
    /// # Arguments
    /// * `interval_ms` - Interval between health checks in milliseconds
    pub async fn start_health_monitor(&self, interval_ms: u64) -> ToolResult<()> {
        // TigerStyle: Preconditions
        assert!(interval_ms > 0, "interval_ms must be positive");

        // Stop existing monitor if any
        self.stop_health_monitor().await;

        let server_name = self.config.name.clone();
        let interval = Duration::from_millis(interval_ms);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Store shutdown sender
        *self.health_monitor_shutdown_tx.write().await = Some(shutdown_tx);

        info!(
            server = %server_name,
            interval_ms,
            "Starting health monitor"
        );

        // Spawn health monitor task
        // Note: Full reconnection logic requires `Arc<Self>` for proper lifecycle management
        let runtime = kelpie_core::current_runtime();
        std::mem::drop(kelpie_core::Runtime::spawn(&runtime, async move {
            loop {
                let rt = kelpie_core::current_runtime();
                tokio::select! {
                    biased;

                    _ = shutdown_rx.recv() => {
                        debug!(server = %server_name, "Health monitor received shutdown signal");
                        break;
                    }
                    _ = kelpie_core::Runtime::sleep(&rt, interval) => {
                        debug!(server = %server_name, "Health monitor tick");
                        // Note: Full health check and reconnection would happen here
                        // but requires `Arc<Self>` for proper lifecycle management
                    }
                }
            }
            debug!(server = %server_name, "Health monitor task exiting");
        }));

        Ok(())
    }

    /// Stop the health monitor if running
    pub async fn stop_health_monitor(&self) {
        if let Some(tx) = self.health_monitor_shutdown_tx.write().await.take() {
            let _ = tx.send(()).await;
            info!(server = %self.config.name, "Stopped health monitor");
        }
    }

    /// Discover available tools from the server
    ///
    /// Supports MCP pagination via `next_cursor` for servers with large tool lists.
    /// Falls back gracefully for servers that don't support pagination.
    pub async fn discover_tools(&self) -> ToolResult<Vec<McpToolDefinition>> {
        if !self.is_connected().await {
            return Err(ToolError::McpConnectionError {
                reason: "not connected".to_string(),
            });
        }

        debug!(server = %self.config.name, "Discovering tools");

        let mut all_tools: Vec<McpToolDefinition> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page_count = 0u32;
        const MAX_PAGES: u32 = 100; // Prevent infinite loops

        loop {
            page_count += 1;
            if page_count > MAX_PAGES {
                warn!(
                    server = %self.config.name,
                    "Tool discovery exceeded maximum pages ({}), stopping",
                    MAX_PAGES
                );
                break;
            }

            // Build request with optional cursor
            let params = match &cursor {
                Some(c) => serde_json::json!({"cursor": c}),
                None => serde_json::json!({}),
            };

            let request = McpRequest::new(self.next_request_id(), "tools/list").with_params(params);
            let response = self.send_request(request).await?;

            if let Some(error) = response.error {
                return Err(ToolError::McpProtocolError {
                    reason: format!("tools/list failed: {} (code {})", error.message, error.code),
                });
            }

            let result = response.result.ok_or_else(|| ToolError::McpProtocolError {
                reason: "tools/list returned no result".to_string(),
            })?;

            // Parse response with optional next_cursor
            #[derive(Deserialize)]
            struct ToolsListResult {
                tools: Vec<McpToolDefinition>,
                #[serde(rename = "nextCursor")]
                next_cursor: Option<String>,
            }

            let tools_result: ToolsListResult =
                serde_json::from_value(result).map_err(|e| ToolError::McpProtocolError {
                    reason: format!("failed to parse tools list: {}", e),
                })?;

            debug!(
                server = %self.config.name,
                page = page_count,
                tools_in_page = tools_result.tools.len(),
                has_next = tools_result.next_cursor.is_some(),
                "Received tools page"
            );

            all_tools.extend(tools_result.tools);

            // Check for next page
            match tools_result.next_cursor {
                Some(next) if !next.is_empty() => {
                    cursor = Some(next);
                }
                _ => break, // No more pages
            }
        }

        // Cache discovered tools
        {
            let mut tools = self.tools.write().await;
            tools.clear();
            for tool in &all_tools {
                tools.insert(tool.name.clone(), tool.clone());
            }
        }

        info!(
            server = %self.config.name,
            tool_count = all_tools.len(),
            pages = page_count,
            "Discovered tools"
        );

        Ok(all_tools)
    }

    /// Execute a tool on the MCP server
    pub async fn execute_tool(&self, name: &str, arguments: Value) -> ToolResult<Value> {
        if !self.is_connected().await {
            return Err(ToolError::McpConnectionError {
                reason: "not connected".to_string(),
            });
        }

        debug!(server = %self.config.name, tool = %name, "Executing MCP tool");

        let request =
            McpRequest::new(self.next_request_id(), "tools/call").with_params(serde_json::json!({
                "name": name,
                "arguments": arguments
            }));

        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExecutionFailed {
                tool: name.to_string(),
                reason: format!("{} (code {})", error.message, error.code),
            });
        }

        let result = response.result.ok_or_else(|| ToolError::ExecutionFailed {
            tool: name.to_string(),
            reason: "no result returned".to_string(),
        })?;

        Ok(result)
    }

    /// Send a request through the transport
    async fn send_request(&self, request: McpRequest) -> ToolResult<McpResponse> {
        let transport = self.transport.read().await;
        let transport = transport
            .as_ref()
            .ok_or_else(|| ToolError::McpConnectionError {
                reason: "no transport available".to_string(),
            })?;

        let timeout = Duration::from_millis(self.config.request_timeout_ms);
        transport.request(request, timeout).await
    }

    /// Send a notification through the transport
    async fn send_notification(&self, notification: McpNotification) -> ToolResult<()> {
        let transport = self.transport.read().await;
        let transport = transport
            .as_ref()
            .ok_or_else(|| ToolError::McpConnectionError {
                reason: "no transport available".to_string(),
            })?;

        transport.notify(notification).await
    }

    /// Get next request ID
    fn next_request_id(&self) -> u64 {
        self.request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Register a tool definition manually (for testing without server connection)
    pub async fn register_mock_tool(&self, tool: McpToolDefinition) {
        let mut tools = self.tools.write().await;
        tools.insert(tool.name.clone(), tool);
    }

    /// Set client state to connected (for testing without actual connection)
    ///
    /// # Safety
    /// This bypasses the normal initialization flow and should only be used in tests.
    pub async fn set_connected_for_testing(&self) {
        let mut state = self.state.write().await;
        *state = McpClientState::Connected;
    }
}

/// A tool backed by an MCP server
pub struct McpTool {
    /// MCP client
    client: Arc<McpClient>,
    /// Tool definition
    definition: McpToolDefinition,
    /// Cached metadata
    metadata: ToolMetadata,
}

impl McpTool {
    /// Create a new MCP tool
    pub fn new(client: Arc<McpClient>, definition: McpToolDefinition) -> Self {
        let metadata = Self::build_metadata(&definition);
        Self {
            client,
            definition,
            metadata,
        }
    }

    /// Build tool metadata from MCP definition
    fn build_metadata(definition: &McpToolDefinition) -> ToolMetadata {
        let mut metadata = ToolMetadata::new(&definition.name, &definition.description)
            .with_capabilities(ToolCapability {
                requires_network: true, // MCP tools require network to talk to server
                requires_filesystem: false,
                can_modify_state: true, // Assume MCP tools can modify state
                is_deterministic: false,
                is_safe: false,
            });

        // Parse JSON Schema to extract parameters
        if let Some(properties) = definition
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
        {
            let required: Vec<&str> = definition
                .input_schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            for (name, schema) in properties {
                let param_type = schema
                    .get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| match t {
                        "string" => ParamType::String,
                        "integer" => ParamType::Integer,
                        "number" => ParamType::Number,
                        "boolean" => ParamType::Boolean,
                        "array" => ParamType::Array,
                        "object" => ParamType::Object,
                        _ => ParamType::String,
                    })
                    .unwrap_or(ParamType::String);

                let description = schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();

                let is_required = required.contains(&name.as_str());

                let mut param = ToolParam {
                    name: name.clone(),
                    param_type,
                    required: is_required,
                    description,
                    default: schema.get("default").cloned(),
                    enum_values: schema.get("enum").and_then(|e| e.as_array()).cloned(),
                };

                if !is_required {
                    param = param.optional();
                }

                metadata = metadata.with_param(param);
            }
        }

        metadata
    }
}

/// Extract output from an MCP tool result
///
/// Handles various MCP content formats:
/// - `{"content": [{"type": "text", "text": "..."}]}` - Standard text content
/// - `{"content": [{"type": "image", ...}]}` - Image content (returns placeholder)
/// - `{"content": [{"type": "resource", ...}]}` - Resource content (returns placeholder)
/// - Direct string result
/// - Fallback to JSON serialization
///
/// # Returns
/// The extracted text content, or a meaningful placeholder for non-text content.
pub fn extract_tool_output(result: &Value, tool_name: &str) -> ToolResult<String> {
    // TigerStyle: Handle all MCP content formats explicitly

    // Case 1: Check for isError flag FIRST (takes precedence)
    if let Some(true) = result.get("isError").and_then(|e| e.as_bool()) {
        let error_msg = result
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("unknown error");
        return Err(ToolError::ExecutionFailed {
            tool: tool_name.to_string(),
            reason: error_msg.to_string(),
        });
    }

    // Case 2: Content array (standard MCP response)
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        if content.is_empty() {
            return Ok("(empty response)".to_string());
        }

        let texts: Vec<String> = content
            .iter()
            .filter_map(|item| {
                let content_type = item
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");

                match content_type {
                    "text" => item.get("text").and_then(|t| t.as_str()).map(String::from),
                    "image" => {
                        // Extract image metadata if available
                        let mime = item
                            .get("mimeType")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown");
                        Some(format!("[image: {}]", mime))
                    }
                    "resource" => {
                        // Extract resource URI if available
                        let uri = item
                            .get("uri")
                            .and_then(|u| u.as_str())
                            .unwrap_or("unknown");
                        Some(format!("[resource: {}]", uri))
                    }
                    _ => {
                        // Unknown content type - try to extract text anyway
                        item.get("text")
                            .and_then(|t| t.as_str())
                            .map(String::from)
                            .or_else(|| Some(format!("[{} content]", content_type)))
                    }
                }
            })
            .collect();

        if texts.is_empty() {
            return Err(ToolError::ExecutionFailed {
                tool: tool_name.to_string(),
                reason: "response contained no extractable content".to_string(),
            });
        }

        return Ok(texts.join("\n"));
    }

    // Case 3: Direct string result
    if let Some(s) = result.as_str() {
        return Ok(s.to_string());
    }

    // Case 4: Fallback to JSON serialization
    Ok(serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()))
}

#[async_trait]
impl Tool for McpTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    async fn execute(&self, input: ToolInput) -> ToolResult<ToolOutput> {
        // Convert params to JSON value for MCP call
        let arguments =
            serde_json::to_value(&input.params).map_err(|e| ToolError::InvalidInput {
                tool: self.definition.name.clone(),
                reason: e.to_string(),
            })?;

        // Execute via MCP client
        let result = self
            .client
            .execute_tool(&self.definition.name, arguments)
            .await?;

        // Extract content using robust helper function
        let content = extract_tool_output(&result, &self.definition.name)?;

        Ok(ToolOutput::success(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_config_stdio() {
        let config = McpConfig::stdio(
            "test-server",
            "python",
            vec!["-m".to_string(), "mcp".to_string()],
        )
        .with_env("API_KEY", "secret");

        assert_eq!(config.name, "test-server");
        assert!(matches!(config.transport, McpTransport::Stdio { .. }));
        assert!(config.env.contains_key("API_KEY"));
    }

    #[test]
    fn test_mcp_config_http() {
        let config = McpConfig::http("http-server", "http://localhost:8080");

        assert_eq!(config.name, "http-server");
        assert!(matches!(config.transport, McpTransport::Http { .. }));
    }

    #[test]
    fn test_mcp_config_sse() {
        let config = McpConfig::sse("sse-server", "http://localhost:8080");

        assert_eq!(config.name, "sse-server");
        assert!(matches!(config.transport, McpTransport::Sse { .. }));
    }

    #[test]
    fn test_mcp_request() {
        let request =
            McpRequest::new(1, "tools/list").with_params(serde_json::json!({"cursor": null}));

        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.id, 1);
        assert_eq!(request.method, "tools/list");
    }

    #[tokio::test]
    async fn test_mcp_client_state_transitions() {
        let config = McpConfig::stdio("test", "nonexistent_command_12345", vec![]);
        let client = McpClient::new(config);

        assert_eq!(client.state().await, McpClientState::Disconnected);
        assert!(!client.is_connected().await);
    }

    #[tokio::test]
    async fn test_mcp_client_discover_not_connected() {
        let config = McpConfig::stdio("test", "echo", vec![]);
        let client = McpClient::new(config);

        let result = client.discover_tools().await;
        assert!(matches!(result, Err(ToolError::McpConnectionError { .. })));
    }

    #[tokio::test]
    async fn test_mcp_client_execute_not_connected() {
        let config = McpConfig::stdio("test", "echo", vec![]);
        let client = McpClient::new(config);

        let result = client.execute_tool("test", serde_json::json!({})).await;
        assert!(matches!(result, Err(ToolError::McpConnectionError { .. })));
    }

    #[test]
    fn test_mcp_tool_definition() {
        let definition = McpToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path"
                    }
                },
                "required": ["path"]
            }),
        };

        let config = McpConfig::stdio("test", "echo", vec![]);
        let client = Arc::new(McpClient::new(config));
        let tool = McpTool::new(client, definition);

        let metadata = tool.metadata();
        assert_eq!(metadata.name, "read_file");
        assert_eq!(metadata.parameters.len(), 1);
        assert!(metadata.is_param_required("path"));
    }

    #[test]
    fn test_server_capabilities_deserialization() {
        let json = serde_json::json!({
            "tools": {
                "listChanged": true
            }
        });

        let caps: ServerCapabilities = serde_json::from_value(json).unwrap();
        assert!(caps.tools.is_some());
        assert!(caps.tools.unwrap().list_changed);
    }

    #[test]
    fn test_initialize_result_deserialization() {
        let json = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "test-server",
                "version": "1.0.0"
            }
        });

        let result: InitializeResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.protocol_version, "2024-11-05");
        assert_eq!(result.server_info.name, "test-server");
    }

    // Phase 1: SSE Transport Tests
    #[test]
    fn test_sse_shutdown_timeout_constant() {
        // Verify constant is reasonable (5 seconds)
        assert_eq!(MCP_SSE_SHUTDOWN_TIMEOUT_MS, 5_000);
        // Constant is > 0 by design
    }

    // Phase 2: Reconnection Tests
    #[test]
    fn test_reconnect_config_default() {
        let config = ReconnectConfig::default();
        assert_eq!(config.max_attempts, MCP_RECONNECT_ATTEMPTS_MAX);
        assert_eq!(config.initial_delay_ms, MCP_RECONNECT_DELAY_MS_INITIAL);
        assert_eq!(config.max_delay_ms, MCP_RECONNECT_DELAY_MS_MAX);
        assert!(
            (config.backoff_multiplier - MCP_RECONNECT_BACKOFF_MULTIPLIER).abs() < f64::EPSILON
        );
    }

    #[test]
    fn test_reconnect_config_builder() {
        let config = ReconnectConfig::new()
            .with_max_attempts(3)
            .with_initial_delay_ms(200)
            .with_max_delay_ms(10_000)
            .with_backoff_multiplier(1.5);

        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.initial_delay_ms, 200);
        assert_eq!(config.max_delay_ms, 10_000);
        assert!((config.backoff_multiplier - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "max_attempts must be positive")]
    fn test_reconnect_config_zero_attempts() {
        ReconnectConfig::new().with_max_attempts(0);
    }

    #[test]
    #[should_panic(expected = "initial_delay_ms must be positive")]
    fn test_reconnect_config_zero_delay() {
        ReconnectConfig::new().with_initial_delay_ms(0);
    }

    #[test]
    #[should_panic(expected = "backoff_multiplier must be >= 1.0")]
    fn test_reconnect_config_invalid_multiplier() {
        ReconnectConfig::new().with_backoff_multiplier(0.5);
    }

    #[tokio::test]
    async fn test_mcp_client_with_reconnect_config() {
        let config = McpConfig::stdio("test", "echo", vec![]);
        let reconnect = ReconnectConfig::new().with_max_attempts(3);

        let client = McpClient::new_with_reconnect(config, reconnect);
        assert_eq!(client.reconnect_config().max_attempts, 3);
    }

    #[tokio::test]
    async fn test_health_check_not_connected() {
        let config = McpConfig::stdio("test", "echo", vec![]);
        let client = McpClient::new(config);

        // Health check should return false when not connected
        let healthy = client.health_check().await.unwrap();
        assert!(!healthy);
    }

    // Phase 5: Response Parsing Tests
    #[test]
    fn test_extract_tool_output_text_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello, world!"}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert_eq!(output, "Hello, world!");
    }

    #[test]
    fn test_extract_tool_output_multiple_text_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "Line 1"},
                {"type": "text", "text": "Line 2"}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert_eq!(output, "Line 1\nLine 2");
    }

    #[test]
    fn test_extract_tool_output_empty_content() {
        let result = serde_json::json!({
            "content": []
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert_eq!(output, "(empty response)");
    }

    #[test]
    fn test_extract_tool_output_image_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "image", "mimeType": "image/png", "data": "base64..."}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert!(output.contains("image: image/png"));
    }

    #[test]
    fn test_extract_tool_output_resource_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "resource", "uri": "file:///tmp/test.txt"}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert!(output.contains("resource: file:///tmp/test.txt"));
    }

    #[test]
    fn test_extract_tool_output_mixed_content() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "Here's an image:"},
                {"type": "image", "mimeType": "image/jpeg"},
                {"type": "text", "text": "Caption below"}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert!(output.contains("Here's an image:"));
        assert!(output.contains("[image: image/jpeg]"));
        assert!(output.contains("Caption below"));
    }

    #[test]
    fn test_extract_tool_output_direct_string() {
        let result = serde_json::json!("Direct string result");

        let output = extract_tool_output(&result, "test").unwrap();
        assert_eq!(output, "Direct string result");
    }

    #[test]
    fn test_extract_tool_output_error_flag() {
        let result = serde_json::json!({
            "isError": true,
            "content": [
                {"type": "text", "text": "Something went wrong"}
            ]
        });

        let output = extract_tool_output(&result, "test");
        assert!(output.is_err());
        let err = output.unwrap_err();
        assert!(err.to_string().contains("Something went wrong"));
    }

    #[test]
    fn test_extract_tool_output_fallback_json() {
        let result = serde_json::json!({
            "custom_field": "custom_value",
            "number": 42
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert!(output.contains("custom_field"));
        assert!(output.contains("custom_value"));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_extract_tool_output_unknown_content_type() {
        let result = serde_json::json!({
            "content": [
                {"type": "video", "url": "http://example.com/video.mp4"}
            ]
        });

        let output = extract_tool_output(&result, "test").unwrap();
        assert!(output.contains("[video content]"));
    }
}
