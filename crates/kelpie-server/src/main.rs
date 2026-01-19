//! Kelpie Server
//!
//! Standalone Kelpie server with Letta-compatible REST API.

mod api;

// Re-export from library
use kelpie_server::state::AppState;
use kelpie_server::{llm, tools};
use tools::{register_heartbeat_tools, register_memory_tools};

use axum::extract::Request;
use axum::ServiceExt;
use clap::Parser;
use kelpie_sandbox::{ExecOptions, ProcessSandbox, Sandbox, SandboxConfig};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tools::BuiltinToolHandler;
use tower_http::normalize_path::NormalizePath;

#[cfg(feature = "otel")]
use kelpie_core::telemetry::{init_telemetry, TelemetryConfig};

#[cfg(not(feature = "otel"))]
use tracing_subscriber::EnvFilter;

/// Kelpie server CLI
#[derive(Parser, Debug)]
#[command(name = "kelpie-server")]
#[command(about = "Kelpie distributed virtual actor server with Letta-compatible API")]
#[command(version)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "kelpie.yaml")]
    config: String,

    /// HTTP API bind address
    #[arg(short, long, default_value = "0.0.0.0:8283")]
    bind: String,

    /// Enable verbose logging
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// FoundationDB cluster file path (enables FDB storage)
    #[cfg(feature = "fdb")]
    #[arg(long)]
    fdb_cluster_file: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize telemetry
    #[cfg(feature = "otel")]
    let _telemetry_guard = {
        let config = TelemetryConfig::from_env().with_metrics(9090);
        init_telemetry(config)
            .map_err(|e| anyhow::anyhow!("Failed to initialize telemetry: {}", e))?
    };

    // Fallback logging when otel feature not enabled
    #[cfg(not(feature = "otel"))]
    {
        let filter = match cli.verbose {
            0 => "info,tower_http=debug",
            1 => "debug",
            _ => "trace",
        };

        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
            .init();
    }

    tracing::info!("Kelpie server v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Config: {}", cli.config);

    // Parse bind address
    let addr: SocketAddr = cli
        .bind
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bind address '{}': {}", cli.bind, e))?;

    // Initialize storage backend (if configured)
    #[cfg(feature = "fdb")]
    let storage = if let Some(ref cluster_file) = cli.fdb_cluster_file {
        use kelpie_server::storage::FdbAgentRegistry;
        use kelpie_storage::FdbKV;

        tracing::info!("Connecting to FoundationDB: {}", cluster_file);
        let fdb_kv = FdbKV::connect(Some(cluster_file))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to FDB: {}", e))?;

        let registry = FdbAgentRegistry::new(Arc::new(fdb_kv));
        tracing::info!("FDB storage initialized");
        Some(Arc::new(registry) as Arc<dyn kelpie_server::storage::AgentStorage>)
    } else {
        tracing::info!("Running in-memory mode (no persistence)");
        None
    };

    #[cfg(not(feature = "fdb"))]
    let storage: Option<Arc<dyn kelpie_server::storage::AgentStorage>> = {
        tracing::info!("Running in-memory mode (no persistence)");
        None
    };

    // Create application state
    #[cfg(feature = "otel")]
    let state = if let Some(storage) = storage {
        AppState::with_storage_and_registry(storage, _telemetry_guard.registry().cloned())
    } else {
        AppState::with_registry(_telemetry_guard.registry())
    };

    #[cfg(not(feature = "otel"))]
    let state = if let Some(storage) = storage {
        AppState::with_storage(storage)
    } else {
        AppState::new()
    };

    // Register builtin tools
    register_builtin_tools(&state).await;

    // Register memory tools
    register_memory_tools(state.tool_registry(), state.clone()).await;

    // Register heartbeat tools
    register_heartbeat_tools(state.tool_registry()).await;

    // Register messaging tools
    tools::register_messaging_tools(state.tool_registry()).await;

    // Register web search tool
    tools::register_web_search_tool(state.tool_registry()).await;

    // Register code execution tool
    tools::register_run_code_tool(state.tool_registry()).await;

    // Load custom tools from storage (if configured)
    if let Err(err) = state.load_custom_tools().await {
        tracing::warn!(error = %err, "Failed to load custom tools from storage");
    }

    // Load agents from storage (if configured)
    if let Err(err) = state.load_agents_from_storage().await {
        tracing::warn!(error = %err, "Failed to load agents from storage");
    }

    // Create router
    let app = api::router(state);

    // Start server
    tracing::info!("Starting HTTP server on {}", addr);
    tracing::info!("API endpoints:");
    tracing::info!("  GET  /health                           - Health check");
    tracing::info!("  GET  /metrics                          - Prometheus metrics");
    tracing::info!("  GET  /v1/capabilities                  - Server capabilities");
    tracing::info!("  GET  /v1/agents                        - List agents");
    tracing::info!("  GET  /v1/agents/{{id}}                   - Get agent");
    tracing::info!("  PATCH /v1/agents/{{id}}                  - Update agent");
    tracing::info!("  DELETE /v1/agents/{{id}}                 - Delete agent");
    tracing::info!("  GET  /v1/agents/{{id}}/blocks            - List blocks");
    tracing::info!("  PATCH /v1/agents/{{id}}/blocks/{{bid}}     - Update block");
    tracing::info!("  GET  /v1/agents/{{id}}/messages          - List messages");
    tracing::info!("  POST /v1/agents/{{id}}/messages          - Send message");
    tracing::info!("  GET  /v1/agents/{{id}}/archival          - Search archival memory");
    tracing::info!("  POST /v1/agents/{{id}}/archival          - Add to archival memory");
    tracing::info!("  GET  /v1/tools                         - List tools");
    tracing::info!("  POST /v1/tools                         - Register tool");
    tracing::info!("  POST /v1/tools/{{name}}/execute          - Execute tool");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Wrap with NormalizePath to handle trailing slashes (letta-code compatibility)
    let app = NormalizePath::trim_trailing_slash(app);
    axum::serve(listener, ServiceExt::<Request>::into_make_service(app)).await?;

    Ok(())
}

