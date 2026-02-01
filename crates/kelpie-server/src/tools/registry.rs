//! Unified Tool Registry Implementation
//!
//! TigerStyle: Single registry for all tool types with explicit source tracking.
//!
//! DST-Compliant: When the `dst` feature is enabled, supports FaultInjector
//! for testing custom tool execution error paths.

use crate::llm::ToolDefinition;
use crate::security::audit::SharedAuditLog;
use kelpie_core::io::{TimeProvider, WallClockTime};
use kelpie_sandbox::{ExecOptions, ProcessSandbox, Sandbox, SandboxConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[cfg(feature = "dst")]
use kelpie_dst::fault::{FaultInjector, FaultType};

/// Helper to compute elapsed time since start_ms using WallClockTime.
/// WallClockTime is zero-sized, so this has no allocation cost.
#[inline]
fn elapsed_ms(start_ms: u64) -> u64 {
    WallClockTime::new().monotonic_ms().saturating_sub(start_ms)
}

// =============================================================================
// Constants (TigerStyle)
// =============================================================================

/// Minimum pause duration in minutes
pub const HEARTBEAT_PAUSE_MINUTES_MIN: u64 = 1;

/// Maximum pause duration in minutes
pub const HEARTBEAT_PAUSE_MINUTES_MAX: u64 = 60;

/// Default pause duration in minutes
pub const HEARTBEAT_PAUSE_MINUTES_DEFAULT: u64 = 2;

/// Maximum agent loop iterations before forced stop
pub const AGENT_LOOP_ITERATIONS_MAX: u32 = 5;

/// Milliseconds per minute
pub const MS_PER_MINUTE: u64 = 60 * 1000;

/// Identifies where a tool comes from
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolSource {
    /// Built-in Rust tool
    Builtin,
    /// MCP server with server name
    Mcp { server: String },
    /// Custom user-defined tool
    Custom,
}

impl std::fmt::Display for ToolSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolSource::Builtin => write!(f, "builtin"),
            ToolSource::Mcp { server } => write!(f, "mcp:{}", server),
            ToolSource::Custom => write!(f, "custom"),
        }
    }
}

/// A registered tool with its metadata and source
#[derive(Debug, Clone)]
pub struct RegisteredTool {
    /// Tool definition (for LLM)
    pub definition: ToolDefinition,
    /// Where this tool comes from
    pub source: ToolSource,
    /// Optional description (more detailed than in definition)
    pub description: Option<String>,
}

/// Dispatcher trait for agent-to-agent communication (Issue #75)
///
/// TigerStyle: Trait abstraction allows DST testing with simulated dispatchers.
#[async_trait::async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Invoke another agent by ID
    ///
    /// # Arguments
    /// * `agent_id` - The ID of the agent to invoke (e.g., "helper-agent")
    /// * `operation` - The operation to invoke (e.g., "handle_message_full")
    /// * `payload` - The payload bytes (serialized request)
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Returns
    /// The response bytes from the target agent
    async fn invoke_agent(
        &self,
        agent_id: &str,
        operation: &str,
        payload: bytes::Bytes,
        timeout_ms: u64,
    ) -> kelpie_core::Result<bytes::Bytes>;
}

/// Execution context for tool calls
///
/// TigerStyle: Extended for multi-agent communication (Issue #75)
#[derive(Clone, Default)]
pub struct ToolExecutionContext {
    /// ID of the agent executing the tool
    pub agent_id: Option<String>,
    /// Project ID for the agent
    pub project_id: Option<String>,
    /// Current call depth for nested agent calls (0 = top level)
    pub call_depth: u32,
    /// Call chain for cycle detection (list of agent IDs in the call stack)
    pub call_chain: Vec<String>,
    /// Dispatcher for invoking other agents (Issue #75)
    /// None if agent-to-agent calls are not available
    pub dispatcher: Option<Arc<dyn AgentDispatcher>>,
    /// Audit log for recording tool executions
    /// None if audit logging is disabled
    pub audit_log: Option<SharedAuditLog>,
}

impl std::fmt::Debug for ToolExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolExecutionContext")
            .field("agent_id", &self.agent_id)
            .field("project_id", &self.project_id)
            .field("call_depth", &self.call_depth)
            .field("call_chain", &self.call_chain)
            .field("dispatcher", &self.dispatcher.is_some())
            .field("audit_log", &self.audit_log.is_some())
            .finish()
    }
}

/// Custom tool definition with source code
#[derive(Debug, Clone)]
pub struct CustomToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub source_code: String,
    pub runtime: String,
    pub requirements: Vec<String>,
}

/// Signals that tools can emit to control agent loop behavior
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ToolSignal {
    /// No signal - normal execution
    #[default]
    None,
    /// Pause heartbeats for the specified duration
    PauseHeartbeats {
        /// Time (in ms since epoch) when pause expires
        pause_until_ms: u64,
        /// Duration in minutes (for logging/display)
        minutes: u64,
    },
}

