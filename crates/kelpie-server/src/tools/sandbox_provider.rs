//! Sandbox Provider for Code Execution
//!
//! TigerStyle: Abstracts over ProcessSandbox and LibkrunSandbox backends.
//!
//! # Backend Selection
//!
//! The sandbox backend is selected based on:
//! 1. Feature flags (libkrun feature enables libkrun VM support)
//! 2. Runtime environment (macOS ARM64 or Linux for libkrun)
//! 3. Configuration (KELPIE_SANDBOX_BACKEND env var)
//!
//! # Isolation Modes
//!
//! - **Shared** (default): All agents share sandbox instances for efficiency
//! - **Dedicated**: Each agent gets its own sandbox for maximum isolation
//!   (controlled by KELPIE_ISOLATION_MODE=dedicated)
//!
//! # Usage
//!
//! ```ignore
//! use kelpie_server::tools::sandbox_provider::{SandboxProvider, execute_in_sandbox};
//!
//! // Create provider (initializes backend based on config)
//! let provider = SandboxProvider::new().await?;
//!
//! // Execute command in sandbox (shared mode)
//! let output = execute_in_sandbox("python3", &["-c", "print('hello')"], 30).await?;
//!
//! // Execute command in per-agent sandbox (dedicated mode)
//! let output = execute_for_agent("agent-123", "python3", &["-c", "print('hello')"], 30).await?;
//! ```

use kelpie_sandbox::{
    AgentSandboxManager, ExecOptions, IsolationMode, PoolConfig, ProcessSandbox,
    ProcessSandboxFactory, Sandbox, SandboxConfig,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

// libkrun imports
#[cfg(feature = "libkrun")]
use kelpie_vm::{LibkrunSandboxConfig, LibkrunSandboxFactory};

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Default execution timeout in seconds
pub const EXEC_TIMEOUT_SECONDS_DEFAULT: u64 = 30;

/// Maximum execution timeout in seconds
pub const EXEC_TIMEOUT_SECONDS_MAX: u64 = 300;

/// Default max output size in bytes (1 MiB)
pub const EXEC_OUTPUT_BYTES_MAX: u64 = 1024 * 1024;

/// Environment variable for backend selection
pub const SANDBOX_BACKEND_ENV_VAR: &str = "KELPIE_SANDBOX_BACKEND";

/// Environment variable for isolation mode selection
pub const ISOLATION_MODE_ENV_VAR: &str = "KELPIE_ISOLATION_MODE";

// =============================================================================
// SandboxBackendKind
// =============================================================================

/// Available sandbox backends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackendKind {
    /// OS process isolation (cross-platform)
    Process,
    /// libkrun VM isolation (macOS ARM64 and Linux)
    #[cfg(feature = "libkrun")]
    Libkrun,
}

impl std::fmt::Display for SandboxBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxBackendKind::Process => write!(f, "process"),
            #[cfg(feature = "libkrun")]
            SandboxBackendKind::Libkrun => write!(f, "libkrun"),
        }
    }
}

impl SandboxBackendKind {
    /// Parse from string (returns None if unrecognized)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "process" => Some(SandboxBackendKind::Process),
            #[cfg(feature = "libkrun")]
            "libkrun" | "krun" => Some(SandboxBackendKind::Libkrun),
            _ => None,
        }
    }

    /// Detect the best available backend
    #[allow(unreachable_code)]
    pub fn detect() -> Self {
        // Check environment variable first
        if let Ok(backend_str) = std::env::var(SANDBOX_BACKEND_ENV_VAR) {
            if let Some(kind) = Self::parse(&backend_str) {
                tracing::info!(backend = %kind, "Using sandbox backend from {}", SANDBOX_BACKEND_ENV_VAR);
                return kind;
            } else {
                tracing::warn!(
                    "Unknown sandbox backend '{}', falling back to process",
                    backend_str
                );
            }
        }

        // Prefer libkrun if available (works on macOS ARM64 and Linux)
        #[cfg(feature = "libkrun")]
        {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                tracing::info!("libkrun sandbox available on macOS ARM64");
                return SandboxBackendKind::Libkrun;
            }
            #[cfg(all(
                target_os = "linux",
                any(target_arch = "x86_64", target_arch = "aarch64")
            ))]
            {
                tracing::info!("libkrun sandbox available on Linux");
                return SandboxBackendKind::Libkrun;
            }
        }

        SandboxBackendKind::Process
    }
}

