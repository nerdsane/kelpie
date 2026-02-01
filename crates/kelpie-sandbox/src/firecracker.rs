//! Firecracker MicroVM sandbox implementation
//!
//! TigerStyle: VM-level isolation with explicit lifecycle and resource bounds.
//!
//! # Overview
//!
//! Firecracker provides strong isolation through microVMs, offering:
//! - Hardware-level isolation via KVM
//! - Minimal attack surface (simple device model)
//! - Fast boot times (~125ms)
//! - Snapshot/restore capabilities
//!
//! # Requirements
//!
//! - Linux with KVM support (`/dev/kvm`)
//! - Firecracker binary (`firecracker`)
//! - Root/sudo access (for VM management)
//! - Guest kernel image
//! - Root filesystem image
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  Kelpie Process                         │
//! │  ┌─────────────────────────────────────┐│
//! │  │  FirecrackerSandbox                 ││
//! │  │  - API socket communication         ││
//! │  │  - vsock for command execution      ││
//! │  └─────────────┬───────────────────────┘│
//! └────────────────┼────────────────────────┘
//!                  │ Unix socket (API)
//!                  ▼
//! ┌─────────────────────────────────────────┐
//! │  Firecracker Process                    │
//! │  ┌─────────────────────────────────────┐│
//! │  │  MicroVM                            ││
//! │  │  - Guest kernel                     ││
//! │  │  - Root filesystem                  ││
//! │  │  - vsock device (guest agent)       ││
//! │  └─────────────────────────────────────┘│
//! └─────────────────────────────────────────┘
//! ```

use crate::config::SandboxConfig;
use crate::error::{SandboxError, SandboxResult};
use crate::exec::{ExecOptions, ExecOutput, ExitStatus};
use crate::snapshot::Snapshot;
use crate::traits::{Sandbox, SandboxFactory, SandboxState, SandboxStats};
use async_trait::async_trait;
use kelpie_core::Runtime;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Default path to firecracker binary
pub const FIRECRACKER_BINARY_PATH_DEFAULT: &str = "/usr/bin/firecracker";

/// Default vsock CID for guest
pub const FIRECRACKER_VSOCK_CID_DEFAULT: u32 = 3;

/// Default vsock port for guest agent
pub const FIRECRACKER_VSOCK_PORT_DEFAULT: u32 = 5000;

/// Default boot time limit in milliseconds
pub const FIRECRACKER_BOOT_TIMEOUT_MS_DEFAULT: u64 = 30_000;

/// Default API socket timeout in milliseconds
pub const FIRECRACKER_API_TIMEOUT_MS_DEFAULT: u64 = 5_000;

/// Firecracker-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirecrackerConfig {
    /// Path to firecracker binary
    pub firecracker_binary: PathBuf,
    /// Path to guest kernel image
    pub kernel_image: PathBuf,
    /// Path to root filesystem image
    pub rootfs_image: PathBuf,
    /// Directory for VM runtime files (sockets, logs)
    pub runtime_dir: PathBuf,
    /// Directory for snapshots
    pub snapshot_dir: PathBuf,
    /// vsock CID for the guest
    pub vsock_cid: u32,
    /// vsock port for guest agent
    pub vsock_port: u32,
    /// Boot timeout in milliseconds
    pub boot_timeout_ms: u64,
    /// Whether to enable MMDS (Microvm Metadata Service)
    pub mmds_enabled: bool,
    /// Kernel boot arguments
    pub kernel_args: String,
}

impl Default for FirecrackerConfig {
    fn default() -> Self {
        Self {
            firecracker_binary: PathBuf::from(FIRECRACKER_BINARY_PATH_DEFAULT),
            kernel_image: PathBuf::from("/var/lib/kelpie/vmlinux"),
            rootfs_image: PathBuf::from("/var/lib/kelpie/rootfs.ext4"),
            runtime_dir: PathBuf::from("/var/run/kelpie/vms"),
            snapshot_dir: PathBuf::from("/var/lib/kelpie/snapshots"),
            vsock_cid: FIRECRACKER_VSOCK_CID_DEFAULT,
            vsock_port: FIRECRACKER_VSOCK_PORT_DEFAULT,
            boot_timeout_ms: FIRECRACKER_BOOT_TIMEOUT_MS_DEFAULT,
            mmds_enabled: true,
            kernel_args: "console=ttyS0 reboot=k panic=1 pci=off".to_string(),
        }
    }
}

