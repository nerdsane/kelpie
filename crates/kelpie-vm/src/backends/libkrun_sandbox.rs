//! LibkrunSandbox - Sandbox trait adapter for libkrun backend
//!
//! TigerStyle: Adapts LibkrunVm (VmInstance) to Sandbox trait for unified sandbox management.
//!
//! This adapter allows libkrun VMs to be used with the AgentSandboxManager and SandboxPool
//! from kelpie-sandbox, providing hardware isolation via libkrun (HVF on macOS, KVM on Linux).

use crate::backends::libkrun::{LibkrunConfig, LibkrunVm, LIBKRUN_VSOCK_PORT_DEFAULT};
use crate::error::VmError;
use crate::traits::{ExecOptions as VmExecOptions, VmInstance, VmState};
use crate::VmConfig;
use async_trait::async_trait;
use kelpie_sandbox::{
    ExecOptions as SandboxExecOptions, ExecOutput as SandboxExecOutput, ExitStatus, ResourceLimits,
    Sandbox, SandboxConfig, SandboxError, SandboxFactory, SandboxResult, SandboxState,
    SandboxStats, Snapshot,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;
use uuid::Uuid;

// ============================================================================
// Constants (TigerStyle: explicit names with units)
// ============================================================================

/// Default memory allocation for libkrun sandbox in MiB
pub const LIBKRUN_SANDBOX_MEMORY_MIB_DEFAULT: u64 = 512;

/// Default vCPU count for libkrun sandbox
pub const LIBKRUN_SANDBOX_VCPU_COUNT_DEFAULT: u32 = 2;

// ============================================================================
// LibkrunSandboxConfig
// ============================================================================

/// Configuration for libkrun sandbox instances
#[derive(Debug, Clone)]
pub struct LibkrunSandboxConfig {
    /// Path to the root filesystem
    pub rootfs_path: PathBuf,
    /// Number of vCPUs
    pub vcpu_count: u32,
    /// Memory in MiB
    pub memory_mib: u64,
    /// Vsock port for guest agent
    pub vsock_port: u32,
    /// Directory for Unix sockets
    pub socket_dir: PathBuf,
    /// Whether root filesystem is read-only
    pub rootfs_readonly: bool,
    /// Enable debug logging
    pub debug: bool,
}

impl Default for LibkrunSandboxConfig {
    fn default() -> Self {
        Self {
            rootfs_path: PathBuf::from("/usr/share/kelpie/rootfs-aarch64.ext4"),
            vcpu_count: LIBKRUN_SANDBOX_VCPU_COUNT_DEFAULT,
            memory_mib: LIBKRUN_SANDBOX_MEMORY_MIB_DEFAULT,
            vsock_port: LIBKRUN_VSOCK_PORT_DEFAULT,
            socket_dir: std::env::temp_dir().join("kelpie-libkrun"),
            rootfs_readonly: false,
            debug: false,
        }
    }
}

impl LibkrunSandboxConfig {
    /// Create a new libkrun sandbox configuration
    pub fn new(rootfs_path: PathBuf) -> Self {
        Self {
            rootfs_path,
            ..Default::default()
        }
    }

    /// Set the number of vCPUs
    pub fn with_vcpu_count(mut self, count: u32) -> Self {
        assert!(count > 0, "vcpu_count must be positive");
        assert!(count <= 64, "vcpu_count must not exceed 64");
        self.vcpu_count = count;
        self
    }

    /// Set memory allocation in MiB
    pub fn with_memory_mib(mut self, mib: u64) -> Self {
        assert!(mib >= 128, "memory_mib must be at least 128");
        assert!(mib <= 65536, "memory_mib must not exceed 65536");
        self.memory_mib = mib;
        self
    }

    /// Set the vsock port
    pub fn with_vsock_port(mut self, port: u32) -> Self {
        self.vsock_port = port;
        self
    }

    /// Set the socket directory
    pub fn with_socket_dir(mut self, dir: PathBuf) -> Self {
        self.socket_dir = dir;
        self
    }

    /// Set root filesystem to read-only
    pub fn with_rootfs_readonly(mut self, readonly: bool) -> Self {
        self.rootfs_readonly = readonly;
        self
    }

    /// Enable debug logging
    pub fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }
}

// ============================================================================
// LibkrunSandbox
// ============================================================================

/// libkrun sandbox - wraps LibkrunVm to implement the Sandbox trait
pub struct LibkrunSandbox {
    /// Unique sandbox identifier
    id: String,
    /// The underlying libkrun VM
    vm: Mutex<LibkrunVm>,
    /// Configuration stored as SandboxConfig for trait compatibility
    sandbox_config: SandboxConfig,
    /// libkrun-specific configuration
    libkrun_config: LibkrunSandboxConfig,
    /// Current state (cached)
    state: std::sync::Mutex<SandboxState>,
    /// Creation timestamp
    created_at: Instant,
    /// Execution counter
    exec_count: AtomicU64,
}

