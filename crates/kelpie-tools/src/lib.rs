//! Kelpie Tools - Tool integration layer for agents
//!
//! TigerStyle: Explicit tool definitions with sandboxed execution.
//!
//! # Overview
//!
//! This crate provides:
//! - Tool trait for defining callable tools
//! - ToolRegistry for discovery and management
//! - MCP (Model Context Protocol) client support
//! - Built-in tools (shell, filesystem, git)
//! - Sandboxed tool execution
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_tools::{Tool, ToolRegistry, ToolInput, ToolOutput};
//!
//! // Create registry and register tools
//! let mut registry = ToolRegistry::new();
//! registry.register(ShellTool::new());
//!
//! // Execute a tool
//! let input = ToolInput::new("shell")
//!     .with_param("command", "echo hello");
//! let output = registry.execute("shell", input).await?;
//! ```

mod builtin;
mod error;
pub mod http_client;
pub mod http_tool;
pub mod mcp;
mod registry;
#[cfg(feature = "dst")]
pub mod sim;
mod traits;

pub use builtin::{FilesystemTool, GitTool, ShellTool};
pub use error::{ToolError, ToolResult};
pub use http_client::{
    default_http_client, HttpClient, HttpError, HttpRequest, HttpResponse, HttpResult,
    ReqwestHttpClient, HTTP_CLIENT_RESPONSE_BYTES_MAX, HTTP_CLIENT_TIMEOUT_MS_DEFAULT,
};
pub use http_tool::{HttpMethod, HttpTool, HttpToolDefinition};
pub use mcp::{
    extract_tool_output, McpClient, McpConfig, McpTool, McpToolDefinition, ReconnectConfig,
    MCP_HEALTH_CHECK_INTERVAL_MS, MCP_RECONNECT_ATTEMPTS_MAX, MCP_RECONNECT_BACKOFF_MULTIPLIER,
    MCP_RECONNECT_DELAY_MS_INITIAL, MCP_RECONNECT_DELAY_MS_MAX, MCP_SSE_SHUTDOWN_TIMEOUT_MS,
};
pub use registry::ToolRegistry;
#[cfg(feature = "dst")]
pub use sim::{
    create_test_tools, ConnectionState, SimMcpClient, SimMcpEnvironment, SimMcpServerConfig,
};
pub use traits::{Tool, ToolCapability, ToolInput, ToolMetadata, ToolOutput, ToolParam};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_module_compiles() {
        // Verify public types are accessible
        let _registry = ToolRegistry::new();
        let _input = ToolInput::new("test");
        let _output = ToolOutput::success("result");
    }
}
