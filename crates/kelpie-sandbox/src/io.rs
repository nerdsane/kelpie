//! I/O Abstraction Layer for Sandbox Operations
//!
//! TigerStyle: Separate I/O from business logic for true DST.
//!
//! This module provides the foundation for deterministic simulation testing
//! by abstracting all I/O operations that a sandbox performs:
//!
//! - VM lifecycle (boot, shutdown)
//! - Command execution
//! - Snapshot capture and restore
//! - File system operations
//!
//! The same sandbox business logic (state machine, validation, error handling)
//! runs in both production and DST modes. Only the I/O differs.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │     GenericSandbox<IO>                              │
//! │     - State machine (SAME CODE)                     │
//! │     - Validation (SAME CODE)                        │
//! │     - Error handling (SAME CODE)                    │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//! ┌──────────────────────▼──────────────────────────────┐
//! │              SandboxIO trait                         │
//! │  boot(), shutdown(), exec(), snapshot(), restore()   │
//! └──────────────────────┬──────────────────────────────┘
//!                        │
//!           ┌────────────┴────────────┐
//!           │                         │
//!     ┌─────▼─────┐            ┌─────▼─────┐
//!     │ Production│            │    DST    │
//!     │   VmIO    │            │  SimIO    │
//!     │ ProcessIO │            │  (faults) │
//!     └───────────┘            └───────────┘
//! ```

use crate::config::SandboxConfig;
use crate::error::{SandboxError, SandboxResult};
use crate::exec::{ExecOptions, ExecOutput};
use crate::snapshot::Snapshot;
use crate::traits::{SandboxState, SandboxStats};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::TimeProvider;
use std::sync::Arc;

// ============================================================================
// SandboxIO Trait
// ============================================================================

/// Low-level I/O operations for sandbox implementations
///
/// This trait abstracts the actual I/O operations that differ between
/// production (real VMs) and DST (simulated). The state machine and
/// validation logic lives in `GenericSandbox`<`IO`>, not here.
///
/// # Implementations
///
/// - `VmSandboxIO`: Real VM-backed sandbox operations
/// - `ProcessSandboxIO`: Real OS process operations
/// - `SimSandboxIO` (in kelpie-dst): Simulated with fault injection
#[async_trait]
pub trait SandboxIO: Send + Sync + std::fmt::Debug {
    /// Boot the VM/process
    ///
    /// Called when sandbox transitions from Stopped -> Running.
    /// Should initialize all resources needed for execution.
    async fn boot(&mut self, config: &SandboxConfig) -> SandboxResult<()>;

    /// Shutdown the VM/process
    ///
    /// Called when sandbox transitions to Stopped.
    /// Should release all resources.
    async fn shutdown(&mut self) -> SandboxResult<()>;

    /// Pause execution
    ///
    /// Called when sandbox transitions Running -> Paused.
    async fn pause(&mut self) -> SandboxResult<()>;

    /// Resume execution
    ///
    /// Called when sandbox transitions Paused -> Running.
    async fn resume(&mut self) -> SandboxResult<()>;

    /// Execute a command
    ///
    /// Must only be called when sandbox is in Running state
    /// (validation happens in GenericSandbox).
    async fn exec(
        &self,
        command: &str,
        args: &[&str],
        options: &ExecOptions,
    ) -> SandboxResult<ExecOutput>;

    /// Capture a snapshot of current state
    ///
    /// Returns raw snapshot data that can be stored and restored later.
    async fn capture_snapshot(&self) -> SandboxResult<SnapshotData>;

    /// Restore from snapshot data
    ///
    /// Replaces current state with the provided snapshot.
    async fn restore_snapshot(&mut self, data: &SnapshotData) -> SandboxResult<()>;

    /// Read a file from the sandbox filesystem
    async fn read_file(&self, path: &str) -> SandboxResult<Bytes>;

    /// Write a file to the sandbox filesystem
    async fn write_file(&self, path: &str, content: &[u8]) -> SandboxResult<()>;

    /// Get resource statistics
    async fn get_stats(&self) -> SandboxResult<SandboxStats>;

    /// Check if the sandbox is healthy
    async fn health_check(&self) -> SandboxResult<bool>;
}

// ============================================================================
// Snapshot Data
// ============================================================================

/// Raw snapshot data for storage and restoration
///
/// This is the I/O-level representation of a snapshot.
/// The higher-level Snapshot type includes metadata and validation.
#[derive(Debug, Clone, Default)]
pub struct SnapshotData {
    /// Memory state (for Suspend/Teleport)
    pub memory: Option<Bytes>,
    /// CPU/register state (for Teleport)
    pub cpu_state: Option<Bytes>,
    /// Filesystem state (serialized)
    pub filesystem: Option<Bytes>,
    /// Environment variables
    pub env_vars: Vec<(String, String)>,
}

// ============================================================================
// GenericSandbox<IO>
// ============================================================================