// =============================================================================
// SandboxProvider
// =============================================================================

/// Global sandbox provider instance
static PROVIDER: OnceCell<Arc<SandboxProvider>> = OnceCell::const_new();

/// Sandbox provider that manages backend selection and initialization
pub struct SandboxProvider {
    /// The backend kind being used
    kind: SandboxBackendKind,
    /// Isolation mode (shared or dedicated per-agent)
    isolation_mode: IsolationMode,
    /// Process sandbox manager for per-agent isolation (when using ProcessSandbox)
    process_manager: Option<Arc<AgentSandboxManager<ProcessSandboxFactory>>>,
    /// libkrun sandbox factory (only initialized when using libkrun backend)
    #[cfg(feature = "libkrun")]
    libkrun_factory: Option<Arc<LibkrunSandboxFactory>>,
    /// libkrun sandbox manager for per-agent isolation
    #[cfg(feature = "libkrun")]
    libkrun_manager: Option<Arc<AgentSandboxManager<LibkrunSandboxFactory>>>,
}

impl SandboxProvider {
    /// Initialize the global sandbox provider
    ///
    /// This should be called once during server startup.
    /// Subsequent calls return the existing provider.
    pub async fn init() -> Result<Arc<Self>, String> {
        PROVIDER
            .get_or_try_init(|| async {
                let provider = Self::new().await?;
                Ok(Arc::new(provider))
            })
            .await
            .cloned()
    }

    /// Get the global provider (must be initialized first)
    pub fn get() -> Option<Arc<Self>> {
        PROVIDER.get().cloned()
    }

    /// Detect isolation mode from environment
    fn detect_isolation_mode() -> IsolationMode {
        if let Ok(mode_str) = std::env::var(ISOLATION_MODE_ENV_VAR) {
            match mode_str.to_lowercase().as_str() {
                "dedicated" | "per-agent" => {
                    tracing::info!(
                        mode = "dedicated",
                        "Using dedicated per-agent sandbox isolation"
                    );
                    IsolationMode::Dedicated
                }
                _ => {
                    tracing::info!(mode = "shared", "Using shared sandbox pool");
                    IsolationMode::Shared
                }
            }
        } else {
            IsolationMode::Shared
        }
    }