impl FirecrackerConfig {
    /// Create configuration with required paths
    pub fn new(kernel_image: impl Into<PathBuf>, rootfs_image: impl Into<PathBuf>) -> Self {
        Self {
            kernel_image: kernel_image.into(),
            rootfs_image: rootfs_image.into(),
            ..Default::default()
        }
    }

    /// Set the runtime directory
    pub fn with_runtime_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.runtime_dir = dir.into();
        self
    }

    /// Set the snapshot directory
    pub fn with_snapshot_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.snapshot_dir = dir.into();
        self
    }

    /// Set kernel boot arguments
    pub fn with_kernel_args(mut self, args: impl Into<String>) -> Self {
        self.kernel_args = args.into();
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> SandboxResult<()> {
        if !self.firecracker_binary.exists() {
            return Err(SandboxError::ConfigError {
                reason: format!(
                    "Firecracker binary not found: {}",
                    self.firecracker_binary.display()
                ),
            });
        }

        if !self.kernel_image.exists() {
            return Err(SandboxError::ConfigError {
                reason: format!("Kernel image not found: {}", self.kernel_image.display()),
            });
        }

        if !self.rootfs_image.exists() {
            return Err(SandboxError::ConfigError {
                reason: format!("Rootfs image not found: {}", self.rootfs_image.display()),
            });
        }

        // Check KVM availability
        if !Path::new("/dev/kvm").exists() {
            return Err(SandboxError::ConfigError {
                reason: "KVM not available: /dev/kvm not found".to_string(),
            });
        }

        Ok(())
    }
}

/// Internal state for the Firecracker VM
struct VmState {
    /// Firecracker process
    process: Option<Child>,
    /// API socket path
    api_socket: PathBuf,
    /// vsock path (UDS path)
    vsock_uds_path: PathBuf,
    /// Current sandbox state
    state: SandboxState,
    /// Start time for uptime tracking
    start_time: Option<Instant>,
    /// Last snapshot path
    last_snapshot: Option<PathBuf>,
}

/// Firecracker MicroVM sandbox
///
/// Provides VM-level isolation using Firecracker microVMs.
/// Each sandbox runs in its own lightweight VM with dedicated
/// CPU, memory, and network resources.
pub struct FirecrackerSandbox {
    /// Unique sandbox ID
    id: String,
    /// Sandbox configuration
    config: SandboxConfig,
    /// Firecracker-specific configuration
    fc_config: FirecrackerConfig,
    /// Internal VM state
    vm: Arc<RwLock<VmState>>,
    /// Request ID counter for API (reserved for future use)
    #[allow(dead_code)]
    request_id: AtomicU64,
}

impl FirecrackerSandbox {
    /// Create a new Firecracker sandbox
    pub async fn new(config: SandboxConfig, fc_config: FirecrackerConfig) -> SandboxResult<Self> {
        fc_config.validate()?;

        let id = format!("fc-{}", Uuid::new_v4());

        // Create runtime directory for this VM
        let vm_dir = fc_config.runtime_dir.join(&id);
        tokio::fs::create_dir_all(&vm_dir).await?;

        let api_socket = vm_dir.join("firecracker.socket");
        let vsock_uds_path = vm_dir.join("vsock.socket");

        let vm = VmState {
            process: None,
            api_socket,
            vsock_uds_path,
            state: SandboxState::Stopped,
            start_time: None,
            last_snapshot: None,
        };

        Ok(Self {
            id,
            config,
            fc_config,
            vm: Arc::new(RwLock::new(vm)),
            request_id: AtomicU64::new(1),
        })
    }