/// Generic sandbox implementation with pluggable I/O
///
/// This struct contains ALL the shared logic (state machine, validation,
/// error handling) that runs identically in production and DST modes.
/// The only thing that differs is the I/O implementation.
///
/// # Example
///
/// ```ignore
/// // Production
/// let io = VmSandboxIO::new(config);
/// let sandbox = GenericSandbox::new("agent-1", config, io, time);
///
/// // DST
/// let io = SimSandboxIO::new(rng, faults);
/// let sandbox = GenericSandbox::new("agent-1", config, io, sim_clock);
///
/// // SAME CODE from here on
/// sandbox.start().await?;
/// let output = sandbox.exec_simple("echo", &["hello"]).await?;
/// ```
pub struct GenericSandbox<IO: SandboxIO> {
    /// Unique sandbox identifier
    id: String,
    /// Configuration
    config: SandboxConfig,
    /// Current state
    state: SandboxState,
    /// I/O implementation (injected)
    io: IO,
    /// Time provider (injected)
    time: Arc<dyn TimeProvider>,
    /// Start time (for uptime calculation)
    start_time_ms: Option<u64>,
}

impl<IO: SandboxIO> std::fmt::Debug for GenericSandbox<IO> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericSandbox")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("io", &self.io)
            .finish()
    }
}

impl<IO: SandboxIO> GenericSandbox<IO> {
    /// Create a new sandbox with the given I/O implementation
    pub fn new(
        id: impl Into<String>,
        config: SandboxConfig,
        io: IO,
        time: Arc<dyn TimeProvider>,
    ) -> Self {
        Self {
            id: id.into(),
            config,
            state: SandboxState::Stopped,
            io,
            time,
            start_time_ms: None,
        }
    }

    /// Get the sandbox ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the current state
    pub fn state(&self) -> SandboxState {
        self.state
    }