    /// Create a new sandbox provider
    async fn new() -> Result<Self, String> {
        let kind = SandboxBackendKind::detect();
        let isolation_mode = Self::detect_isolation_mode();

        // Use a writable temp directory for sandbox workdir
        let sandbox_config = SandboxConfig::default().with_workdir("/tmp/kelpie-sandbox");
        let pool_config = PoolConfig::new(sandbox_config);

        match kind {
            SandboxBackendKind::Process => {
                tracing::info!(
                    backend = "process",
                    isolation_mode = %isolation_mode,
                    "Initializing ProcessSandbox provider"
                );

                // Create process manager for per-agent execution
                let factory = ProcessSandboxFactory::new();
                let process_manager = match isolation_mode {
                    IsolationMode::Dedicated => Some(Arc::new(AgentSandboxManager::dedicated(
                        factory,
                        pool_config,
                    ))),
                    IsolationMode::Shared => {
                        let manager =
                            AgentSandboxManager::shared(factory, pool_config).map_err(|e| {
                                format!("Failed to create shared sandbox manager: {}", e)
                            })?;
                        Some(Arc::new(manager))
                    }
                };

                Ok(Self {
                    kind,
                    isolation_mode,
                    process_manager,
                    #[cfg(feature = "libkrun")]
                    libkrun_factory: None,
                    #[cfg(feature = "libkrun")]
                    libkrun_manager: None,
                })
            }
            #[cfg(feature = "libkrun")]
            SandboxBackendKind::Libkrun => {
                tracing::info!(
                    backend = "libkrun",
                    isolation_mode = %isolation_mode,
                    "Initializing LibkrunSandbox provider"
                );

                // Initialize VM image manager for rootfs
                let image_manager = kelpie_vm::VmImageManager::new()
                    .map_err(|e| format!("Failed to create VM image manager: {}", e))?;

                // libkrun uses directory-based rootfs, not ext4 image
                let rootfs_path = image_manager
                    .libkrun_rootfs_path()
                    .map_err(|e| format!("Failed to get libkrun rootfs: {}", e))?;

                tracing::info!(
                    rootfs = ?rootfs_path,
                    "libkrun rootfs ready (libkrun uses bundled kernel)"
                );

                // Alias for the rest of the code
                let images = kelpie_vm::VmImagePaths::new(
                    std::path::PathBuf::new(), // kernel not used by libkrun
                    rootfs_path,
                );

                // Create libkrun sandbox factory with rootfs path
                let libkrun_config = LibkrunSandboxConfig::new(images.rootfs);
                let factory = LibkrunSandboxFactory::new(libkrun_config);

                // Create libkrun manager for per-agent execution
                let libkrun_manager = match isolation_mode {
                    IsolationMode::Dedicated => Some(Arc::new(AgentSandboxManager::dedicated(
                        factory.clone(),
                        pool_config,
                    ))),
                    IsolationMode::Shared => {
                        let manager = AgentSandboxManager::shared(factory.clone(), pool_config)
                            .map_err(|e| {
                                format!("Failed to create shared libkrun sandbox manager: {}", e)
                            })?;
                        Some(Arc::new(manager))
                    }
                };

                Ok(Self {
                    kind,
                    isolation_mode,
                    process_manager: None,
                    libkrun_factory: Some(Arc::new(factory)),
                    libkrun_manager,
                })
            }
        }
    }

    /// Get the backend kind
    pub fn kind(&self) -> SandboxBackendKind {
        self.kind
    }

    /// Execute a command in a sandbox
    ///
    /// Creates a new sandbox, executes the command, and cleans up.
    pub async fn exec(
        &self,
        command: &str,
        args: &[&str],
        timeout_seconds: u64,
    ) -> Result<ExecResult, String> {
        let timeout = Duration::from_secs(timeout_seconds.min(EXEC_TIMEOUT_SECONDS_MAX));

        match self.kind {
            SandboxBackendKind::Process => self.exec_process(command, args, timeout).await,
            #[cfg(feature = "libkrun")]
            SandboxBackendKind::Libkrun => self.exec_libkrun(command, args, timeout).await,
        }
    }

