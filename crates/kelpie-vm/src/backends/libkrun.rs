//! libkrun backend for hardware VM isolation
//!
//! TigerStyle: Explicit lifecycle, vsock exec, manual FFI bindings.
//!
//! libkrun uses Hypervisor.framework (macOS) or KVM (Linux) for hardware virtualization.
//! Unlike Apple VZ, libkrun bundles its own optimized kernel via libkrunfw, so no
//! kernel image management is required.
//!
//! Execution model:
//! - `krun_start_enter()` runs a single command and blocks until completion
//! - For multi-command sessions, we run a guest agent and use vsock for communication
//! - The guest agent (kelpie-guest) listens on vsock port 9001 for JSON-RPC commands

use async_trait::async_trait;
use serde_json::Value;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{VmError, VmResult};
use crate::snapshot::VmSnapshot;
use crate::traits::{
    ExecOptions as VmExecOptions, ExecOutput as VmExecOutput, VmFactory, VmInstance, VmState,
};
use crate::{VmConfig, VM_EXEC_TIMEOUT_MS_DEFAULT};
use kelpie_core::Runtime;

// ============================================================================
// libkrun FFI bindings (manual, to avoid bindgen/libclang dependency)
// ============================================================================

#[link(name = "krun")]
extern "C" {
    /// Create a new libkrun context. Returns context ID or negative error code.
    fn krun_create_ctx() -> i32;

    /// Free a libkrun context.
    fn krun_free_ctx(ctx_id: u32) -> i32;

    /// Set VM configuration (vCPUs and memory).
    fn krun_set_vm_config(ctx_id: u32, num_vcpus: u8, ram_mib: u32) -> i32;

    /// Set the root filesystem path (directory).
    fn krun_set_root(ctx_id: u32, root_path: *const c_char) -> i32;

    /// Add a vsock port with a Unix socket path. listen=true creates server socket.
    fn krun_add_vsock_port2(ctx_id: u32, port: u32, filepath: *const c_char, listen: bool) -> i32;

    /// Set the executable to run in the VM.
    fn krun_set_exec(
        ctx_id: u32,
        exec_path: *const c_char,
        argv: *const *const c_char,
        envp: *const *const c_char,
    ) -> i32;

    /// Set the working directory in the VM.
    fn krun_set_workdir(ctx_id: u32, workdir_path: *const c_char) -> i32;

    /// Start the VM and enter (blocks until VM exits).
    fn krun_start_enter(ctx_id: u32) -> i32;
}

// ============================================================================
// Constants
// ============================================================================

/// Default vsock port for Kelpie guest agent (matches guest agent default)
pub const LIBKRUN_VSOCK_PORT_DEFAULT: u32 = 9001;

/// Default memory allocation in MiB
pub const LIBKRUN_MEMORY_MIB_DEFAULT: u32 = 512;

/// Default vCPU count
pub const LIBKRUN_VCPU_COUNT_DEFAULT: u32 = 2;

/// Maximum retry count for vsock connection
const VSOCK_CONNECT_RETRY_COUNT_MAX: u32 = 30;

/// Retry delay for vsock connection
const VSOCK_CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// Maximum vsock response size in bytes (16 MiB)
const VSOCK_RESPONSE_SIZE_BYTES_MAX: usize = 16 * 1024 * 1024;

// ============================================================================
// LibkrunConfig
// ============================================================================

/// Configuration specific to the libkrun backend
#[derive(Debug, Clone)]
pub struct LibkrunConfig {
    /// Port for guest agent vsock connection
    pub vsock_port: u32,
    /// Directory for vsock Unix sockets
    pub socket_dir: PathBuf,
    /// Enable debug logging in libkrun
    pub debug: bool,
}

impl Default for LibkrunConfig {
    fn default() -> Self {
        Self {
            vsock_port: LIBKRUN_VSOCK_PORT_DEFAULT,
            socket_dir: std::env::temp_dir().join("kelpie-libkrun"),
            debug: false,
        }
    }
}

// ============================================================================
// LibkrunVmFactory
// ============================================================================

/// Factory for creating libkrun VMs
#[derive(Debug, Clone)]
pub struct LibkrunVmFactory {
    config: LibkrunConfig,
}

impl LibkrunVmFactory {
    /// Create a new factory with the given configuration
    pub fn new(config: LibkrunConfig) -> Self {
        Self { config }
    }