    /// Get the configuration
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Start the sandbox
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Sandbox is not in Stopped state
    /// - I/O boot fails
    pub async fn start(&mut self) -> SandboxResult<()> {
        // State validation (SAME in all modes)
        if !self.state.can_start() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "stopped".to_string(),
            });
        }

        // I/O operation (delegated)
        self.io.boot(&self.config).await?;

        // State transition (SAME in all modes)
        self.state = SandboxState::Running;
        self.start_time_ms = Some(self.time.now_ms());

        Ok(())
    }

    /// Stop the sandbox
    pub async fn stop(&mut self) -> SandboxResult<()> {
        if !self.state.can_stop() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "running or paused".to_string(),
            });
        }

        self.io.shutdown().await?;

        self.state = SandboxState::Stopped;
        self.start_time_ms = None;

        Ok(())
    }

    /// Pause the sandbox
    pub async fn pause(&mut self) -> SandboxResult<()> {
        if !self.state.can_pause() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "running".to_string(),
            });
        }

        self.io.pause().await?;
        self.state = SandboxState::Paused;

        Ok(())
    }

    /// Resume the sandbox
    pub async fn resume(&mut self) -> SandboxResult<()> {
        if !self.state.can_resume() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "paused".to_string(),
            });
        }

        self.io.resume().await?;
        self.state = SandboxState::Running;

        Ok(())
    }

    /// Execute a command
    pub async fn exec(
        &self,
        command: &str,
        args: &[&str],
        options: ExecOptions,
    ) -> SandboxResult<ExecOutput> {
        if !self.state.can_exec() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "running".to_string(),
            });
        }

        self.io.exec(command, args, &options).await
    }

    /// Execute a command with default options
    pub async fn exec_simple(&self, command: &str, args: &[&str]) -> SandboxResult<ExecOutput> {
        self.exec(command, args, ExecOptions::default()).await
    }

    /// Create a snapshot
    pub async fn snapshot(&self) -> SandboxResult<Snapshot> {
        if !self.state.can_snapshot() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "running or paused".to_string(),
            });
        }

        let data = self.io.capture_snapshot().await?;

        // Convert SnapshotData to Snapshot
        Ok(Snapshot::suspend(&self.id)
            .with_memory(data.memory.unwrap_or_default())
            .with_cpu_state(data.cpu_state.unwrap_or_default())
            .with_env_state(data.env_vars))
    }

    /// Restore from a snapshot
    pub async fn restore(&mut self, snapshot: &Snapshot) -> SandboxResult<()> {
        // Convert Snapshot to SnapshotData
        let data = SnapshotData {
            memory: snapshot.memory.clone(),
            cpu_state: snapshot.cpu_state.clone(),
            filesystem: None, // TODO: Add filesystem to Snapshot
            env_vars: snapshot.env_state.clone().unwrap_or_default(),
        };

        self.io.restore_snapshot(&data).await
    }

    /// Destroy the sandbox
    pub async fn destroy(&mut self) -> SandboxResult<()> {
        if !self.state.can_destroy() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: self.state.to_string(),
                expected: "not destroying".to_string(),
            });
        }

        self.state = SandboxState::Destroying;

        // Best effort shutdown
        let _ = self.io.shutdown().await;

        Ok(())
    }

    /// Health check
    pub async fn health_check(&self) -> SandboxResult<bool> {
        self.io.health_check().await
    }

    /// Get statistics
    pub async fn stats(&self) -> SandboxResult<SandboxStats> {
        let mut stats = self.io.get_stats().await?;

        // Add uptime calculation (SAME in all modes)
        if let Some(start) = self.start_time_ms {
            stats.uptime_ms = self.time.now_ms().saturating_sub(start);
        }

        Ok(stats)
    }

    /// Read a file from the sandbox
    pub async fn read_file(&self, path: &str) -> SandboxResult<Bytes> {
        self.io.read_file(path).await
    }

    /// Write a file to the sandbox
    pub async fn write_file(&self, path: &str, content: &[u8]) -> SandboxResult<()> {
        self.io.write_file(path, content).await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExitStatus;
    use kelpie_core::WallClockTime;
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    /// Test I/O implementation for unit tests
    #[derive(Debug)]
    struct TestSandboxIO {
        booted: RwLock<bool>,
        paused: RwLock<bool>,
        filesystem: RwLock<HashMap<String, Bytes>>,
    }

    impl TestSandboxIO {
        fn new() -> Self {
            Self {
                booted: RwLock::new(false),
                paused: RwLock::new(false),
                filesystem: RwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl SandboxIO for TestSandboxIO {
        async fn boot(&mut self, _config: &SandboxConfig) -> SandboxResult<()> {
            *self.booted.write().await = true;
            Ok(())
        }

        async fn shutdown(&mut self) -> SandboxResult<()> {
            *self.booted.write().await = false;
            Ok(())
        }

        async fn pause(&mut self) -> SandboxResult<()> {
            *self.paused.write().await = true;
            Ok(())
        }

        async fn resume(&mut self) -> SandboxResult<()> {
            *self.paused.write().await = false;
            Ok(())
        }

        async fn exec(
            &self,
            command: &str,
            args: &[&str],
            _options: &ExecOptions,
        ) -> SandboxResult<ExecOutput> {
            Ok(ExecOutput {
                status: ExitStatus::success(),
                stdout: Bytes::from(format!("executed: {} {:?}", command, args)),
                stderr: Bytes::new(),
                duration_ms: 1,
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }

        async fn capture_snapshot(&self) -> SandboxResult<SnapshotData> {
            Ok(SnapshotData::default())
        }

        async fn restore_snapshot(&mut self, _data: &SnapshotData) -> SandboxResult<()> {
            Ok(())
        }

        async fn read_file(&self, path: &str) -> SandboxResult<Bytes> {
            self.filesystem
                .read()
                .await
                .get(path)
                .cloned()
                .ok_or_else(|| SandboxError::ExecFailed {
                    command: format!("read_file({})", path),
                    reason: "file not found".to_string(),
                })
        }

        async fn write_file(&self, path: &str, content: &[u8]) -> SandboxResult<()> {
            self.filesystem
                .write()
                .await
                .insert(path.to_string(), Bytes::copy_from_slice(content));
            Ok(())
        }

        async fn get_stats(&self) -> SandboxResult<SandboxStats> {
            Ok(SandboxStats::default())
        }

        async fn health_check(&self) -> SandboxResult<bool> {
            Ok(*self.booted.read().await)
        }
    }

    #[tokio::test]
    async fn test_generic_sandbox_lifecycle() {
        let io = TestSandboxIO::new();
        let time = Arc::new(WallClockTime::new());
        let mut sandbox = GenericSandbox::new("test-1", SandboxConfig::default(), io, time);

        // Initial state
        assert_eq!(sandbox.state(), SandboxState::Stopped);

        // Start
        sandbox.start().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        // Exec
        let output = sandbox.exec_simple("echo", &["hello"]).await.unwrap();
        assert!(output.is_success());

        // Pause
        sandbox.pause().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Paused);

        // Resume
        sandbox.resume().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        // Stop
        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Stopped);
    }

    #[tokio::test]
    async fn test_generic_sandbox_invalid_state() {
        let io = TestSandboxIO::new();
        let time = Arc::new(WallClockTime::new());
        let mut sandbox = GenericSandbox::new("test-2", SandboxConfig::default(), io, time);

        // Can't exec when stopped
        let result = sandbox.exec_simple("echo", &["hello"]).await;
        assert!(result.is_err());

        // Can't pause when stopped
        let result = sandbox.pause().await;
        assert!(result.is_err());

        // Start
        sandbox.start().await.unwrap();

        // Can't start when running
        let result = sandbox.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_generic_sandbox_file_ops() {
        let io = TestSandboxIO::new();
        let time = Arc::new(WallClockTime::new());
        let mut sandbox = GenericSandbox::new("test-3", SandboxConfig::default(), io, time);

        sandbox.start().await.unwrap();

        // Write file
        sandbox.write_file("/test.txt", b"hello").await.unwrap();

        // Read file
        let content = sandbox.read_file("/test.txt").await.unwrap();
        assert_eq!(content, Bytes::from("hello"));

        // File not found
        let result = sandbox.read_file("/nonexistent").await;
        assert!(result.is_err());
    }
}