/// Result of tool execution
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    /// Output from the tool
    pub output: String,
    /// Whether execution succeeded
    pub success: bool,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Error message if failed
    pub error: Option<String>,
    /// Optional signal for agent loop control
    pub signal: ToolSignal,
}

impl ToolExecutionResult {
    /// Create a successful result
    pub fn success(output: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            output: output.into(),
            success: true,
            duration_ms,
            error: None,
            signal: ToolSignal::None,
        }
    }

    /// Create a failed result
    pub fn failure(error: impl Into<String>, duration_ms: u64) -> Self {
        let error_str = error.into();
        Self {
            output: error_str.clone(),
            success: false,
            duration_ms,
            error: Some(error_str),
            signal: ToolSignal::None,
        }
    }

    /// Add a pause heartbeats signal to this result
    pub fn with_pause_signal(mut self, pause_until_ms: u64, minutes: u64) -> Self {
        self.signal = ToolSignal::PauseHeartbeats {
            pause_until_ms,
            minutes,
        };
        self
    }
}

/// Handler function type for builtin tools
pub type BuiltinToolHandler = Arc<
    dyn Fn(&Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
        + Send
        + Sync,
>;

/// Handler function type for context-aware builtin tools (Issue #75)
///
/// TigerStyle: Separate type for tools that need execution context (e.g., call_agent).
/// These handlers receive the full ToolExecutionContext for dispatcher access.
pub type ContextAwareToolHandler = Arc<
    dyn Fn(
            &Value,
            &ToolExecutionContext,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolExecutionResult> + Send>>
        + Send
        + Sync,
>;

/// Unified tool registry combining all tool sources
pub struct UnifiedToolRegistry {
    /// All registered tools by name
    tools: RwLock<HashMap<String, RegisteredTool>>,
    /// Builtin tool handlers
    builtin_handlers: RwLock<HashMap<String, BuiltinToolHandler>>,
    /// Context-aware builtin tool handlers (Issue #75)
    context_aware_handlers: RwLock<HashMap<String, ContextAwareToolHandler>>,
    /// MCP client pool (server_name -> client) for production
    mcp_clients: RwLock<HashMap<String, Arc<kelpie_tools::McpClient>>>,
    /// Simulated MCP client for DST testing
    #[cfg(feature = "dst")]
    sim_mcp_client: RwLock<Option<Arc<kelpie_tools::SimMcpClient>>>,
    /// Custom tool definitions (source code + runtime)
    custom_tools: RwLock<HashMap<String, CustomToolDefinition>>,
    /// Optional sandbox pool for better performance (uses RwLock for interior mutability)
    sandbox_pool:
        RwLock<Option<Arc<kelpie_sandbox::SandboxPool<kelpie_sandbox::ProcessSandboxFactory>>>>,
    /// Fault injector for DST testing (optional)
    #[cfg(feature = "dst")]
    fault_injector: RwLock<Option<Arc<FaultInjector>>>,
}

impl UnifiedToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            builtin_handlers: RwLock::new(HashMap::new()),
            context_aware_handlers: RwLock::new(HashMap::new()),
            mcp_clients: RwLock::new(HashMap::new()),
            #[cfg(feature = "dst")]
            sim_mcp_client: RwLock::new(None),
            custom_tools: RwLock::new(HashMap::new()),
            sandbox_pool: RwLock::new(None),
            #[cfg(feature = "dst")]
            fault_injector: RwLock::new(None),
        }
    }

    /// Set the fault injector for DST testing
    ///
    /// When set, the registry will inject faults during custom tool execution
    /// based on the FaultInjector configuration.
    #[cfg(feature = "dst")]
    pub async fn set_fault_injector(&self, injector: Arc<FaultInjector>) {
        *self.fault_injector.write().await = Some(injector);
        tracing::info!("Fault injector configured for DST testing");
    }

    /// Check for fault injection and return the fault type if triggered
    #[cfg(feature = "dst")]
    async fn check_fault(&self, operation: &str) -> Option<FaultType> {
        let guard = self.fault_injector.read().await;
        guard.as_ref().and_then(|fi| fi.should_inject(operation))
    }

    /// Check for fault injection (no-op when dst feature is disabled)
    #[cfg(not(feature = "dst"))]
    #[allow(dead_code)]
    async fn check_fault(&self, _operation: &str) -> Option<()> {
        None
    }

    /// Set a sandbox pool for custom tool execution (builder pattern)
    ///
    /// When set, custom tools will use sandboxes from the pool for better performance
    /// (avoiding sandbox startup overhead on each execution).
    pub fn with_sandbox_pool(
        self,
        pool: Arc<kelpie_sandbox::SandboxPool<kelpie_sandbox::ProcessSandboxFactory>>,
    ) -> Self {
        // Use blocking lock since this is called during construction
        *self.sandbox_pool.blocking_write() = Some(pool);
        self
    }

    /// Set a sandbox pool after construction
    ///
    /// This allows setting the sandbox pool on an existing registry instance,
    /// which is useful when the registry is created by AppState.
    pub async fn set_sandbox_pool(
        &self,
        pool: Arc<kelpie_sandbox::SandboxPool<kelpie_sandbox::ProcessSandboxFactory>>,
    ) {
        *self.sandbox_pool.write().await = Some(pool);
        tracing::info!("Sandbox pool configured for custom tool execution");
    }

    /// Register a builtin tool
    pub async fn register_builtin(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        handler: BuiltinToolHandler,
    ) {
        let name = name.into();
        let description_str = description.into();

        // TigerStyle: Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");

        let definition = ToolDefinition {
            name: name.clone(),
            description: description_str.clone(),
            input_schema,
        };

        let tool = RegisteredTool {
            definition,
            source: ToolSource::Builtin,
            description: Some(description_str),
        };

        self.tools.write().await.insert(name.clone(), tool);
        self.builtin_handlers.write().await.insert(name, handler);
    }

    /// Register a context-aware builtin tool (Issue #75)
    ///
    /// Context-aware tools receive the full ToolExecutionContext, enabling:
    /// - Agent-to-agent calls via dispatcher
    /// - Call chain tracking for cycle detection
    /// - Call depth enforcement
    ///
    /// TigerStyle: Separate registration method for context-aware tools.
    pub async fn register_context_aware_builtin(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        handler: ContextAwareToolHandler,
    ) {
        let name = name.into();
        let description_str = description.into();

        // TigerStyle: Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");

        let definition = ToolDefinition {
            name: name.clone(),
            description: description_str.clone(),
            input_schema,
        };

        let tool = RegisteredTool {
            definition,
            source: ToolSource::Builtin,
            description: Some(description_str),
        };

        self.tools.write().await.insert(name.clone(), tool);
        self.context_aware_handlers
            .write()
            .await
            .insert(name, handler);
    }

    /// Register an MCP tool
    pub async fn register_mcp_tool(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        server_name: impl Into<String>,
    ) {
        let name = name.into();
        let description_str = description.into();
        let server = server_name.into();

        // TigerStyle: Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");
        assert!(!server.is_empty(), "server name cannot be empty");

        let definition = ToolDefinition {
            name: name.clone(),
            description: description_str.clone(),
            input_schema,
        };

        let tool = RegisteredTool {
            definition,
            source: ToolSource::Mcp {
                server: server.clone(),
            },
            description: Some(description_str),
        };

        tracing::debug!(
            tool_name = %name,
            server = %server,
            "Registering MCP tool in registry"
        );

        self.tools.write().await.insert(name, tool);
    }

    /// Register a custom tool with source code
    pub async fn register_custom_tool(
        &self,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        source_code: String,
        runtime: impl Into<String>,
        requirements: Vec<String>,
    ) {
        let name = name.into();
        let description_str = description.into();
        let runtime = runtime.into();

        assert!(!name.is_empty(), "tool name cannot be empty");
        assert!(!source_code.is_empty(), "source code cannot be empty");

        let definition = ToolDefinition {
            name: name.clone(),
            description: description_str.clone(),
            input_schema: input_schema.clone(),
        };

        let tool = RegisteredTool {
            definition,
            source: ToolSource::Custom,
            description: Some(description_str.clone()),
        };

        self.tools.write().await.insert(name.clone(), tool);
        self.custom_tools.write().await.insert(
            name.clone(),
            CustomToolDefinition {
                name,
                description: description_str,
                input_schema,
                source_code,
                runtime,
                requirements,
            },
        );
    }

    /// Set simulated MCP client for DST testing
    #[cfg(feature = "dst")]
    pub async fn set_sim_mcp_client(&self, client: Arc<kelpie_tools::SimMcpClient>) {
        *self.sim_mcp_client.write().await = Some(client);
    }

    /// Connect to an MCP server and auto-discover its tools
    ///
    /// # Arguments
    /// * `server_name` - Unique name for this MCP server connection
    /// * `config` - MCP configuration (transport, timeouts, env vars)
    ///
    /// # Returns
    /// Number of tools discovered and registered
    pub async fn connect_mcp_server(
        &self,
        server_name: impl Into<String>,
        config: kelpie_tools::McpConfig,
    ) -> Result<usize, String> {
        let server_name = server_name.into();

        // TigerStyle: Preconditions
        assert!(!server_name.is_empty(), "server name cannot be empty");

        // Create MCP client
        let client = Arc::new(kelpie_tools::McpClient::new(config));

        // Connect to server
        client
            .connect()
            .await
            .map_err(|e| format!("Failed to connect to MCP server '{}': {}", server_name, e))?;

        // Discover tools
        let tools = client.discover_tools().await.map_err(|e| {
            format!(
                "Failed to discover tools from MCP server '{}': {}",
                server_name, e
            )
        })?;

        // Register discovered tools
        let tool_count = tools.len();
        for tool in tools {
            self.register_mcp_tool(
                tool.name.clone(),
                tool.description.clone(),
                tool.input_schema,
                server_name.clone(),
            )
            .await;
        }

        // Store client
        self.mcp_clients.write().await.insert(server_name, client);

        Ok(tool_count)
    }

    /// Disconnect from an MCP server and unregister its tools
    pub async fn disconnect_mcp_server(&self, server_name: &str) -> Result<(), String> {
        // Remove client
        let client = self.mcp_clients.write().await.remove(server_name);

        if let Some(client) = client {
            // Disconnect
            client.disconnect().await.map_err(|e| {
                format!(
                    "Failed to disconnect from MCP server '{}': {}",
                    server_name, e
                )
            })?;

            // Remove tools from this server
            let mut tools = self.tools.write().await;
            tools.retain(|_, tool| {
                !matches!(&tool.source, ToolSource::Mcp { server } if server == server_name)
            });
        }

        Ok(())
    }

    /// Get all connected MCP server names
    pub async fn list_mcp_servers(&self) -> Vec<String> {
        self.mcp_clients.read().await.keys().cloned().collect()
    }

    /// Get all tool definitions for LLM
    pub async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .read()
            .await
            .values()
            .map(|t| t.definition.clone())
            .collect()
    }

    /// Get a specific tool by name
    pub async fn get_tool(&self, name: &str) -> Option<RegisteredTool> {
        self.tools.read().await.get(name).cloned()
    }

    /// Check if a tool exists
    pub async fn has_tool(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    /// Get tool source
    pub async fn get_tool_source(&self, name: &str) -> Option<ToolSource> {
        self.tools.read().await.get(name).map(|t| t.source.clone())
    }

    /// List all tool names
    pub async fn list_tools(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    /// List registered tools with metadata
    pub async fn list_registered_tools(&self) -> Vec<RegisteredTool> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Get a custom tool definition
    pub async fn get_custom_tool(&self, name: &str) -> Option<CustomToolDefinition> {
        self.custom_tools.read().await.get(name).cloned()
    }

    /// Execute a tool by name
    pub async fn execute(&self, name: &str, input: &Value) -> ToolExecutionResult {
        self.execute_with_context(name, input, None).await
    }

    /// Execute a tool by name with optional context
    pub async fn execute_with_context(
        &self,
        name: &str,
        input: &Value,
        context: Option<&ToolExecutionContext>,
    ) -> ToolExecutionResult {
        let start_ms = WallClockTime::new().monotonic_ms();

        // TigerStyle: Preconditions
        assert!(!name.is_empty(), "tool name cannot be empty");

        // Look up tool
        let tool = match self.tools.read().await.get(name) {
            Some(t) => t.clone(),
            None => {
                return ToolExecutionResult::failure(
                    format!("Tool not found: {}", name),
                    elapsed_ms(start_ms),
                );
            }
        };

        // Route to appropriate handler based on source
        let result = match &tool.source {
            ToolSource::Builtin => self.execute_builtin(name, input, context, start_ms).await,
            ToolSource::Mcp { server } => self.execute_mcp(name, server, input, start_ms).await,
            ToolSource::Custom => self.execute_custom(name, input, context, start_ms).await,
        };

        // Audit logging: Record tool execution if audit log is available
        if let Some(ctx) = context {
            if let Some(audit_log) = &ctx.audit_log {
                let input_str = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
                let agent_id = ctx.agent_id.as_deref().unwrap_or("unknown");

                audit_log.write().await.log_tool_execution(
                    name,
                    agent_id,
                    &input_str,
                    &result.output,
                    result.duration_ms,
                    result.success,
                    if result.success {
                        None
                    } else {
                        Some(result.output.clone())
                    },
                );
            }
        }

        result
    }

    /// Execute a builtin tool
    ///
    /// TigerStyle (Issue #75): Checks for context-aware handlers first.
    /// Context-aware tools (like call_agent) need dispatcher access for inter-agent calls.
    async fn execute_builtin(
        &self,
        name: &str,
        input: &Value,
        context: Option<&ToolExecutionContext>,
        start_ms: u64,
    ) -> ToolExecutionResult {
        // First, check for context-aware handler (Issue #75)
        if let Some(handler) = self.context_aware_handlers.read().await.get(name).cloned() {
            // Context-aware tools require context; provide default if not supplied
            let default_context = ToolExecutionContext::default();
            let ctx = context.unwrap_or(&default_context);
            return handler(input, ctx).await;
        }

        // Fall back to regular builtin handler
        let handler = match self.builtin_handlers.read().await.get(name) {
            Some(h) => h.clone(),
            None => {
                return ToolExecutionResult::failure(
                    format!("No handler for builtin tool: {}", name),
                    elapsed_ms(start_ms),
                );
            }
        };

        let output = handler(input).await;
        let duration = elapsed_ms(start_ms);

        // Check if output looks like an error
        let success = !output.starts_with("Error:") && !output.starts_with("Failed:");

        if success {
            ToolExecutionResult::success(output, duration)
        } else {
            ToolExecutionResult::failure(output, duration)
        }
    }

    /// Execute an MCP tool
    async fn execute_mcp(
        &self,
        name: &str,
        server: &str,
        input: &Value,
        start_ms: u64,
    ) -> ToolExecutionResult {
        // Check for simulated MCP client (DST mode)
        #[cfg(feature = "dst")]
        {
            if let Some(client) = self.sim_mcp_client.read().await.as_ref() {
                match client.execute_tool(name, input.clone()).await {
                    Ok(result) => {
                        let output = serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| result.to_string());
                        return ToolExecutionResult::success(output, elapsed_ms(start_ms));
                    }
                    Err(e) => {
                        return ToolExecutionResult::failure(e.to_string(), elapsed_ms(start_ms));
                    }
                }
            }
        }

        // Get MCP client for the server
        let client = {
            let clients = self.mcp_clients.read().await;
            match clients.get(server) {
                Some(client) => Arc::clone(client),
                None => {
                    return ToolExecutionResult::failure(
                        format!("MCP server '{}' not connected", server),
                        elapsed_ms(start_ms),
                    );
                }
            }
        };

        // Check if client is connected, attempt reconnection if not
        if !client.is_connected().await {
            tracing::warn!(server = %server, "MCP server disconnected, attempting reconnect");

            match client.reconnect().await {
                Ok(()) => {
                    tracing::info!(server = %server, "Successfully reconnected to MCP server");
                }
                Err(e) => {
                    return ToolExecutionResult::failure(
                        format!(
                            "MCP server '{}' is not connected and reconnection failed: {}",
                            server, e
                        ),
                        elapsed_ms(start_ms),
                    );
                }
            }
        }

        // Execute tool via MCP client
        match client.execute_tool(name, input.clone()).await {
            Ok(result) => {
                // Extract content using robust helper function
                match kelpie_tools::extract_tool_output(&result, name) {
                    Ok(output) => ToolExecutionResult::success(output, elapsed_ms(start_ms)),
                    Err(e) => ToolExecutionResult::failure(
                        format!("Failed to extract tool output: {}", e),
                        elapsed_ms(start_ms),
                    ),
                }
            }
            Err(e) => ToolExecutionResult::failure(
                format!("MCP tool execution failed: {}", e),
                elapsed_ms(start_ms),
            ),
        }
    }

    /// Execute a custom tool in a sandboxed runtime
    ///
    /// Supports Python, JavaScript (Node.js), and Shell (Bash) runtimes.
    ///
    /// DST-Compliant: When the `dst` feature is enabled, checks for fault injection
    /// before sandbox acquisition and execution.
    async fn execute_custom(
        &self,
        name: &str,
        input: &Value,
        context: Option<&ToolExecutionContext>,
        start_ms: u64,
    ) -> ToolExecutionResult {
        // DST: Check for custom tool execution faults
        #[cfg(feature = "dst")]
        if let Some(fault) = self.check_fault("custom_tool_execute").await {
            match fault {
                FaultType::CustomToolExecFail => {
                    return ToolExecutionResult::failure(
                        "DST fault injection: simulated custom tool execution failure",
                        elapsed_ms(start_ms),
                    );
                }
                FaultType::CustomToolExecTimeout { timeout_ms } => {
                    return ToolExecutionResult::failure(
                        format!(
                            "DST fault injection: simulated timeout after {}ms",
                            timeout_ms
                        ),
                        elapsed_ms(start_ms),
                    );
                }
                FaultType::CustomToolSandboxAcquireFail => {
                    return ToolExecutionResult::failure(
                        "DST fault injection: simulated sandbox acquisition failure (pool exhausted)",
                        elapsed_ms(start_ms),
                    );
                }
                _ => {}
            }
        }

        let Some(custom_tool) = self.custom_tools.read().await.get(name).cloned() else {
            return ToolExecutionResult::failure(
                format!("Custom tool not found: {}", name),
                elapsed_ms(start_ms),
            );
        };

        let runtime = custom_tool.runtime.to_lowercase();

        // Build the script and command based on runtime
        let (command, args, _script) = match runtime.as_str() {
            "python" | "py" => {
                let script = Self::build_python_wrapper(name, &custom_tool.source_code);
                (
                    "python3".to_string(),
                    vec!["-c".to_string(), script.clone()],
                    script,
                )
            }
            "javascript" | "js" | "node" => {
                let script = Self::build_javascript_wrapper(name, &custom_tool.source_code);
                (
                    "node".to_string(),
                    vec!["-e".to_string(), script.clone()],
                    script,
                )
            }
            "shell" | "bash" | "sh" => {
                let script = Self::build_shell_wrapper(input, &custom_tool.source_code);
                (
                    "bash".to_string(),
                    vec!["-c".to_string(), script.clone()],
                    script,
                )
            }
            _ => {
                return ToolExecutionResult::failure(
                    format!("Unsupported custom tool runtime: {}", custom_tool.runtime),
                    elapsed_ms(start_ms),
                );
            }
        };

        // Get or create sandbox
        let pool_guard = self.sandbox_pool.read().await;
        let result = if let Some(pool) = pool_guard.as_ref() {
            // Use sandbox from pool
            match pool.acquire().await {
                Ok(sandbox) => {
                    let result = self
                        .run_in_sandbox(
                            &sandbox,
                            &command,
                            &args,
                            input,
                            context,
                            &custom_tool,
                            start_ms,
                        )
                        .await;
                    pool.release(sandbox).await;
                    result
                }
                Err(e) => ToolExecutionResult::failure(
                    format!("Failed to acquire sandbox from pool: {}", e),
                    elapsed_ms(start_ms),
                ),
            }
        } else {
            // Create a one-off sandbox
            let mut sandbox = ProcessSandbox::new(SandboxConfig::default());
            if let Err(e) = sandbox.start().await {
                return ToolExecutionResult::failure(
                    format!("Failed to start sandbox: {}", e),
                    elapsed_ms(start_ms),
                );
            }

            let result = self
                .run_in_sandbox(
                    &sandbox,
                    &command,
                    &args,
                    input,
                    context,
                    &custom_tool,
                    start_ms,
                )
                .await;

            let _ = sandbox.stop().await;
            result
        };

        result
    }

    /// Run a command in a sandbox
    async fn run_in_sandbox(
        &self,
        sandbox: &ProcessSandbox,
        command: &str,
        args: &[String],
        input: &Value,
        context: Option<&ToolExecutionContext>,
        custom_tool: &CustomToolDefinition,
        start_ms: u64,
    ) -> ToolExecutionResult {
        // Install requirements for Python
        if command == "python3" && !custom_tool.requirements.is_empty() {
            let mut install_args = vec!["-m", "pip", "install"];
            for requirement in &custom_tool.requirements {
                install_args.push(requirement);
            }

            if let Err(e) = sandbox
                .exec(
                    "python3",
                    &install_args,
                    ExecOptions::new().with_timeout(Duration::from_secs(120)),
                )
                .await
            {
                return ToolExecutionResult::failure(
                    format!("Failed to install tool requirements: {}", e),
                    elapsed_ms(start_ms),
                );
            }
        }

        let mut exec_opts = ExecOptions::new()
            .with_timeout(Duration::from_secs(30))
            .with_max_output(1024 * 1024)
            .with_stdin(serde_json::to_vec(input).unwrap_or_default());

        if let Some(ctx) = context {
            if let Some(agent_id) = &ctx.agent_id {
                exec_opts = exec_opts.with_env("LETTA_AGENT_ID", agent_id.clone());
            }
            if let Some(project_id) = &ctx.project_id {
                exec_opts = exec_opts.with_env("LETTA_PROJECT_ID", project_id.clone());
            }
        }

        if let Ok(api_key) = std::env::var("LETTA_API_KEY") {
            exec_opts = exec_opts.with_env("LETTA_API_KEY", api_key);
        }
        if let Ok(base_url) = std::env::var("LETTA_BASE_URL") {
            exec_opts = exec_opts.with_env("LETTA_BASE_URL", base_url);
        }

        let args_strs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = match sandbox.exec(command, &args_strs, exec_opts).await {
            Ok(output) => output,
            Err(e) => {
                return ToolExecutionResult::failure(
                    format!("Custom tool execution failed: {}", e),
                    elapsed_ms(start_ms),
                )
            }
        };

        let duration = elapsed_ms(start_ms);
        if output.is_success() {
            ToolExecutionResult::success(output.stdout_string(), duration)
        } else {
            let error = output.stderr_string();
            ToolExecutionResult::failure(
                if error.is_empty() {
                    "Custom tool execution failed".to_string()
                } else {
                    error
                },
                duration,
            )
        }
    }

    /// Build Python wrapper script
    fn build_python_wrapper(name: &str, source_code: &str) -> String {
        let mut script = String::new();
        script.push_str("import json\nimport sys\n\n");
        script.push_str(source_code);
        script.push_str("\n\n");
        script.push_str("def _kelpie_call(args):\n");
        script.push_str(&format!("    fn = globals().get(\"{}\")\n", name));
        script.push_str("    if fn is None:\n");
        script.push_str(&format!(
            "        raise RuntimeError(\"Tool function '{}' not found\")\n",
            name
        ));
        script.push_str("    if isinstance(args, dict):\n");
        script.push_str("        try:\n");
        script.push_str("            return fn(**args)\n");
        script.push_str("        except TypeError:\n");
        script.push_str("            return fn(args)\n");
        script.push_str("    return fn(args)\n\n");
        script.push_str("def _kelpie_main():\n");
        script.push_str("    payload = sys.stdin.read()\n");
        script.push_str("    args = json.loads(payload) if payload else {}\n");
        script.push_str("    result = _kelpie_call(args)\n");
        script.push_str("    if not isinstance(result, str):\n");
        script.push_str("        try:\n");
        script.push_str("            result = json.dumps(result)\n");
        script.push_str("        except Exception:\n");
        script.push_str("            result = str(result)\n");
        script.push_str("    sys.stdout.write(result)\n\n");
        script.push_str("if __name__ == \"__main__\":\n");
        script.push_str("    _kelpie_main()\n");
        script
    }

    /// Build JavaScript wrapper script
    fn build_javascript_wrapper(name: &str, source_code: &str) -> String {
        let mut script = String::new();
        script.push_str(source_code);
        script.push_str("\n\n");
        script.push_str("(async function() {\n");
        script.push_str("  let input = '';\n");
        script.push_str("  process.stdin.setEncoding('utf8');\n");
        script.push_str("  for await (const chunk of process.stdin) {\n");
        script.push_str("    input += chunk;\n");
        script.push_str("  }\n");
        script.push_str("  const args = input ? JSON.parse(input) : {};\n");
        script.push_str(&format!(
            "  const fn = typeof {} === 'function' ? {} : null;\n",
            name, name
        ));
        script.push_str("  if (!fn) {\n");
        script.push_str(&format!(
            "    throw new Error(\"Tool function '{}' not found\");\n",
            name
        ));
        script.push_str("  }\n");
        script.push_str("  let result = await fn(args);\n");
        script.push_str("  if (typeof result !== 'string') {\n");
        script.push_str("    result = JSON.stringify(result);\n");
        script.push_str("  }\n");
        script.push_str("  process.stdout.write(result);\n");
        script.push_str("})();\n");
        script
    }

    /// Build Shell wrapper script
    fn build_shell_wrapper(input: &Value, source_code: &str) -> String {
        let mut script = String::new();
        script.push_str("#!/bin/bash\n");
        script.push_str("set -e\n\n");

        // Export input as environment variables if it's an object
        if let Some(obj) = input.as_object() {
            for (key, value) in obj {
                let val_str = match value {
                    Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                // Escape single quotes for bash
                let escaped = val_str.replace('\'', "'\\''");
                script.push_str(&format!(
                    "export TOOL_{}='{}'\n",
                    key.to_uppercase(),
                    escaped
                ));
            }
        }

        script.push('\n');
        script.push_str(source_code);
        script
    }

    /// Unregister a tool from all registries
    ///
    /// Removes the tool from the main tools registry, builtin handlers, and custom tools.
    /// Returns true if the tool was found and removed from any registry.
    ///
    /// TigerStyle: Cleanup operation for MCP server deletion or custom tool removal
    pub async fn unregister(&self, name: &str) -> bool {
        assert!(!name.is_empty(), "tool name cannot be empty");

        let removed_tool = self.tools.write().await.remove(name).is_some();
        let removed_handler = self.builtin_handlers.write().await.remove(name).is_some();
        let removed_custom = self.custom_tools.write().await.remove(name).is_some();

        let removed = removed_tool || removed_handler || removed_custom;
        if removed {
            tracing::debug!(tool_name = %name, "Unregistered tool from registry");
        }
        removed
    }

    /// Clear all tools
    pub async fn clear(&self) {
        self.tools.write().await.clear();
        self.builtin_handlers.write().await.clear();
        self.custom_tools.write().await.clear();
    }

    /// Get statistics about registered tools
    pub async fn stats(&self) -> RegistryStats {
        let tools = self.tools.read().await;
        let mut builtin_count = 0;
        let mut mcp_count = 0;
        let mut custom_count = 0;

        for tool in tools.values() {
            match &tool.source {
                ToolSource::Builtin => builtin_count += 1,
                ToolSource::Mcp { .. } => mcp_count += 1,
                ToolSource::Custom => custom_count += 1,
            }
        }

        RegistryStats {
            total_tools: tools.len(),
            builtin_count,
            mcp_count,
            custom_count,
        }
    }
}

impl Default for UnifiedToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about registered tools
#[derive(Debug, Clone)]
pub struct RegistryStats {
    pub total_tools: usize,
    pub builtin_count: usize,
    pub mcp_count: usize,
    pub custom_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_register_builtin_tool() {
        let registry = UnifiedToolRegistry::new();

        // Create a simple echo handler
        let handler: BuiltinToolHandler = Arc::new(|input| {
            let input = input.clone();
            Box::pin(async move {
                input
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            })
        });

        registry
            .register_builtin(
                "echo",
                "Echo back the input",
                json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    }
                }),
                handler,
            )
            .await;

        assert!(registry.has_tool("echo").await);
        assert_eq!(
            registry.get_tool_source("echo").await,
            Some(ToolSource::Builtin)
        );
    }

    #[tokio::test]
    async fn test_register_mcp_tool() {
        let registry = UnifiedToolRegistry::new();

        registry
            .register_mcp_tool(
                "external_tool",
                "An external tool",
                json!({"type": "object"}),
                "test-server",
            )
            .await;

        assert!(registry.has_tool("external_tool").await);
        assert_eq!(
            registry.get_tool_source("external_tool").await,
            Some(ToolSource::Mcp {
                server: "test-server".to_string()
            })
        );
    }

    #[tokio::test]
    async fn test_execute_builtin_tool() {
        let registry = UnifiedToolRegistry::new();

        let handler: BuiltinToolHandler = Arc::new(|input| {
            let input = input.clone();
            Box::pin(async move {
                let msg = input
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no message");
                format!("Echo: {}", msg)
            })
        });

        registry
            .register_builtin("echo", "Echo tool", json!({"type": "object"}), handler)
            .await;

        let result = registry.execute("echo", &json!({"message": "hello"})).await;

        assert!(result.success);
        assert_eq!(result.output, "Echo: hello");
    }

    #[tokio::test]
    async fn test_get_tool_definitions() {
        let registry = UnifiedToolRegistry::new();

        let handler: BuiltinToolHandler =
            Arc::new(|_| Box::pin(async move { "result".to_string() }));

        registry
            .register_builtin(
                "tool1",
                "Tool 1",
                json!({"type": "object"}),
                handler.clone(),
            )
            .await;

        registry
            .register_builtin("tool2", "Tool 2", json!({"type": "object"}), handler)
            .await;

        let definitions = registry.get_tool_definitions().await;
        assert_eq!(definitions.len(), 2);
    }

    #[tokio::test]
    async fn test_registry_stats() {
        let registry = UnifiedToolRegistry::new();

        let handler: BuiltinToolHandler =
            Arc::new(|_| Box::pin(async move { "result".to_string() }));

        registry
            .register_builtin("builtin1", "Builtin 1", json!({}), handler)
            .await;

        registry
            .register_mcp_tool("mcp1", "MCP 1", json!({}), "server1")
            .await;

        registry
            .register_mcp_tool("mcp2", "MCP 2", json!({}), "server2")
            .await;

        let stats = registry.stats().await;
        assert_eq!(stats.total_tools, 3);
        assert_eq!(stats.builtin_count, 1);
        assert_eq!(stats.mcp_count, 2);
        assert_eq!(stats.custom_count, 0);
    }

    #[tokio::test]
    async fn test_tool_not_found() {
        let registry = UnifiedToolRegistry::new();

        let result = registry.execute("nonexistent", &json!({})).await;

        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_mcp_server_not_connected() {
        let registry = UnifiedToolRegistry::new();

        // Register an MCP tool without connecting the server
        registry
            .register_mcp_tool("test_tool", "Test tool", json!({}), "test_server")
            .await;

        // Try to execute - should fail because server not connected
        let result = registry.execute("test_tool", &json!({})).await;

        assert!(!result.success);
        assert!(result.error.is_some());
        let error = result.error.unwrap();
        assert!(error.contains("not connected"));
    }

    #[tokio::test]
    async fn test_list_mcp_servers() {
        let registry = UnifiedToolRegistry::new();

        // Initially no servers
        let servers = registry.list_mcp_servers().await;
        assert_eq!(servers.len(), 0);

        // Note: Can't test actual connection without a real MCP server
        // Full integration tests will be in separate test file
    }

    #[tokio::test]
    async fn test_mcp_execute_with_text_content() {
        let registry = UnifiedToolRegistry::new();

        // Register MCP tool
        registry
            .register_mcp_tool("test_tool", "Test", json!({}), "server1")
            .await;

        // Note: execute_mcp is private, so we test through execute()
        // which will fail because server not connected
        // Full MCP execution tests require real or mock MCP client
    }
}