    /// Create a new VM with the factory's configuration
    pub async fn create_vm(&self, config: VmConfig) -> VmResult<LibkrunVm> {
        LibkrunVm::new(config, self.config.clone()).await
    }
}

impl Default for LibkrunVmFactory {
    fn default() -> Self {
        Self::new(LibkrunConfig::default())
    }
}

#[async_trait]
impl VmFactory for LibkrunVmFactory {
    async fn create(&self, config: VmConfig) -> VmResult<Box<dyn VmInstance>> {
        let vm = self.create_vm(config).await?;
        Ok(Box::new(vm))
    }
}

// ============================================================================
// LibkrunVm
// ============================================================================

/// libkrun VM instance implementing VmInstance trait
///
/// This implementation uses libkrun's `krun_start_enter()` to boot the VM
/// with a guest agent, then communicates via vsock for command execution.
pub struct LibkrunVm {
    /// Unique VM identifier
    id: String,
    /// VM configuration
    config: VmConfig,
    /// libkrun-specific configuration
    libkrun_config: LibkrunConfig,
    /// Current VM state
    state: Mutex<VmState>,
    /// Path to the vsock Unix socket
    vsock_path: PathBuf,
    /// Flag indicating if the VM thread is running
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for LibkrunVm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibkrunVm")
            .field("id", &self.id)
            .field("config", &self.config)
            .field("state", &self.state)
            .field("vsock_path", &self.vsock_path)
            .finish()
    }
}

// Safety: LibkrunVm is Send + Sync because all mutable state is protected by Mutex
unsafe impl Send for LibkrunVm {}
unsafe impl Sync for LibkrunVm {}

impl LibkrunVm {
    /// Create a new libkrun VM
    pub async fn new(config: VmConfig, libkrun_config: LibkrunConfig) -> VmResult<Self> {
        config.validate()?;

        // Validate rootfs exists
        if !std::path::Path::new(&config.root_disk_path).exists() {
            return Err(VmError::RootDiskNotFound {
                path: config.root_disk_path.clone(),
            });
        }

        let id = format!("libkrun-{}", Uuid::new_v4());

        // Create socket directory if needed
        tokio::fs::create_dir_all(&libkrun_config.socket_dir)
            .await
            .map_err(|e| VmError::CreateFailed {
                reason: format!("failed to create socket dir: {}", e),
            })?;

        let vsock_path = libkrun_config.socket_dir.join(format!("{}.sock", id));

        Ok(Self {
            id,
            config,
            libkrun_config,
            state: Mutex::new(VmState::Created),
            vsock_path,
            running: Arc::new(AtomicBool::new(false)),
        })
    }

    fn set_state(&self, next: VmState) {
        if let Ok(mut state) = self.state.lock() {
            *state = next;
        }
    }

    /// Connect to the guest agent via vsock Unix socket
    ///
    /// libkrun creates a Unix socket for vsock communication. When the guest
    /// connects to the vsock port, it's tunneled through this socket.
    async fn connect_vsock(&self) -> VmResult<UnixStream> {
        let runtime = kelpie_core::current_runtime();

        // Wait for the vsock socket to be created and try to connect
        for i in 0..VSOCK_CONNECT_RETRY_COUNT_MAX {
            // Check if socket exists
            if !self.vsock_path.exists() {
                debug!(vm_id = %self.id, attempt = i + 1, path = ?self.vsock_path, "Waiting for vsock socket");
                runtime.sleep(VSOCK_CONNECT_RETRY_DELAY).await;
                continue;
            }

            // Try to connect
            match UnixStream::connect(&self.vsock_path).await {
                Ok(stream) => {
                    info!(vm_id = %self.id, "Connected to vsock after {} attempts", i + 1);
                    return Ok(stream);
                }
                Err(e) if i < VSOCK_CONNECT_RETRY_COUNT_MAX - 1 => {
                    debug!(vm_id = %self.id, attempt = i + 1, error = %e, path = ?self.vsock_path, "Vsock connect retry");
                    runtime.sleep(VSOCK_CONNECT_RETRY_DELAY).await;
                }
                Err(e) => {
                    return Err(VmError::ExecFailed {
                        reason: format!(
                            "failed to connect to vsock at {:?} after {} attempts: {}",
                            self.vsock_path, VSOCK_CONNECT_RETRY_COUNT_MAX, e
                        ),
                    });
                }
            }
        }

        Err(VmError::ExecFailed {
            reason: format!(
                "vsock socket {:?} not created after {} attempts",
                self.vsock_path, VSOCK_CONNECT_RETRY_COUNT_MAX
            ),
        })
    }