    /// Execute in ProcessSandbox
    async fn exec_process(
        &self,
        command: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<ExecResult, String> {
        // Use a writable temp directory for sandbox workdir
        let config = SandboxConfig::default().with_workdir("/tmp/kelpie-sandbox");
        let mut sandbox = ProcessSandbox::new(config);

        sandbox
            .start()
            .await
            .map_err(|e| format!("Failed to start process sandbox: {}", e))?;

        let exec_opts = ExecOptions::new()
            .with_timeout(timeout)
            .with_max_output(EXEC_OUTPUT_BYTES_MAX);

        let output = sandbox
            .exec(command, args, exec_opts)
            .await
            .map_err(|e| format!("Process sandbox execution failed: {}", e))?;

        let _ = sandbox.stop().await;

        Ok(ExecResult {
            stdout: output.stdout_string(),
            stderr: output.stderr_string(),
            exit_code: output.status.code,
            success: output.is_success(),
        })
    }

    /// Execute in LibkrunSandbox
    #[cfg(feature = "libkrun")]
    async fn exec_libkrun(
        &self,
        command: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<ExecResult, String> {
        use kelpie_sandbox::SandboxFactory;

        let factory = self
            .libkrun_factory
            .as_ref()
            .ok_or_else(|| "libkrun factory not initialized".to_string())?;

        // Use a writable temp directory for sandbox workdir
        let sandbox_config = SandboxConfig::default().with_workdir("/tmp/kelpie-sandbox");
        let mut sandbox = factory
            .create(sandbox_config)
            .await
            .map_err(|e| format!("Failed to create libkrun sandbox: {}", e))?;

        sandbox
            .start()
            .await
            .map_err(|e| format!("Failed to start libkrun sandbox: {}", e))?;

        let exec_opts = ExecOptions::new()
            .with_timeout(timeout)
            .with_max_output(EXEC_OUTPUT_BYTES_MAX);

        let output = sandbox
            .exec(command, args, exec_opts)
            .await
            .map_err(|e| format!("libkrun sandbox execution failed: {}", e))?;

        let _ = sandbox.stop().await;

        Ok(ExecResult {
            stdout: output.stdout_string(),
            stderr: output.stderr_string(),
            exit_code: output.status.code,
            success: output.is_success(),
        })
    }

    /// Execute a command in a sandbox for a specific agent
    ///
    /// Uses AgentSandboxManager for per-agent isolation. In dedicated mode,
    /// each agent gets its own sandbox. In shared mode, sandboxes are
    /// pooled across agents.
    pub async fn exec_for_agent(
        &self,
        agent_id: &str,
        command: &str,
        args: &[&str],
        timeout_seconds: u64,
    ) -> Result<ExecResult, String> {
        let timeout = Duration::from_secs(timeout_seconds.min(EXEC_TIMEOUT_SECONDS_MAX));
        let exec_opts = ExecOptions::new()
            .with_timeout(timeout)
            .with_max_output(EXEC_OUTPUT_BYTES_MAX);

        match self.kind {
            SandboxBackendKind::Process => {
                self.exec_for_agent_process(agent_id, command, args, exec_opts)
                    .await
            }
            #[cfg(feature = "libkrun")]
            SandboxBackendKind::Libkrun => {
                self.exec_for_agent_libkrun(agent_id, command, args, exec_opts)
                    .await
            }
        }
    }

    /// Execute in ProcessSandbox for a specific agent
    async fn exec_for_agent_process(
        &self,
        agent_id: &str,
        command: &str,
        args: &[&str],
        exec_opts: ExecOptions,
    ) -> Result<ExecResult, String> {
        let manager = self
            .process_manager
            .as_ref()
            .ok_or_else(|| "Process sandbox manager not initialized".to_string())?;

        // Acquire sandbox for this agent
        let sandbox = manager
            .acquire_for_agent(agent_id)
            .await
            .map_err(|e| format!("Failed to acquire sandbox: {}", e))?;

        // Execute command
        let output = sandbox.exec(command, args, exec_opts).await;

        // Release sandbox back to pool
        manager.release(agent_id, sandbox).await;

        // Process result
        let output = output.map_err(|e| format!("Process sandbox execution failed: {}", e))?;

        Ok(ExecResult {
            stdout: output.stdout_string(),
            stderr: output.stderr_string(),
            exit_code: output.status.code,
            success: output.is_success(),
        })
    }

    /// Execute in LibkrunSandbox for a specific agent
    #[cfg(feature = "libkrun")]
    async fn exec_for_agent_libkrun(
        &self,
        agent_id: &str,
        command: &str,
        args: &[&str],
        exec_opts: ExecOptions,
    ) -> Result<ExecResult, String> {
        let manager = self
            .libkrun_manager
            .as_ref()
            .ok_or_else(|| "libkrun sandbox manager not initialized".to_string())?;

        // Acquire sandbox for this agent
        let sandbox = manager
            .acquire_for_agent(agent_id)
            .await
            .map_err(|e| format!("Failed to acquire libkrun sandbox: {}", e))?;

        // Execute command
        let output = sandbox.exec(command, args, exec_opts).await;

        // Release sandbox back to pool
        manager.release(agent_id, sandbox).await;

        // Process result
        let output = output.map_err(|e| format!("libkrun sandbox execution failed: {}", e))?;

        Ok(ExecResult {
            stdout: output.stdout_string(),
            stderr: output.stderr_string(),
            exit_code: output.status.code,
            success: output.is_success(),
        })
    }

    /// Cleanup sandbox resources for an agent
    ///
    /// Should be called when an agent terminates to release its dedicated sandbox.
    /// In shared mode, this is a no-op.
    pub async fn cleanup_agent(&self, agent_id: &str) -> Result<(), String> {
        match self.kind {
            SandboxBackendKind::Process => {
                if let Some(manager) = &self.process_manager {
                    manager
                        .cleanup_agent(agent_id)
                        .await
                        .map_err(|e| format!("Failed to cleanup process sandbox: {}", e))?;
                }
            }
            #[cfg(feature = "libkrun")]
            SandboxBackendKind::Libkrun => {
                if let Some(manager) = &self.libkrun_manager {
                    manager
                        .cleanup_agent(agent_id)
                        .await
                        .map_err(|e| format!("Failed to cleanup libkrun sandbox: {}", e))?;
                }
            }
        }
        Ok(())
    }

    /// Get the current isolation mode
    pub fn isolation_mode(&self) -> IsolationMode {
        self.isolation_mode
    }
}

// =============================================================================
// ExecResult
// =============================================================================

/// Result of sandbox execution
#[derive(Debug, Clone)]
pub struct ExecResult {
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Exit code
    pub exit_code: i32,
    /// Whether execution succeeded (exit code 0)
    pub success: bool,
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Execute a command in a sandbox using the global provider
///
/// Convenience function that uses the global provider.
/// Must call `SandboxProvider::init()` first during server startup.
pub async fn execute_in_sandbox(
    command: &str,
    args: &[&str],
    timeout_seconds: u64,
) -> Result<ExecResult, String> {
    let provider = SandboxProvider::get().ok_or_else(|| {
        "Sandbox provider not initialized. Call SandboxProvider::init() first.".to_string()
    })?;

    provider.exec(command, args, timeout_seconds).await
}

/// Execute a command in a sandbox for a specific agent
///
/// Convenience function that routes execution through AgentSandboxManager.
/// In dedicated mode, each agent gets its own sandbox. In shared mode,
/// sandboxes are pooled across agents.
///
/// Must call `SandboxProvider::init()` first during server startup.
pub async fn execute_for_agent(
    agent_id: &str,
    command: &str,
    args: &[&str],
    timeout_seconds: u64,
) -> Result<ExecResult, String> {
    let provider = SandboxProvider::get().ok_or_else(|| {
        "Sandbox provider not initialized. Call SandboxProvider::init() first.".to_string()
    })?;

    provider
        .exec_for_agent(agent_id, command, args, timeout_seconds)
        .await
}

/// Cleanup sandbox resources for an agent using the global provider
///
/// Should be called when an agent terminates to release its dedicated sandbox.
/// In shared mode, this is a no-op.
///
/// Must call `SandboxProvider::init()` first during server startup.
pub async fn cleanup_agent_sandbox(agent_id: &str) -> Result<(), String> {
    let provider = SandboxProvider::get().ok_or_else(|| {
        "Sandbox provider not initialized. Call SandboxProvider::init() first.".to_string()
    })?;

    provider.cleanup_agent(agent_id).await
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_kind_parse() {
        assert_eq!(
            SandboxBackendKind::parse("process"),
            Some(SandboxBackendKind::Process)
        );
        #[cfg(feature = "libkrun")]
        assert_eq!(
            SandboxBackendKind::parse("libkrun"),
            Some(SandboxBackendKind::Libkrun)
        );
        assert_eq!(SandboxBackendKind::parse("unknown"), None);
    }

    #[test]
    fn test_backend_kind_display() {
        assert_eq!(SandboxBackendKind::Process.to_string(), "process");
        #[cfg(feature = "libkrun")]
        assert_eq!(SandboxBackendKind::Libkrun.to_string(), "libkrun");
    }

    #[test]
    fn test_constants_valid() {
        assert!(EXEC_TIMEOUT_SECONDS_DEFAULT <= EXEC_TIMEOUT_SECONDS_MAX);
        assert!(EXEC_OUTPUT_BYTES_MAX > 0);
    }
}