    /// Get the next request ID (reserved for future use)
    #[allow(dead_code)]
    fn next_request_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Make an API request to Firecracker
    async fn api_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> SandboxResult<String> {
        let vm = self.vm.read().await;

        let mut stream =
            UnixStream::connect(&vm.api_socket)
                .await
                .map_err(|e| SandboxError::Internal {
                    message: format!("Failed to connect to API socket: {}", e),
                })?;

        // Build HTTP request
        let content_length = body.map(|b| b.len()).unwrap_or(0);
        let request = if let Some(body) = body {
            format!(
                "{} {} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 \r\n\
                 {}",
                method, path, content_length, body
            )
        } else {
            format!(
                "{} {} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 \r\n",
                method, path
            )
        };

        stream.write_all(request.as_bytes()).await?;

        // Read response
        let mut reader = BufReader::new(stream);
        let mut response = String::new();

        // Read status line
        reader.read_line(&mut response).await?;

        // Read headers until empty line
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            if line == "\r\n" || line.is_empty() {
                break;
            }
            response.push_str(&line);
        }

        // Read body (simple approach - read remaining)
        let mut body_buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut body_buf).await?;

        Ok(String::from_utf8_lossy(&body_buf).to_string())
    }

    /// Configure the VM boot source
    async fn configure_boot_source(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "kernel_image_path": self.fc_config.kernel_image.to_string_lossy(),
            "boot_args": self.fc_config.kernel_args
        });

        self.api_request("PUT", "/boot-source", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Configure the root drive
    async fn configure_drives(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "drive_id": "rootfs",
            "path_on_host": self.fc_config.rootfs_image.to_string_lossy(),
            "is_root_device": true,
            "is_read_only": false
        });

        self.api_request("PUT", "/drives/rootfs", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Configure the machine resources
    async fn configure_machine(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "vcpu_count": self.config.limits.vcpu_count,
            "mem_size_mib": self.config.limits.memory_bytes_max / (1024 * 1024)
        });

        self.api_request("PUT", "/machine-config", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Configure vsock device
    async fn configure_vsock(&self) -> SandboxResult<()> {
        let vm = self.vm.read().await;

        let body = serde_json::json!({
            "vsock_id": "vsock0",
            "guest_cid": self.fc_config.vsock_cid,
            "uds_path": vm.vsock_uds_path.to_string_lossy()
        });

        drop(vm);

        self.api_request("PUT", "/vsock", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Start the VM instance
    async fn start_vm_instance(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "action_type": "InstanceStart"
        });

        self.api_request("PUT", "/actions", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Execute a command via vsock
    async fn exec_via_vsock(
        &self,
        command: &str,
        args: &[&str],
        options: &ExecOptions,
    ) -> SandboxResult<ExecOutput> {
        let vm = self.vm.read().await;

        // Connect to vsock UDS
        let mut stream =
            UnixStream::connect(&vm.vsock_uds_path)
                .await
                .map_err(|e| SandboxError::Internal {
                    message: format!("Failed to connect to vsock: {}", e),
                })?;

        // Build command request
        let request = serde_json::json!({
            "type": "exec",
            "command": command,
            "args": args,
            "env": options.env,
            "cwd": options.workdir,
            "timeout_ms": options.timeout_ms
        });

        // Send request
        let request_bytes = serde_json::to_vec(&request).map_err(|e| SandboxError::Internal {
            message: format!("Failed to serialize request: {}", e),
        })?;
        let len_bytes = (request_bytes.len() as u32).to_be_bytes();
        stream.write_all(&len_bytes).await?;
        stream.write_all(&request_bytes).await?;

        // Read response length
        let mut len_buf = [0u8; 4];
        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut len_buf).await?;
        let response_len = u32::from_be_bytes(len_buf) as usize;

        // Read response
        let mut response_buf = vec![0u8; response_len];
        tokio::io::AsyncReadExt::read_exact(&mut stream, &mut response_buf).await?;

        let response: serde_json::Value =
            serde_json::from_slice(&response_buf).map_err(|e| SandboxError::Internal {
                message: format!("Failed to deserialize response: {}", e),
            })?;

        // Parse response
        let exit_code = response["exit_code"].as_i64().unwrap_or(-1) as i32;
        let stdout = response["stdout"].as_str().unwrap_or("");
        let stderr = response["stderr"].as_str().unwrap_or("");
        let duration_ms = response["duration_ms"].as_u64().unwrap_or(0);

        Ok(ExecOutput {
            status: ExitStatus::with_code(exit_code),
            stdout: bytes::Bytes::from(stdout.to_string()),
            stderr: bytes::Bytes::from(stderr.to_string()),
            duration_ms,
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }

    /// Pause the VM
    async fn pause_vm(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "state": "Paused"
        });

        self.api_request("PATCH", "/vm", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Resume the VM
    async fn resume_vm(&self) -> SandboxResult<()> {
        let body = serde_json::json!({
            "state": "Resumed"
        });

        self.api_request("PATCH", "/vm", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Create a VM snapshot
    async fn create_snapshot(&self, snapshot_path: &Path) -> SandboxResult<()> {
        let mem_path = snapshot_path.with_extension("mem");
        let state_path = snapshot_path.with_extension("state");

        let body = serde_json::json!({
            "snapshot_type": "Full",
            "snapshot_path": state_path.to_string_lossy(),
            "mem_file_path": mem_path.to_string_lossy()
        });

        self.api_request("PUT", "/snapshot/create", Some(&body.to_string()))
            .await?;

        Ok(())
    }

    /// Restore from a snapshot
    async fn restore_from_snapshot(&self, snapshot_path: &Path) -> SandboxResult<()> {
        let mem_path = snapshot_path.with_extension("mem");
        let state_path = snapshot_path.with_extension("state");

        let body = serde_json::json!({
            "snapshot_path": state_path.to_string_lossy(),
            "mem_backend": {
                "backend_type": "File",
                "backend_path": mem_path.to_string_lossy()
            },
            "enable_diff_snapshots": false
        });

        self.api_request("PUT", "/snapshot/load", Some(&body.to_string()))
            .await?;

        Ok(())
    }
}

#[async_trait]
impl Sandbox for FirecrackerSandbox {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> SandboxState {
        // This needs to be sync, so we can't await
        // In practice, we'd cache the state
        SandboxState::Running // Placeholder
    }

    fn config(&self) -> &SandboxConfig {
        &self.config
    }

    async fn start(&mut self) -> SandboxResult<()> {
        let mut vm = self.vm.write().await;

        if !vm.state.can_start() && vm.state != SandboxState::Stopped {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: vm.state.to_string(),
                expected: "stopped".to_string(),
            });
        }

        vm.state = SandboxState::Creating;

        // Spawn Firecracker process
        let mut cmd = Command::new(&self.fc_config.firecracker_binary);
        cmd.arg("--api-sock")
            .arg(&vm.api_socket)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| SandboxError::Internal {
            message: format!("Failed to spawn Firecracker: {}", e),
        })?;

        vm.process = Some(child);

        // Wait for API socket to be available
        for _ in 0..50 {
            if vm.api_socket.exists() {
                break;
            }
            kelpie_core::current_runtime()
                .sleep(tokio::time::Duration::from_millis(100))
                .await;
        }

        if !vm.api_socket.exists() {
            return Err(SandboxError::Internal {
                message: "API socket not created within timeout".to_string(),
            });
        }

        drop(vm); // Release lock for API calls

        // Configure VM
        self.configure_boot_source().await?;
        self.configure_drives().await?;
        self.configure_machine().await?;
        self.configure_vsock().await?;

        // Start the VM
        self.start_vm_instance().await?;

        let mut vm = self.vm.write().await;
        vm.state = SandboxState::Running;
        vm.start_time = Some(Instant::now());

        info!(sandbox_id = %self.id, "Firecracker sandbox started");

        Ok(())
    }

    async fn stop(&mut self) -> SandboxResult<()> {
        let mut vm = self.vm.write().await;

        if !vm.state.can_stop() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: vm.state.to_string(),
                expected: "running or paused".to_string(),
            });
        }

        // Kill the Firecracker process
        if let Some(ref mut process) = vm.process {
            process.kill().await.ok();
        }

        vm.process = None;
        vm.state = SandboxState::Stopped;
        vm.start_time = None;

        // Clean up socket
        if vm.api_socket.exists() {
            tokio::fs::remove_file(&vm.api_socket).await.ok();
        }

        info!(sandbox_id = %self.id, "Firecracker sandbox stopped");

        Ok(())
    }

    async fn pause(&mut self) -> SandboxResult<()> {
        {
            let vm = self.vm.read().await;
            if !vm.state.can_pause() {
                return Err(SandboxError::InvalidState {
                    sandbox_id: self.id.clone(),
                    current: vm.state.to_string(),
                    expected: "running".to_string(),
                });
            }
        }

        self.pause_vm().await?;

        let mut vm = self.vm.write().await;
        vm.state = SandboxState::Paused;

        debug!(sandbox_id = %self.id, "Firecracker sandbox paused");

        Ok(())
    }

    async fn resume(&mut self) -> SandboxResult<()> {
        {
            let vm = self.vm.read().await;
            if !vm.state.can_resume() {
                return Err(SandboxError::InvalidState {
                    sandbox_id: self.id.clone(),
                    current: vm.state.to_string(),
                    expected: "paused".to_string(),
                });
            }
        }

        self.resume_vm().await?;

        let mut vm = self.vm.write().await;
        vm.state = SandboxState::Running;

        debug!(sandbox_id = %self.id, "Firecracker sandbox resumed");

        Ok(())
    }

    async fn exec(
        &self,
        command: &str,
        args: &[&str],
        options: ExecOptions,
    ) -> SandboxResult<ExecOutput> {
        let vm = self.vm.read().await;

        if !vm.state.can_exec() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: vm.state.to_string(),
                expected: "running".to_string(),
            });
        }

        drop(vm);

        self.exec_via_vsock(command, args, &options).await
    }

    async fn snapshot(&self) -> SandboxResult<Snapshot> {
        let vm = self.vm.read().await;

        if !vm.state.can_snapshot() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: vm.state.to_string(),
                expected: "running or paused".to_string(),
            });
        }

        drop(vm);

        // First pause if running
        let was_running = {
            let vm = self.vm.read().await;
            vm.state == SandboxState::Running
        };

        if was_running {
            self.pause_vm().await?;
        }

        // Create snapshot
        let snapshot_id = Uuid::new_v4().to_string();
        let snapshot_path = self.fc_config.snapshot_dir.join(&snapshot_id);

        tokio::fs::create_dir_all(&self.fc_config.snapshot_dir).await?;

        self.create_snapshot(&snapshot_path).await?;

        // Resume if was running
        if was_running {
            self.resume_vm().await?;
        }

        // Update last snapshot
        {
            let mut vm = self.vm.write().await;
            vm.last_snapshot = Some(snapshot_path.clone());
        }

        // Create snapshot object - Firecracker creates full Teleport snapshots
        let snapshot = Snapshot::teleport(&self.id)
            .with_disk_reference(snapshot_path.to_string_lossy().to_string());

        info!(sandbox_id = %self.id, snapshot_id = %snapshot.id(), "Created snapshot");

        Ok(snapshot)
    }

    async fn restore(&mut self, snapshot: &Snapshot) -> SandboxResult<()> {
        let disk_ref =
            snapshot
                .disk_reference
                .as_ref()
                .ok_or_else(|| SandboxError::RestoreFailed {
                    sandbox_id: self.id.clone(),
                    reason: "Snapshot has no disk reference".to_string(),
                })?;

        let snapshot_path = PathBuf::from(disk_ref);

        if !snapshot_path.exists() {
            return Err(SandboxError::RestoreFailed {
                sandbox_id: self.id.clone(),
                reason: format!("Snapshot file not found: {}", snapshot_path.display()),
            });
        }

        // Stop if running
        let vm_state = {
            let vm = self.vm.read().await;
            vm.state
        };

        if vm_state != SandboxState::Stopped {
            self.stop().await?;
        }

        // Start fresh process and restore
        self.start().await?;

        // Wait a bit then restore
        kelpie_core::current_runtime()
            .sleep(tokio::time::Duration::from_millis(500))
            .await;

        self.restore_from_snapshot(&snapshot_path).await?;

        info!(sandbox_id = %self.id, snapshot_id = %snapshot.id(), "Restored from snapshot");

        Ok(())
    }

    async fn destroy(&mut self) -> SandboxResult<()> {
        let mut vm = self.vm.write().await;

        if !vm.state.can_destroy() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: vm.state.to_string(),
                expected: "not destroying".to_string(),
            });
        }

        vm.state = SandboxState::Destroying;

        // Kill process
        if let Some(ref mut process) = vm.process {
            process.kill().await.ok();
        }
        vm.process = None;

        // Clean up runtime directory
        let vm_dir = self.fc_config.runtime_dir.join(&self.id);
        if vm_dir.exists() {
            tokio::fs::remove_dir_all(&vm_dir).await.ok();
        }

        info!(sandbox_id = %self.id, "Firecracker sandbox destroyed");

        Ok(())
    }

    async fn health_check(&self) -> SandboxResult<bool> {
        let vm = self.vm.read().await;

        if vm.state != SandboxState::Running {
            return Ok(false);
        }

        // Check process is still alive
        if let Some(ref process) = vm.process {
            if process.id().is_none() {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }

        // Try a simple exec
        drop(vm);

        match self
            .exec_via_vsock("echo", &["health"], &ExecOptions::default())
            .await
        {
            Ok(output) => Ok(output.status.is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn stats(&self) -> SandboxResult<SandboxStats> {
        let vm = self.vm.read().await;

        let uptime_ms = vm
            .start_time
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        // In a real implementation, we'd query actual resource usage
        // from Firecracker's metrics endpoint
        Ok(SandboxStats {
            memory_bytes_used: 0, // Would query /metrics
            cpu_percent: 0.0,
            disk_bytes_used: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            uptime_ms,
        })
    }
}

impl Drop for FirecrackerSandbox {
    fn drop(&mut self) {
        // Clean up in a blocking way if we have a running process
        // In practice, you'd want to ensure proper cleanup
        warn!(sandbox_id = %self.id, "FirecrackerSandbox dropped - ensure destroy() was called");
    }
}

/// Factory for creating Firecracker sandboxes
pub struct FirecrackerSandboxFactory {
    fc_config: FirecrackerConfig,
}

impl FirecrackerSandboxFactory {
    /// Create a new factory with the given configuration
    pub fn new(fc_config: FirecrackerConfig) -> Self {
        Self { fc_config }
    }
}

#[async_trait]
impl SandboxFactory for FirecrackerSandboxFactory {
    type Sandbox = FirecrackerSandbox;

    async fn create(&self, config: SandboxConfig) -> SandboxResult<Self::Sandbox> {
        FirecrackerSandbox::new(config, self.fc_config.clone()).await
    }

    async fn create_from_snapshot(
        &self,
        config: SandboxConfig,
        snapshot: &Snapshot,
    ) -> SandboxResult<Self::Sandbox> {
        let mut sandbox = FirecrackerSandbox::new(config, self.fc_config.clone()).await?;
        sandbox.restore(snapshot).await?;
        Ok(sandbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_firecracker_config_default() {
        let config = FirecrackerConfig::default();
        assert_eq!(config.vsock_cid, FIRECRACKER_VSOCK_CID_DEFAULT);
        assert_eq!(config.vsock_port, FIRECRACKER_VSOCK_PORT_DEFAULT);
    }

    #[test]
    fn test_firecracker_config_builder() {
        let config = FirecrackerConfig::new("/path/to/kernel", "/path/to/rootfs")
            .with_runtime_dir("/var/run/test")
            .with_kernel_args("custom args");

        assert_eq!(config.kernel_image, PathBuf::from("/path/to/kernel"));
        assert_eq!(config.rootfs_image, PathBuf::from("/path/to/rootfs"));
        assert_eq!(config.runtime_dir, PathBuf::from("/var/run/test"));
        assert_eq!(config.kernel_args, "custom args");
    }

    #[test]
    fn test_firecracker_config_validation_missing_binary() {
        let config = FirecrackerConfig {
            firecracker_binary: PathBuf::from("/nonexistent/firecracker"),
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
    }
}