    /// Execute a command via the vsock guest agent
    async fn exec_via_vsock(
        &self,
        cmd: &str,
        args: &[&str],
        options: VmExecOptions,
    ) -> VmResult<VmExecOutput> {
        let mut stream = self.connect_vsock().await?;

        // Build JSON request (same protocol as VZ backend)
        let request = serde_json::json!({
            "type": "exec",
            "command": cmd,
            "args": args,
            "env": options.env,
            "cwd": options.workdir,
            "timeout_ms": options.timeout_ms,
        });

        let request_bytes = serde_json::to_vec(&request).map_err(|e| VmError::ExecFailed {
            reason: format!("failed to serialize exec request: {}", e),
        })?;

        // Send length-prefixed request
        let len_bytes = (request_bytes.len() as u32).to_be_bytes();
        stream
            .write_all(&len_bytes)
            .await
            .map_err(|e| VmError::ExecFailed {
                reason: format!("failed to write request length: {}", e),
            })?;
        stream
            .write_all(&request_bytes)
            .await
            .map_err(|e| VmError::ExecFailed {
                reason: format!("failed to write request: {}", e),
            })?;

        // Read length-prefixed response
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| VmError::ExecFailed {
                reason: format!("failed to read response length: {}", e),
            })?;
        let response_len = u32::from_be_bytes(len_buf) as usize;

        // Validate response length to prevent memory exhaustion
        if response_len > VSOCK_RESPONSE_SIZE_BYTES_MAX {
            return Err(VmError::ExecFailed {
                reason: format!(
                    "response too large: {} bytes (max: {} bytes)",
                    response_len, VSOCK_RESPONSE_SIZE_BYTES_MAX
                ),
            });
        }

        let mut response_buf = vec![0u8; response_len];
        stream
            .read_exact(&mut response_buf)
            .await
            .map_err(|e| VmError::ExecFailed {
                reason: format!("failed to read response: {}", e),
            })?;

        // Parse response
        let response: Value =
            serde_json::from_slice(&response_buf).map_err(|e| VmError::ExecFailed {
                reason: format!("failed to parse response: {}", e),
            })?;