impl std::fmt::Debug for LibkrunSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibkrunSandbox")
            .field("id", &self.id)
            .field("libkrun_config", &self.libkrun_config)
            .field("exec_count", &self.exec_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl LibkrunSandbox {
    /// Create a new libkrun sandbox
    pub async fn new(config: LibkrunSandboxConfig) -> SandboxResult<Self> {
        // Build VmConfig from LibkrunSandboxConfig
        // Note: libkrun uses its bundled kernel, so no kernel_image_path needed
        let vm_config = VmConfig {
            kernel_image_path: None, // libkrun bundles kernel via libkrunfw
            initrd_path: None,
            root_disk_path: config.rootfs_path.to_string_lossy().to_string(),
            root_disk_readonly: config.rootfs_readonly,
            vcpu_count: config.vcpu_count,
            memory_mib: config.memory_mib as u32,
            kernel_args: None, // libkrun handles kernel args
            ..Default::default()
        };

        let libkrun_config_inner = LibkrunConfig {
            vsock_port: config.vsock_port,
            socket_dir: config.socket_dir.clone(),
            debug: config.debug,
        };

        let vm = LibkrunVm::new(vm_config, libkrun_config_inner)
            .await
            .map_err(vm_error_to_sandbox_error)?;
        let id = format!("libkrun-sandbox-{}", Uuid::new_v4());

        // Create SandboxConfig for trait compatibility
        let sandbox_config = SandboxConfig {
            limits: ResourceLimits {
                memory_bytes_max: config.memory_mib * 1024 * 1024,
                vcpu_count: config.vcpu_count,
                disk_bytes_max: 10 * 1024 * 1024 * 1024, // 10GB default
                exec_timeout_ms: 60_000,
                network_bandwidth_bytes_per_sec: 0,
                network_enabled: false,
            },
            workdir: "/".to_string(),
            env: Vec::new(),
            idle_timeout_ms: 5 * 60 * 1000,
            debug: config.debug,
            image: Some(config.rootfs_path.to_string_lossy().to_string()),
        };

        Ok(Self {
            id,
            vm: Mutex::new(vm),
            sandbox_config,
            libkrun_config: config,
            state: std::sync::Mutex::new(SandboxState::Stopped),
            created_at: Instant::now(),
            exec_count: AtomicU64::new(0),
        })
    }

    fn set_state(&self, state: SandboxState) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
    }
}

#[async_trait]
impl Sandbox for LibkrunSandbox {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> SandboxState {
        self.state
            .lock()
            .map(|s| *s)
            .unwrap_or(SandboxState::Failed)
    }

    fn config(&self) -> &SandboxConfig {
        &self.sandbox_config
    }

    async fn start(&mut self) -> SandboxResult<()> {
        let mut vm = self.vm.lock().await;
        vm.start().await.map_err(vm_error_to_sandbox_error)?;
        self.set_state(SandboxState::Running);
        Ok(())
    }

    async fn stop(&mut self) -> SandboxResult<()> {
        let mut vm = self.vm.lock().await;
        vm.stop().await.map_err(vm_error_to_sandbox_error)?;
        self.set_state(SandboxState::Stopped);
        Ok(())
    }

    async fn pause(&mut self) -> SandboxResult<()> {
        // libkrun does not support pause
        Err(SandboxError::Internal {
            message: "libkrun does not support pause".to_string(),
        })
    }

    async fn resume(&mut self) -> SandboxResult<()> {
        // libkrun does not support resume
        Err(SandboxError::Internal {
            message: "libkrun does not support resume".to_string(),
        })
    }

    async fn exec(
        &self,
        cmd: &str,
        args: &[&str],
        options: SandboxExecOptions,
    ) -> SandboxResult<SandboxExecOutput> {
        self.exec_count.fetch_add(1, Ordering::Relaxed);

        let vm = self.vm.lock().await;
        let vm_options = sandbox_exec_options_to_vm(options);

        let vm_output = vm
            .exec_with_options(cmd, args, vm_options)
            .await
            .map_err(|e| vm_error_to_sandbox_error_with_cmd(e, cmd))?;

        Ok(vm_exec_output_to_sandbox(vm_output))
    }

    async fn snapshot(&self) -> SandboxResult<Snapshot> {
        // libkrun does not support snapshots
        Err(SandboxError::SnapshotFailed {
            sandbox_id: self.id.clone(),
            reason: "libkrun does not support snapshots".to_string(),
        })
    }

