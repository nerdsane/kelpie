//! Tool registry for discovery and management
//!
//! TigerStyle: Centralized tool management with explicit lifecycle.
//!
//! DST-Compliant: Uses TimeProvider abstraction for deterministic testing.

use crate::error::{ToolError, ToolResult};
use crate::traits::{DynTool, Tool, ToolInput, ToolMetadata, ToolOutput};
use kelpie_core::io::{TimeProvider, WallClockTime};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maximum number of tools in a registry
pub const REGISTRY_TOOLS_COUNT_MAX: usize = 1000;

/// Tool registry for managing available tools
///
/// Provides:
/// - Tool registration and discovery
/// - Tool execution with timeout handling
/// - Statistics tracking
///
/// DST-Compliant: Uses TimeProvider for deterministic timing in tests.
pub struct ToolRegistry {
    /// Registered tools
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    /// Execution statistics
    stats: RwLock<RegistryStats>,
    /// Time provider for DST-compatible timing
    time_provider: Arc<dyn TimeProvider>,
}

/// Registry statistics
#[derive(Debug, Clone, Default)]
pub struct RegistryStats {
    /// Total tool executions
    pub total_executions: u64,
    /// Successful executions
    pub successful_executions: u64,
    /// Failed executions
    pub failed_executions: u64,
    /// Timed out executions
    pub timed_out_executions: u64,
    /// Per-tool execution counts
    pub tool_executions: HashMap<String, u64>,
}

impl ToolRegistry {
    /// Create a new empty registry with wall clock time (production default)
    pub fn new() -> Self {
        Self::with_time_provider(Arc::new(WallClockTime::new()))
    }

