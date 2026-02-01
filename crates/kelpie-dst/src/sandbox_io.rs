//! Simulated SandboxIO for Deterministic Simulation Testing
//!
//! TigerStyle: Same business logic, different I/O - enables true DST.
//!
//! This module provides `SimSandboxIO`, an implementation of the `SandboxIO` trait
//! that runs entirely in-memory with fault injection. When combined with
//! `GenericSandbox`<`SimSandboxIO`>, you get a sandbox that:
//!
//! 1. Runs the SAME state machine code as production
//! 2. Uses simulated I/O instead of real VMs
//! 3. Supports deterministic fault injection
//! 4. Uses SimClock for deterministic time
//!
//! # Example
//!
//! ```ignore
//! // Production uses GenericSandbox<VmSandboxIO>
//! // DST uses GenericSandbox<SimSandboxIO>
//! // SAME state machine code runs in both!
//!
//! let sim_io = SimSandboxIO::new(rng.clone(), faults.clone(), clock.clone());
//! let sandbox = GenericSandbox::new("agent-1", config, sim_io, clock);
//!
//! sandbox.start().await?;  // State machine validation runs HERE
//! sandbox.exec_simple("echo", &["hello"]).await?;  // I/O runs in SimSandboxIO
//! ```

use crate::clock::SimClock;
use crate::fault::{FaultInjector, FaultType};
use crate::rng::DeterministicRng;
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_sandbox::io::{SandboxIO, SnapshotData};
use kelpie_sandbox::{
    ExecOptions, ExecOutput, ExitStatus, SandboxConfig, SandboxError, SandboxResult, SandboxStats,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Simulated SandboxIO with fault injection
///
/// This I/O implementation runs entirely in-memory and supports
/// deterministic fault injection for DST.
#[derive(Debug)]
pub struct SimSandboxIO {
    /// Simulated filesystem
    filesystem: RwLock<HashMap<String, Bytes>>,
    /// Simulated environment variables
    env: RwLock<HashMap<String, String>>,
    /// Whether the "VM" is running
    running: RwLock<bool>,
    /// Whether paused
    paused: RwLock<bool>,
    /// Deterministic RNG
    rng: Arc<DeterministicRng>,
    /// Fault injector
    faults: Arc<FaultInjector>,
    /// Simulated clock
    clock: Arc<SimClock>,
    /// Simulated memory usage
    memory_used: RwLock<u64>,
}

impl SimSandboxIO {
    /// Create a new simulated sandbox I/O
    pub fn new(
        rng: Arc<DeterministicRng>,
        faults: Arc<FaultInjector>,
        clock: Arc<SimClock>,
    ) -> Self {
        Self {
            filesystem: RwLock::new(HashMap::new()),
            env: RwLock::new(HashMap::new()),
            running: RwLock::new(false),
            paused: RwLock::new(false),
            rng,
            faults,
            clock,
            memory_used: RwLock::new(0),
        }
    }

    /// Check for fault injection
    fn check_fault(&self, operation: &str) -> Option<FaultType> {
        self.faults.should_inject(operation)
    }

    /// Convert fault to sandbox error
    fn fault_to_error(&self, fault: FaultType) -> SandboxError {
        match fault {
            FaultType::SandboxBootFail => SandboxError::ExecFailed {
                command: "boot".to_string(),
                reason: "injected boot failure".to_string(),
            },
            FaultType::SandboxCrash => SandboxError::ExecFailed {
                command: "runtime".to_string(),
                reason: "injected crash".to_string(),
            },
            FaultType::SandboxPauseFail => SandboxError::ExecFailed {
                command: "pause".to_string(),
                reason: "injected pause failure".to_string(),
            },
            FaultType::SandboxResumeFail => SandboxError::ExecFailed {
                command: "resume".to_string(),
                reason: "injected resume failure".to_string(),
            },
            FaultType::SandboxExecFail => SandboxError::ExecFailed {
                command: "exec".to_string(),
                reason: "injected exec failure".to_string(),
            },
            FaultType::SandboxExecTimeout { timeout_ms } => SandboxError::ExecTimeout {
                command: "exec".to_string(),
                timeout_ms,
            },
            FaultType::SnapshotCreateFail => SandboxError::SnapshotFailed {
                sandbox_id: "sim".to_string(),
                reason: "injected snapshot failure".to_string(),
            },
            FaultType::SnapshotRestoreFail => SandboxError::RestoreFailed {
                sandbox_id: "sim".to_string(),
                reason: "injected restore failure".to_string(),
            },
            FaultType::SnapshotCorruption => SandboxError::RestoreFailed {
                sandbox_id: "sim".to_string(),
                reason: "injected corruption".to_string(),
            },
            FaultType::SnapshotTooLarge { max_bytes } => SandboxError::SnapshotFailed {
                sandbox_id: "sim".to_string(),
                reason: format!("snapshot exceeds {} bytes", max_bytes),
            },
            _ => SandboxError::ExecFailed {
                command: "unknown".to_string(),
                reason: format!("injected fault: {:?}", fault),
            },
        }
    }

    /// Default command handler - simulates basic commands
    fn default_handler(command: &str, args: &[&str]) -> ExecOutput {
        let stdout = match command {
            "echo" => Bytes::from(args.join(" ")),
            "pwd" => Bytes::from("/workspace"),
            "whoami" => Bytes::from("agent"),
            "date" => Bytes::from("2024-01-01T00:00:00Z"),
            "true" => Bytes::new(),
            "false" => {
                return ExecOutput {
                    status: ExitStatus::with_code(1),
                    stdout: Bytes::new(),
                    stderr: Bytes::new(),
                    duration_ms: 1,
                    stdout_truncated: false,
                    stderr_truncated: false,
                }
            }
            _ => Bytes::from(format!("simulated: {} {:?}", command, args)),
        };

        ExecOutput {
            status: ExitStatus::success(),
            stdout,
            stderr: Bytes::new(),
            duration_ms: 1,
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }
}

#[async_trait]
impl SandboxIO for SimSandboxIO {
    async fn boot(&mut self, config: &SandboxConfig) -> SandboxResult<()> {
        // Check for boot fault
        if let Some(fault) = self.check_fault("sandbox_boot") {
            if matches!(fault, FaultType::SandboxBootFail) {
                return Err(self.fault_to_error(fault));
            }
        }

        // Initialize environment from config
        let mut env = self.env.write().await;
        for (k, v) in &config.env {
            env.insert(k.clone(), v.clone());
        }

        *self.running.write().await = true;
        *self.memory_used.write().await = config.limits.memory_bytes_max;

        Ok(())
    }

    async fn shutdown(&mut self) -> SandboxResult<()> {
        *self.running.write().await = false;
        *self.paused.write().await = false;
        Ok(())
    }

    async fn pause(&mut self) -> SandboxResult<()> {
        // Check for pause fault
        if let Some(fault) = self.check_fault("sandbox_pause") {
            if matches!(fault, FaultType::SandboxPauseFail) {
                return Err(self.fault_to_error(fault));
            }
        }

        *self.paused.write().await = true;
        Ok(())
    }

    async fn resume(&mut self) -> SandboxResult<()> {
        // Check for resume fault
        if let Some(fault) = self.check_fault("sandbox_resume") {
            if matches!(fault, FaultType::SandboxResumeFail) {
                return Err(self.fault_to_error(fault));
            }
        }

        *self.paused.write().await = false;
        Ok(())
    }

    async fn exec(
        &self,
        command: &str,
        args: &[&str],
        options: &ExecOptions,
    ) -> SandboxResult<ExecOutput> {
        // Check for crash fault
        if let Some(fault) = self.check_fault("sandbox_exec") {
            match &fault {
                FaultType::SandboxCrash | FaultType::SandboxExecFail => {
                    return Err(self.fault_to_error(fault));
                }
                FaultType::SandboxExecTimeout { timeout_ms } => {
                    // Simulate timeout by advancing clock
                    self.clock.advance_ms(*timeout_ms);
                    return Err(self.fault_to_error(fault));
                }
                _ => {}
            }
        }

        // Check for network delay fault (simulates slow execution)
        if let Some(FaultType::NetworkDelay { min_ms, max_ms }) =
            self.check_fault("sandbox_exec_delay")
        {
            let delay = self.rng.next_range(min_ms, max_ms);
            self.clock.advance_ms(delay);
        }

        // Execute command (simulated)
        let mut output = Self::default_handler(command, args);

        // Apply timeout if set
        if let Some(timeout_ms) = options.timeout_ms {
            if output.duration_ms > timeout_ms {
                return Err(SandboxError::ExecTimeout {
                    command: command.to_string(),
                    timeout_ms,
                });
            }
        }

        // Apply output limits
        let max_bytes = options.max_output_bytes as usize;
        if output.stdout.len() > max_bytes {
            output.stdout = output.stdout.slice(0..max_bytes);
            output.stdout_truncated = true;
        }
        if output.stderr.len() > max_bytes {
            output.stderr = output.stderr.slice(0..max_bytes);
            output.stderr_truncated = true;
        }

        Ok(output)
    }

    async fn capture_snapshot(&self) -> SandboxResult<SnapshotData> {
        // Check for snapshot fault
        if let Some(fault) = self.check_fault("sandbox_snapshot") {
            match &fault {
                FaultType::SnapshotCreateFail | FaultType::SnapshotTooLarge { .. } => {
                    return Err(self.fault_to_error(fault));
                }
                _ => {}
            }
        }

        // Serialize filesystem and env
        let fs = self.filesystem.read().await;
        let env = self.env.read().await;

        let fs_bytes = serde_json::to_vec(&*fs).unwrap_or_default();
        let env_vars: Vec<(String, String)> =
            env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        Ok(SnapshotData {
            memory: Some(Bytes::from(fs_bytes)),
            cpu_state: None,
            filesystem: None,
            env_vars,
        })
    }

    async fn restore_snapshot(&mut self, data: &SnapshotData) -> SandboxResult<()> {
        // Check for restore fault
        if let Some(fault) = self.check_fault("sandbox_restore") {
            match &fault {
                FaultType::SnapshotRestoreFail | FaultType::SnapshotCorruption => {
                    return Err(self.fault_to_error(fault));
                }
                _ => {}
            }
        }

        // Restore filesystem from memory (if present)
        if let Some(ref memory) = data.memory {
            if let Ok(fs) = serde_json::from_slice::<HashMap<String, Bytes>>(memory) {
                *self.filesystem.write().await = fs;
            }
        }

        // Restore environment
        let mut env = self.env.write().await;
        env.clear();
        for (k, v) in &data.env_vars {
            env.insert(k.clone(), v.clone());
        }

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
        Ok(SandboxStats {
            memory_bytes_used: *self.memory_used.read().await,
            cpu_percent: 0.0,
            disk_bytes_used: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            uptime_ms: 0, // GenericSandbox calculates this
        })
    }

    async fn health_check(&self) -> SandboxResult<bool> {
        Ok(*self.running.read().await && !*self.paused.read().await)
    }
}

// ============================================================================
// Factory
// ============================================================================

use kelpie_core::TimeProvider;
use kelpie_sandbox::io::GenericSandbox;
use std::sync::atomic::{AtomicU64, Ordering};

/// Factory for creating `GenericSandbox`<`SimSandboxIO`> instances
#[derive(Clone)]
pub struct SimSandboxIOFactory {
    /// Shared RNG
    rng: Arc<DeterministicRng>,
    /// Shared fault injector
    faults: Arc<FaultInjector>,
    /// Shared clock
    clock: Arc<SimClock>,
    /// Counter for unique IDs
    id_counter: Arc<AtomicU64>,
}

impl std::fmt::Debug for SimSandboxIOFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimSandboxIOFactory")
            .field("rng_seed", &self.rng.seed())
            .finish()
    }
}