    async fn restore(&mut self, _snapshot: &Snapshot) -> SandboxResult<()> {
        // libkrun does not support restore
        Err(SandboxError::RestoreFailed {
            sandbox_id: self.id.clone(),
            reason: "libkrun does not support restore".to_string(),
        })
    }

    async fn destroy(&mut self) -> SandboxResult<()> {
        self.set_state(SandboxState::Destroying);
        let mut vm = self.vm.lock().await;
        // Stop the VM if running
        if matches!(vm.state(), VmState::Running) {
            let _ = vm.stop().await;
        }
        // VM will be cleaned up when dropped
        self.set_state(SandboxState::Stopped);
        Ok(())
    }

    async fn health_check(&self) -> SandboxResult<bool> {
        let vm = self.vm.lock().await;
        // Simple health check: try to execute a trivial command
        match vm.exec("true", &[]).await {
            Ok(output) => Ok(output.exit_code == 0),
            Err(_) => Ok(false),
        }
    }

    async fn stats(&self) -> SandboxResult<SandboxStats> {
        let uptime_ms = self.created_at.elapsed().as_millis() as u64;

        Ok(SandboxStats {
            memory_bytes_used: self.libkrun_config.memory_mib * 1024 * 1024,
            cpu_percent: 0.0, // libkrun doesn't expose this directly
            disk_bytes_used: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            uptime_ms,
        })
    }
}

// ============================================================================
// LibkrunSandboxFactory
// ============================================================================

/// Factory for creating libkrun sandboxes
#[derive(Debug, Clone)]
pub struct LibkrunSandboxFactory {
    config: LibkrunSandboxConfig,
}

impl LibkrunSandboxFactory {
    /// Create a new libkrun sandbox factory
    pub fn new(config: LibkrunSandboxConfig) -> Self {
        Self { config }
    }

    /// Create with rootfs path
    pub fn with_rootfs(rootfs_path: PathBuf) -> Self {
        Self::new(LibkrunSandboxConfig::new(rootfs_path))
    }
}

impl Default for LibkrunSandboxFactory {
    fn default() -> Self {
        Self::new(LibkrunSandboxConfig::default())
    }
}

#[async_trait]
impl SandboxFactory for LibkrunSandboxFactory {
    type Sandbox = LibkrunSandbox;

    async fn create(&self, _config: SandboxConfig) -> SandboxResult<Self::Sandbox> {
        // Note: We use our LibkrunSandboxConfig, not the passed SandboxConfig
        // The SandboxConfig is for generic sandbox configuration, but libkrun needs
        // specific VM parameters (rootfs, etc.)
        LibkrunSandbox::new(self.config.clone()).await
    }

    async fn create_from_snapshot(
        &self,
        _config: SandboxConfig,
        _snapshot: &Snapshot,
    ) -> SandboxResult<Self::Sandbox> {
        // libkrun does not support snapshots
        Err(SandboxError::RestoreFailed {
            sandbox_id: "unknown".to_string(),
            reason: "libkrun does not support creating sandboxes from snapshots".to_string(),
        })
    }
}

// ============================================================================
// Type Conversions
// ============================================================================

/// Convert VmError to SandboxError with command context
fn vm_error_to_sandbox_error_with_cmd(error: VmError, cmd: &str) -> SandboxError {
    match error {
        VmError::ExecFailed { reason } => SandboxError::ExecFailed {
            command: cmd.to_string(),
            reason,
        },
        VmError::ExecTimeout { timeout_ms } => SandboxError::ExecTimeout {
            command: cmd.to_string(),
            timeout_ms,
        },
        _ => vm_error_to_sandbox_error(error),
    }
}

