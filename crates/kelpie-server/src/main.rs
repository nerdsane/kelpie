//! Kelpie Server
//!
//! Standalone Kelpie server with Letta-compatible REST API.

mod api;

// Re-export from library
use kelpie_core::TokioRuntime;
use kelpie_server::state::AppState;
use kelpie_server::{llm, tools};
use tools::{register_heartbeat_tools, register_memory_tools};

use axum::extract::Request;
use axum::ServiceExt;
use clap::Parser;
use kelpie_server::tools::SandboxProvider;
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
    /// Can also be set via KELPIE_FDB_CLUSTER or FDB_CLUSTER_FILE env vars
    #[arg(long)]
    fdb_cluster_file: Option<String>,

    /// Force in-memory mode (no persistence)
    /// Disables FDB even if cluster file is configured or auto-detected
    #[arg(long)]
    memory_only: bool,
}

/// Storage backend detection result
enum StorageBackend {
    /// In-memory storage (no persistence)
    Memory,
    /// FoundationDB storage with cluster file path
    Fdb(String),
}

/// Standard paths to check for FDB cluster file
const FDB_CLUSTER_PATHS: &[&str] = &[
    "/etc/foundationdb/fdb.cluster",
    "/usr/local/etc/foundationdb/fdb.cluster",
    "/opt/foundationdb/fdb.cluster",
    "/var/foundationdb/fdb.cluster",
];