impl SimSandboxIOFactory {
    /// Create a new factory
    pub fn new(
        rng: Arc<DeterministicRng>,
        faults: Arc<FaultInjector>,
        clock: Arc<SimClock>,
    ) -> Self {
        Self {
            rng,
            faults,
            clock,
            id_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Create a new sandbox using the generic sandbox + sim I/O
    pub async fn create(
        &self,
        config: SandboxConfig,
    ) -> SandboxResult<GenericSandbox<SimSandboxIO>> {
        let id = format!(
            "sim-sandbox-{}",
            self.id_counter.fetch_add(1, Ordering::SeqCst)
        );
        let io = SimSandboxIO::new(self.rng.clone(), self.faults.clone(), self.clock.clone());
        let time: Arc<dyn TimeProvider> = self.clock.clone();
        Ok(GenericSandbox::new(id, config, io, time))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::FaultConfig;

    /// Helper to create test components
    fn create_test_components() -> (Arc<DeterministicRng>, Arc<FaultInjector>, Arc<SimClock>) {
        let rng = DeterministicRng::new(42);
        let rng_for_faults = rng.fork();
        let rng_arc = Arc::new(rng);
        let faults = Arc::new(FaultInjector::new(rng_for_faults));
        let clock = Arc::new(SimClock::default());
        (rng_arc, faults, clock)
    }

    #[tokio::test]
    async fn test_sim_sandbox_io_lifecycle() {
        let (rng, faults, clock) = create_test_components();

        let factory = SimSandboxIOFactory::new(rng, faults, clock.clone());
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        // Lifecycle
        sandbox.start().await.unwrap();
        assert_eq!(sandbox.state(), kelpie_sandbox::SandboxState::Running);

        let output = sandbox.exec_simple("echo", &["hello"]).await.unwrap();
        assert!(output.is_success());
        assert_eq!(output.stdout, Bytes::from("hello"));

        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state(), kelpie_sandbox::SandboxState::Stopped);
    }

    #[tokio::test]
    async fn test_sim_sandbox_io_with_faults() {
        let rng = DeterministicRng::new(42);
        let rng_for_faults = rng.fork();
        let rng_arc = Arc::new(rng);
        let mut fault_injector = FaultInjector::new(rng_for_faults);
        // Add boot failure fault at 100%
        fault_injector.register(FaultConfig::new(FaultType::SandboxBootFail, 1.0));
        let faults = Arc::new(fault_injector);
        let clock = Arc::new(SimClock::default());

        let factory = SimSandboxIOFactory::new(rng_arc, faults, clock);
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        // Boot should fail
        let result = sandbox.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sim_sandbox_io_state_validation() {
        let (rng, faults, clock) = create_test_components();

        let factory = SimSandboxIOFactory::new(rng, faults, clock);
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        // Can't exec when stopped (state validation in GenericSandbox)
        let result = sandbox.exec_simple("echo", &["hello"]).await;
        assert!(result.is_err());

        // Start and try again
        sandbox.start().await.unwrap();
        let result = sandbox.exec_simple("echo", &["hello"]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_sim_sandbox_io_file_operations() {
        let (rng, faults, clock) = create_test_components();

        let factory = SimSandboxIOFactory::new(rng, faults, clock);
        let mut sandbox = factory.create(SandboxConfig::default()).await.unwrap();
        sandbox.start().await.unwrap();

        // Write file
        sandbox
            .write_file("/test.txt", b"hello world")
            .await
            .unwrap();

        // Read file
        let content = sandbox.read_file("/test.txt").await.unwrap();
        assert_eq!(content, Bytes::from("hello world"));
    }

    #[tokio::test]
    async fn test_sim_sandbox_io_snapshot_restore() {
        let (rng, faults, clock) = create_test_components();

        let factory = SimSandboxIOFactory::new(rng, faults, clock);

        // Create first sandbox, write file, snapshot
        let mut sandbox1 = factory.create(SandboxConfig::default()).await.unwrap();
        sandbox1.start().await.unwrap();
        sandbox1
            .write_file("/data.txt", b"important")
            .await
            .unwrap();
        let snapshot = sandbox1.snapshot().await.unwrap();

        // Create second sandbox, restore, verify file
        let mut sandbox2 = factory.create(SandboxConfig::default()).await.unwrap();
        sandbox2.start().await.unwrap();
        sandbox2.restore(&snapshot).await.unwrap();

        let content = sandbox2.read_file("/data.txt").await.unwrap();
        assert_eq!(content, Bytes::from("important"));
    }
}