/// Register builtin tools with the unified registry
async fn register_builtin_tools(state: &AppState) {
    let registry = state.tool_registry();

    // Register shell tool
    let shell_handler: BuiltinToolHandler = Arc::new(|input: &Value| {
        let input = input.clone();
        Box::pin(async move { execute_shell_command(&input).await })
    });

    registry
        .register_builtin(
            "shell",
            "Execute a shell command. Use this to run commands, check files, or perform system operations.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
            shell_handler,
        )
        .await;

    tracing::info!("Registered builtin tools: shell");
}

/// Execute a shell command in a sandboxed environment
async fn execute_shell_command(input: &Value) -> String {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        return "Error: No command provided".to_string();
    }

    // Create and start sandbox
    let config = SandboxConfig::default();
    let mut sandbox = ProcessSandbox::new(config);

    if let Err(e) = sandbox.start().await {
        return format!("Failed to start sandbox: {}", e);
    }

    // Execute command via sh -c for shell expansion
    let exec_opts = ExecOptions::new()
        .with_timeout(std::time::Duration::from_secs(30))
        .with_max_output(1024 * 1024);

    match sandbox.exec("sh", &["-c", command], exec_opts).await {
        Ok(output) => {
            let stdout = output.stdout_string();
            let stderr = output.stderr_string();

            if output.is_success() {
                if stdout.is_empty() {
                    "Command executed successfully (no output)".to_string()
                } else {
                    // Truncate long output
                    if stdout.len() > 4000 {
                        format!(
                            "{}...\n[truncated, {} total bytes]",
                            &stdout[..4000],
                            stdout.len()
                        )
                    } else {
                        stdout
                    }
                }
            } else {
                format!(
                    "Command failed with exit code {}:\n{}{}",
                    output.status.code, stdout, stderr
                )
            }
        }
        Err(e) => format!("Sandbox execution failed: {}", e),
    }
}