/// Detect the storage backend to use based on CLI flags, env vars, and auto-detection
///
/// Priority order:
/// 1. --memory-only flag (explicit in-memory mode)
/// 2. --fdb-cluster-file CLI argument
/// 3. KELPIE_FDB_CLUSTER env var
/// 4. FDB_CLUSTER_FILE env var (standard FDB env var)
/// 5. Auto-detect from standard paths
/// 6. Fall back to in-memory mode
fn detect_storage_backend(cli: &Cli) -> StorageBackend {
    // 1. Check explicit memory-only flag
    if cli.memory_only {
        tracing::info!("Storage: --memory-only flag set, using in-memory storage");
        return StorageBackend::Memory;
    }

    // 2. Check CLI argument
    if let Some(ref cluster_file) = cli.fdb_cluster_file {
        tracing::info!(
            "Storage: Using FDB cluster file from --fdb-cluster-file: {}",
            cluster_file
        );
        return StorageBackend::Fdb(cluster_file.clone());
    }

    // 3. Check KELPIE_FDB_CLUSTER env var
    if let Ok(cluster_file) = std::env::var("KELPIE_FDB_CLUSTER") {
        if !cluster_file.is_empty() {
            tracing::info!(
                "Storage: Using FDB cluster file from KELPIE_FDB_CLUSTER: {}",
                cluster_file
            );
            return StorageBackend::Fdb(cluster_file);
        }
    }

    // 4. Check FDB_CLUSTER_FILE env var (standard FDB env var)
    if let Ok(cluster_file) = std::env::var("FDB_CLUSTER_FILE") {
        if !cluster_file.is_empty() {
            tracing::info!(
                "Storage: Using FDB cluster file from FDB_CLUSTER_FILE: {}",
                cluster_file
            );
            return StorageBackend::Fdb(cluster_file);
        }
    }

    // 5. Auto-detect from standard paths
    for path in FDB_CLUSTER_PATHS {
        if std::path::Path::new(path).exists() {
            tracing::info!("Storage: Auto-detected FDB cluster file at: {}", path);
            return StorageBackend::Fdb((*path).to_string());
        }
    }

    // 6. Fall back to in-memory mode
    tracing::info!("Storage: No FDB cluster file found, using in-memory storage");
    tracing::info!("  To enable persistence, provide a cluster file via:");
    tracing::info!("    --fdb-cluster-file <path>");
    tracing::info!("    KELPIE_FDB_CLUSTER=<path>");
    tracing::info!("    FDB_CLUSTER_FILE=<path>");
    StorageBackend::Memory
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

    // Create runtime for dispatcher
    let runtime = TokioRuntime;

    // Detect and initialize storage backend
    let storage_backend = detect_storage_backend(&cli);
    let storage = match storage_backend {
        StorageBackend::Fdb(cluster_file) => {
            use kelpie_server::storage::FdbAgentRegistry;
            use kelpie_storage::FdbKV;

            tracing::info!("Connecting to FoundationDB: {}", cluster_file);
            let fdb_kv = FdbKV::connect(Some(&cluster_file))
                .await
                .map_err(|e| anyhow::anyhow!("Failed to connect to FDB: {}", e))?;

            let registry = FdbAgentRegistry::new(Arc::new(fdb_kv));
            tracing::info!("FDB storage initialized - data will be persisted");
            Some(Arc::new(registry) as Arc<dyn kelpie_server::storage::AgentStorage>)
        }
        StorageBackend::Memory => {
            tracing::warn!("Running in-memory mode - data will NOT be persisted!");
            tracing::warn!("Use --fdb-cluster-file or set KELPIE_FDB_CLUSTER for persistence");
            None
        }
    };

    // Create application state
    #[cfg(feature = "otel")]
    let state = if let Some(storage) = storage {
        AppState::with_storage_and_registry(
            runtime.clone(),
            storage,
            _telemetry_guard.registry().cloned(),
        )
    } else {
        AppState::with_registry(runtime.clone(), _telemetry_guard.registry())
    };

    #[cfg(not(feature = "otel"))]
    let state = if let Some(storage) = storage {
        AppState::with_storage(runtime.clone(), storage)
    } else {
        AppState::new(runtime.clone())
    };

    // Initialize sandbox provider (selects backend based on config)
    SandboxProvider::init()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize sandbox provider: {}", e))?;
    tracing::info!(
        backend = %kelpie_server::tools::SandboxBackendKind::detect(),
        "Sandbox provider initialized"
    );

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

    // Load MCP servers from storage (if configured)
    if let Err(err) = state.load_mcp_servers_from_storage().await {
        tracing::warn!(error = %err, "Failed to load MCP servers from storage");
    }

    // Load agent groups from storage (if configured)
    if let Err(err) = state.load_agent_groups_from_storage().await {
        tracing::warn!(error = %err, "Failed to load agent groups from storage");
    }

    // Load identities from storage (if configured)
    if let Err(err) = state.load_identities_from_storage().await {
        tracing::warn!(error = %err, "Failed to load identities from storage");
    }

    // Load projects from storage (if configured)
    if let Err(err) = state.load_projects_from_storage().await {
        tracing::warn!(error = %err, "Failed to load projects from storage");
    }

    // Load jobs from storage (if configured)
    if let Err(err) = state.load_jobs_from_storage().await {
        tracing::warn!(error = %err, "Failed to load jobs from storage");
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
async fn register_builtin_tools<R: kelpie_core::Runtime + 'static>(state: &AppState<R>) {
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
///
/// Uses SandboxProvider which selects the appropriate backend
/// (ProcessSandbox or VzSandbox) based on configuration.
async fn execute_shell_command(input: &Value) -> String {
    let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        return "Error: No command provided".to_string();
    }

    // Execute command via sh -c for shell expansion using sandbox provider
    match kelpie_server::tools::execute_in_sandbox("sh", &["-c", command], 30).await {
        Ok(result) => {
            if result.success {
                if result.stdout.is_empty() {
                    "Command executed successfully (no output)".to_string()
                } else {
                    // Truncate long output
                    if result.stdout.len() > 4000 {
                        format!(
                            "{}...\n[truncated, {} total bytes]",
                            &result.stdout[..4000],
                            result.stdout.len()
                        )
                    } else {
                        result.stdout
                    }
                }
            } else {
                format!(
                    "Command failed with exit code {}:\n{}{}",
                    result.exit_code, result.stdout, result.stderr
                )
            }
        }
        Err(e) => format!("Sandbox execution failed: {}", e),
    }
}