    /// Create a new registry with custom TimeProvider (for DST)
    pub fn with_time_provider(time_provider: Arc<dyn TimeProvider>) -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            stats: RwLock::new(RegistryStats::default()),
            time_provider,
        }
    }

    /// Register a tool
    pub async fn register<T: Tool + 'static>(&self, tool: T) -> ToolResult<()> {
        let name = tool.name().to_string();

        let mut tools = self.tools.write().await;

        if tools.len() >= REGISTRY_TOOLS_COUNT_MAX {
            return Err(ToolError::ConfigError {
                reason: format!(
                    "registry is at capacity ({} tools)",
                    REGISTRY_TOOLS_COUNT_MAX
                ),
            });
        }

        if tools.contains_key(&name) {
            return Err(ToolError::AlreadyRegistered { name });
        }

        info!(tool = %name, "Registering tool");
        tools.insert(name, Arc::new(tool));
        Ok(())
    }

    /// Register a boxed tool
    pub async fn register_boxed(&self, tool: DynTool) -> ToolResult<()> {
        let name = tool.name().to_string();

        let mut tools = self.tools.write().await;

        if tools.len() >= REGISTRY_TOOLS_COUNT_MAX {
            return Err(ToolError::ConfigError {
                reason: format!(
                    "registry is at capacity ({} tools)",
                    REGISTRY_TOOLS_COUNT_MAX
                ),
            });
        }

        if tools.contains_key(&name) {
            return Err(ToolError::AlreadyRegistered { name });
        }

        info!(tool = %name, "Registering boxed tool");
        tools.insert(name, Arc::from(tool));
        Ok(())
    }

    /// Unregister a tool
    pub async fn unregister(&self, name: &str) -> ToolResult<()> {
        let mut tools = self.tools.write().await;

        if tools.remove(name).is_none() {
            return Err(ToolError::NotFound {
                name: name.to_string(),
            });
        }

        info!(tool = %name, "Unregistered tool");
        Ok(())
    }

    /// Get a tool by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// Check if a tool exists
    pub async fn contains(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    /// List all registered tool names
    pub async fn list_names(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        tools.keys().cloned().collect()
    }

    /// List all tool metadata
    pub async fn list_metadata(&self) -> Vec<ToolMetadata> {
        let tools = self.tools.read().await;
        tools.values().map(|t| t.metadata().clone()).collect()
    }

    /// Get tool count
    pub async fn count(&self) -> usize {
        let tools = self.tools.read().await;
        tools.len()
    }

    /// Execute a tool by name
    pub async fn execute(&self, name: &str, input: ToolInput) -> ToolResult<ToolOutput> {
        let tool = self.get(name).await.ok_or_else(|| ToolError::NotFound {
            name: name.to_string(),
        })?;

        // Validate input
        tool.validate(&input)?;

        let timeout_ms = tool.metadata().timeout_ms;
        let timeout_duration = std::time::Duration::from_millis(timeout_ms);

        debug!(tool = %name, timeout_ms = %timeout_ms, "Executing tool");

        let start_ms = self.time_provider.monotonic_ms();

        // Execute with timeout
        let runtime = kelpie_core::current_runtime();
        let result =
            kelpie_core::Runtime::timeout(&runtime, timeout_duration, tool.execute(input)).await;

        let duration_ms = self.time_provider.monotonic_ms().saturating_sub(start_ms);

        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.total_executions += 1;
            *stats.tool_executions.entry(name.to_string()).or_insert(0) += 1;

            match &result {
                Ok(Ok(output)) if output.is_success() => {
                    stats.successful_executions += 1;
                }
                Ok(Ok(_)) | Ok(Err(_)) => {
                    stats.failed_executions += 1;
                }
                Err(_) => {
                    stats.timed_out_executions += 1;
                }
            }
        }

        match result {
            Ok(Ok(output)) => {
                debug!(
                    tool = %name,
                    duration_ms = %duration_ms,
                    success = %output.is_success(),
                    "Tool execution completed"
                );
                Ok(output.with_duration(duration_ms))
            }
            Ok(Err(e)) => {
                warn!(tool = %name, error = %e, "Tool execution failed");
                Err(e)
            }
            Err(_) => {
                warn!(tool = %name, timeout_ms = %timeout_ms, "Tool execution timed out");
                Err(ToolError::ExecutionTimeout {
                    tool: name.to_string(),
                    timeout_ms,
                })
            }
        }
    }

    /// Get registry statistics
    pub async fn stats(&self) -> RegistryStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// Reset statistics
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = RegistryStats::default();
    }

    /// Clear all tools from the registry
    pub async fn clear(&self) {
        let mut tools = self.tools.write().await;
        tools.clear();
        info!("Cleared tool registry");
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ToolParam;
    use async_trait::async_trait;

    // Test tool implementation
    struct EchoTool {
        metadata: ToolMetadata,
    }

    impl EchoTool {
        fn new() -> Self {
            Self {
                metadata: ToolMetadata::new("echo", "Echoes input back")
                    .with_param(ToolParam::string("message", "Message to echo")),
            }
        }
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.metadata
        }

        async fn execute(&self, input: ToolInput) -> ToolResult<ToolOutput> {
            let message = input
                .get_string("message")
                .unwrap_or("no message")
                .to_string();
            Ok(ToolOutput::success(message))
        }
    }

    // Slow tool for timeout testing
    struct SlowTool {
        metadata: ToolMetadata,
    }

    impl SlowTool {
        fn new() -> Self {
            Self {
                metadata: ToolMetadata::new("slow", "A slow tool")
                    .with_timeout(std::time::Duration::from_millis(100)),
            }
        }
    }

    #[async_trait]
    impl Tool for SlowTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.metadata
        }

        async fn execute(&self, _input: ToolInput) -> ToolResult<ToolOutput> {
            let runtime = kelpie_core::current_runtime();
            kelpie_core::Runtime::sleep(&runtime, std::time::Duration::from_secs(10)).await;
            Ok(ToolOutput::success("done"))
        }
    }

    #[tokio::test]
    async fn test_registry_register() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        assert!(registry.contains("echo").await);
        assert_eq!(registry.count().await, 1);
    }

    #[tokio::test]
    async fn test_registry_register_duplicate() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        let result = registry.register(EchoTool::new()).await;
        assert!(matches!(result, Err(ToolError::AlreadyRegistered { .. })));
    }

    #[tokio::test]
    async fn test_registry_unregister() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        registry.unregister("echo").await.unwrap();
        assert!(!registry.contains("echo").await);
    }

    #[tokio::test]
    async fn test_registry_execute() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        let input = ToolInput::new("echo").with_param("message", "hello world");
        let output = registry.execute("echo", input).await.unwrap();

        assert!(output.is_success());
        assert_eq!(output.result_string(), Some("hello world".to_string()));
    }

    #[tokio::test]
    async fn test_registry_execute_not_found() {
        let registry = ToolRegistry::new();

        let input = ToolInput::new("nonexistent");
        let result = registry.execute("nonexistent", input).await;

        assert!(matches!(result, Err(ToolError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_registry_execute_timeout() {
        let registry = ToolRegistry::new();
        registry.register(SlowTool::new()).await.unwrap();

        let input = ToolInput::new("slow");
        let result = registry.execute("slow", input).await;

        assert!(matches!(result, Err(ToolError::ExecutionTimeout { .. })));
    }

    #[tokio::test]
    async fn test_registry_list_metadata() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        let metadata = registry.list_metadata().await;
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].name, "echo");
    }

    #[tokio::test]
    async fn test_registry_stats() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        let input = ToolInput::new("echo").with_param("message", "test");
        registry.execute("echo", input).await.unwrap();

        let stats = registry.stats().await;
        assert_eq!(stats.total_executions, 1);
        assert_eq!(stats.successful_executions, 1);
    }

    #[tokio::test]
    async fn test_registry_clear() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool::new()).await.unwrap();

        registry.clear().await;
        assert_eq!(registry.count().await, 0);
    }
}