/// Convert VmError to SandboxError
fn vm_error_to_sandbox_error(error: VmError) -> SandboxError {
    match error {
        VmError::CreateFailed { reason } => SandboxError::ConfigError { reason },
        VmError::BootFailed { reason } => SandboxError::Internal {
            message: format!("boot failed: {}", reason),
        },
        VmError::BootTimeout { timeout_ms } => SandboxError::Internal {
            message: format!("boot timed out after {}ms", timeout_ms),
        },
        VmError::AlreadyRunning => SandboxError::InvalidState {
            sandbox_id: "unknown".to_string(),
            current: "running".to_string(),
            expected: "stopped".to_string(),
        },
        VmError::NotRunning { state } => SandboxError::InvalidState {
            sandbox_id: "unknown".to_string(),
            current: state,
            expected: "running".to_string(),
        },
        VmError::ExecFailed { reason } => SandboxError::ExecFailed {
            command: "unknown".to_string(),
            reason,
        },
        VmError::ExecTimeout { timeout_ms } => SandboxError::ExecTimeout {
            command: "unknown".to_string(),
            timeout_ms,
        },
        VmError::SnapshotFailed { reason } => SandboxError::SnapshotFailed {
            sandbox_id: "unknown".to_string(),
            reason,
        },
        VmError::SnapshotCorrupted => SandboxError::SnapshotFailed {
            sandbox_id: "unknown".to_string(),
            reason: "snapshot corrupted".to_string(),
        },
        VmError::RestoreFailed { reason } => SandboxError::RestoreFailed {
            sandbox_id: "unknown".to_string(),
            reason,
        },
        VmError::PauseFailed { reason } => SandboxError::Internal {
            message: format!("pause failed: {}", reason),
        },
        VmError::ResumeFailed { reason } => SandboxError::Internal {
            message: format!("resume failed: {}", reason),
        },
        VmError::Crashed { reason } => SandboxError::Internal {
            message: format!("vm crashed: {}", reason),
        },
        VmError::ConfigInvalid { reason } => SandboxError::ConfigError { reason },
        VmError::RootDiskNotFound { path } => SandboxError::ConfigError {
            reason: format!("rootfs not found: {}", path),
        },
        VmError::ContextCreationFailed { reason } => SandboxError::Internal {
            message: format!("context creation failed: {}", reason),
        },
        VmError::ConfigurationFailed { reason } => SandboxError::ConfigError { reason },
        _ => SandboxError::Internal {
            message: format!("vm error: {:?}", error),
        },
    }
}

/// Convert SandboxExecOptions to VmExecOptions
fn sandbox_exec_options_to_vm(options: SandboxExecOptions) -> VmExecOptions {
    VmExecOptions {
        timeout_ms: options.timeout_ms,
        workdir: options.workdir,
        env: options.env,
        stdin: options.stdin,
    }
}

/// Convert VmExecOutput to SandboxExecOutput
fn vm_exec_output_to_sandbox(output: crate::traits::ExecOutput) -> SandboxExecOutput {
    use std::cmp::Ordering;

    let status = match output.exit_code.cmp(&0) {
        Ordering::Equal => ExitStatus::success(),
        Ordering::Less => ExitStatus::with_signal(-output.exit_code),
        Ordering::Greater => ExitStatus::with_code(output.exit_code),
    };

    SandboxExecOutput::new(
        status,
        output.stdout,
        output.stderr,
        0, // VmExecOutput doesn't track duration
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_libkrun_sandbox_config_defaults() {
        let config = LibkrunSandboxConfig::default();
        assert_eq!(config.vcpu_count, LIBKRUN_SANDBOX_VCPU_COUNT_DEFAULT);
        assert_eq!(config.memory_mib, LIBKRUN_SANDBOX_MEMORY_MIB_DEFAULT);
        assert_eq!(config.vsock_port, LIBKRUN_VSOCK_PORT_DEFAULT);
    }

    #[test]
    fn test_libkrun_sandbox_config_builder() {
        let config = LibkrunSandboxConfig::default()
            .with_vcpu_count(4)
            .with_memory_mib(1024)
            .with_vsock_port(9002)
            .with_rootfs_readonly(true);

        assert_eq!(config.vcpu_count, 4);
        assert_eq!(config.memory_mib, 1024);
        assert_eq!(config.vsock_port, 9002);
        assert!(config.rootfs_readonly);
    }

    #[test]
    #[should_panic(expected = "vcpu_count must be positive")]
    fn test_libkrun_sandbox_config_invalid_vcpu() {
        LibkrunSandboxConfig::default().with_vcpu_count(0);
    }

    #[test]
    #[should_panic(expected = "memory_mib must be at least 128")]
    fn test_libkrun_sandbox_config_invalid_memory() {
        LibkrunSandboxConfig::default().with_memory_mib(64);
    }

    #[test]
    fn test_exit_status_conversion() {
        // Success case
        let output = crate::traits::ExecOutput::new(0, vec![], vec![]);
        let sandbox_output = vm_exec_output_to_sandbox(output);
        assert!(sandbox_output.status.is_success());

        // Error code case
        let output = crate::traits::ExecOutput::new(1, vec![], vec![]);
        let sandbox_output = vm_exec_output_to_sandbox(output);
        assert!(!sandbox_output.status.is_success());
        assert_eq!(sandbox_output.status.code, 1);

        // Signal case (negative exit code)
        let output = crate::traits::ExecOutput::new(-9, vec![], vec![]);
        let sandbox_output = vm_exec_output_to_sandbox(output);
        assert!(sandbox_output.status.is_signal());
        assert_eq!(sandbox_output.status.signal, Some(9));
    }
}