        let exit_code = response
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1) as i32;
        let stdout = response
            .get("stdout")
            .and_then(|v| v.as_str())
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();
        let stderr = response
            .get("stderr")
            .and_then(|v| v.as_str())
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();

        Ok(VmExecOutput::new(exit_code, stdout, stderr))
    }

    /// Start the VM in a background thread using libkrun
    fn start_vm_thread(&self) -> VmResult<()> {
        // Create libkrun context
        let ctx_id = unsafe { krun_create_ctx() };
        if ctx_id < 0 {
            return Err(VmError::ContextCreationFailed {
                reason: format!("krun_create_ctx failed with code {}", ctx_id),
            });
        }
        let ctx_id = ctx_id as u32;

        // Configure VM resources
        let result = unsafe {
            krun_set_vm_config(ctx_id, self.config.vcpu_count as u8, self.config.memory_mib)
        };
        if result < 0 {
            unsafe { krun_free_ctx(ctx_id) };
            return Err(VmError::ConfigurationFailed {
                reason: format!("krun_set_vm_config failed with code {}", result),
            });
        }

        // Set root filesystem (libkrun uses its bundled kernel)
        let rootfs_c = CString::new(self.config.root_disk_path.clone()).map_err(|e| {
            unsafe { krun_free_ctx(ctx_id) };
            VmError::ConfigInvalid {
                reason: format!("invalid rootfs path: {}", e),
            }
        })?;

        let result = unsafe { krun_set_root(ctx_id, rootfs_c.as_ptr()) };
        if result < 0 {
            unsafe { krun_free_ctx(ctx_id) };
            return Err(VmError::ConfigurationFailed {
                reason: format!("krun_set_root failed with code {}", result),
            });
        }

        // Add vsock port for guest agent communication
        let vsock_path_c =
            CString::new(self.vsock_path.to_string_lossy().to_string()).map_err(|e| {
                unsafe { krun_free_ctx(ctx_id) };
                VmError::ConfigInvalid {
                    reason: format!("invalid vsock path: {}", e),
                }
            })?;

        // krun_add_vsock_port2 with listen=true: host listens on Unix socket, guest connects via vsock
        // When guest connects to this vsock port, libkrun forwards it to the Unix socket
        let result = unsafe {
            krun_add_vsock_port2(
                ctx_id,
                self.libkrun_config.vsock_port,
                vsock_path_c.as_ptr(),
                true, // Host listens, guest connects
            )
        };
        if result < 0 {
            unsafe { krun_free_ctx(ctx_id) };
            return Err(VmError::ConfigurationFailed {
                reason: format!("krun_add_vsock_port2 failed with code {}", result),
            });
        }

        // Set the executable to run (guest agent)
        let exec_path = CString::new("/usr/local/bin/kelpie-guest").unwrap();
        let argv: [*const c_char; 1] = [std::ptr::null()];
        let envp: [*const c_char; 1] = [std::ptr::null()];

        let result =
            unsafe { krun_set_exec(ctx_id, exec_path.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
        if result < 0 {
            unsafe { krun_free_ctx(ctx_id) };
            return Err(VmError::ConfigurationFailed {
                reason: format!("krun_set_exec failed with code {}", result),
            });
        }

        // Set working directory if specified
        if let Some(ref workdir) = self.config.workdir {
            let workdir_c = CString::new(workdir.clone()).map_err(|e| {
                unsafe { krun_free_ctx(ctx_id) };
                VmError::ConfigInvalid {
                    reason: format!("invalid workdir: {}", e),
                }
            })?;
            let result = unsafe { krun_set_workdir(ctx_id, workdir_c.as_ptr()) };
            if result < 0 {
                warn!(vm_id = %self.id, "krun_set_workdir failed with code {}", result);
            }
        }

        // Store context ID and start in background thread
        // Note: krun_start_enter blocks, so we run it in a thread
        let id = self.id.clone();
        let running = self.running.clone();

        thread::spawn(move || {
            running.store(true, Ordering::SeqCst);
            info!(vm_id = %id, "Starting libkrun VM");

            let result = unsafe { krun_start_enter(ctx_id) };

            running.store(false, Ordering::SeqCst);

            if result < 0 {
                error!(vm_id = %id, "krun_start_enter failed with code {}", result);
            } else {
                info!(vm_id = %id, "libkrun VM exited with code {}", result);
            }

            // Free the libkrun context
            let free_result = unsafe { krun_free_ctx(ctx_id) };
            if free_result < 0 {
                warn!(vm_id = %id, "krun_free_ctx failed with code {}", free_result);
            }
        });

        Ok(())
    }
}

impl Drop for LibkrunVm {
    fn drop(&mut self) {
        // Clean up vsock socket file
        if self.vsock_path.exists() {
            let _ = std::fs::remove_file(&self.vsock_path);
        }
    }
}

#[async_trait]
impl VmInstance for LibkrunVm {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> VmState {
        match self.state.lock() {
            Ok(state) => *state,
            Err(_) => VmState::Crashed,
        }
    }

    fn config(&self) -> &VmConfig {
        &self.config
    }

    async fn start(&mut self) -> VmResult<()> {
        let current = self.state();
        if matches!(
            current,
            VmState::Running | VmState::Starting | VmState::Paused
        ) {
            return Err(VmError::AlreadyRunning);
        }

        self.set_state(VmState::Starting);

        // Start the VM in a background thread
        match self.start_vm_thread() {
            Ok(()) => {
                // Wait for the VM to be ready (vsock becomes available)
                let runtime = kelpie_core::current_runtime();
                let max_wait_ms = crate::VM_BOOT_TIMEOUT_MS;
                let start = std::time::Instant::now();

                while start.elapsed().as_millis() < max_wait_ms as u128 {
                    if self.vsock_path.exists() {
                        // Try to connect to verify the guest agent is ready
                        match self.connect_vsock().await {
                            Ok(_) => {
                                self.set_state(VmState::Running);
                                info!(vm_id = %self.id, "libkrun VM started and ready");
                                return Ok(());
                            }
                            Err(_) => {
                                runtime.sleep(VSOCK_CONNECT_RETRY_DELAY).await;
                            }
                        }
                    } else {
                        runtime.sleep(VSOCK_CONNECT_RETRY_DELAY).await;
                    }

                    if !self.running.load(Ordering::SeqCst) {
                        self.set_state(VmState::Crashed);
                        return Err(VmError::BootFailed {
                            reason: "VM thread exited unexpectedly".to_string(),
                        });
                    }
                }

                self.set_state(VmState::Crashed);
                Err(VmError::BootTimeout {
                    timeout_ms: max_wait_ms,
                })
            }
            Err(e) => {
                self.set_state(VmState::Crashed);
                Err(e)
            }
        }
    }

    async fn stop(&mut self) -> VmResult<()> {
        let current = self.state();
        if !matches!(current, VmState::Running | VmState::Paused) {
            return Err(VmError::NotRunning {
                state: current.to_string(),
            });
        }

        // Send shutdown signal via vsock if possible
        if let Ok(mut stream) = self.connect_vsock().await {
            let request = serde_json::json!({
                "type": "shutdown",
            });
            let request_bytes = serde_json::to_vec(&request).unwrap_or_default();
            let len_bytes = (request_bytes.len() as u32).to_be_bytes();
            let _ = stream.write_all(&len_bytes).await;
            let _ = stream.write_all(&request_bytes).await;
        }

        // Wait for VM thread to exit
        let runtime = kelpie_core::current_runtime();
        let timeout_ms = 5000u64;
        let start = std::time::Instant::now();
        while self.running.load(Ordering::SeqCst)
            && start.elapsed().as_millis() < timeout_ms as u128
        {
            runtime.sleep(VSOCK_CONNECT_RETRY_DELAY).await;
        }

        self.set_state(VmState::Stopped);
        info!(vm_id = %self.id, "libkrun VM stopped");
        Ok(())
    }

    async fn pause(&mut self) -> VmResult<()> {
        // libkrun does not support pause/resume
        Err(VmError::PauseFailed {
            reason: "libkrun does not support pause".to_string(),
        })
    }

    async fn resume(&mut self) -> VmResult<()> {
        // libkrun does not support pause/resume
        Err(VmError::ResumeFailed {
            reason: "libkrun does not support resume".to_string(),
        })
    }

    async fn exec(&self, cmd: &str, args: &[&str]) -> VmResult<VmExecOutput> {
        self.exec_with_options(cmd, args, VmExecOptions::default())
            .await
    }

    async fn exec_with_options(
        &self,
        cmd: &str,
        args: &[&str],
        options: VmExecOptions,
    ) -> VmResult<VmExecOutput> {
        let current = self.state();
        if current != VmState::Running {
            return Err(VmError::NotRunning {
                state: current.to_string(),
            });
        }

        let timeout_ms = options.timeout_ms.unwrap_or(VM_EXEC_TIMEOUT_MS_DEFAULT);
        if timeout_ms == 0 {
            return Err(VmError::ExecTimeout { timeout_ms });
        }

        let result = kelpie_core::current_runtime()
            .timeout(
                std::time::Duration::from_millis(timeout_ms),
                self.exec_via_vsock(cmd, args, options),
            )
            .await;

        match result {
            Ok(result) => result,
            Err(_) => Err(VmError::ExecTimeout { timeout_ms }),
        }
    }

    async fn snapshot(&self) -> VmResult<VmSnapshot> {
        // libkrun does not support snapshots
        Err(VmError::SnapshotFailed {
            reason: "libkrun does not support snapshots".to_string(),
        })
    }

    async fn restore(&mut self, _snapshot: &VmSnapshot) -> VmResult<()> {
        // libkrun does not support snapshots
        Err(VmError::RestoreFailed {
            reason: "libkrun does not support restore".to_string(),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_libkrun_config_defaults() {
        let config = LibkrunConfig::default();
        assert_eq!(config.vsock_port, LIBKRUN_VSOCK_PORT_DEFAULT);
        assert!(!config.debug);
    }

    #[test]
    fn test_libkrun_vm_factory() {
        let factory = LibkrunVmFactory::default();
        assert_eq!(factory.config.vsock_port, LIBKRUN_VSOCK_PORT_DEFAULT);
    }

    #[tokio::test]
    async fn test_libkrun_vm_creation_missing_rootfs() {
        let config = VmConfig::builder()
            .root_disk("/nonexistent/rootfs.ext4")
            .build()
            .unwrap();

        let result = LibkrunVm::new(config, LibkrunConfig::default()).await;
        assert!(matches!(result, Err(VmError::RootDiskNotFound { .. })));
    }
}
