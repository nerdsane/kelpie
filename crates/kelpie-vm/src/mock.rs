//! Mock VM implementation for testing
//!
//! TigerStyle: Simulated VM with configurable behavior for testing.

// Allow direct tokio usage in test/mock code
#![allow(clippy::disallowed_methods)]

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;

use crate::config::VmConfig;
use crate::error::{VmError, VmResult};
use crate::snapshot::{VmSnapshot, VmSnapshotMetadata};
use crate::traits::{ExecOptions, ExecOutput, VmInstance, VmState};
use crate::VM_EXEC_TIMEOUT_MS_DEFAULT;

/// Counter for generating unique VM IDs
static VM_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Mock VM implementation for testing without a hypervisor
#[derive(Debug)]
pub struct MockVm {
    /// Unique VM identifier
    id: String,

    /// VM configuration
    config: VmConfig,

    /// Current state
    state: VmState,

    /// Simulated boot delay in ms (for testing)
    boot_delay_ms: u64,

    /// Whether to simulate boot failure
    simulate_boot_failure: bool,

    /// Whether to simulate exec failure
    simulate_exec_failure: bool,

    /// Simulated architecture
    architecture: String,
}

impl MockVm {
    /// Create a new mock VM
    pub fn new(config: VmConfig) -> VmResult<Self> {
        config.validate()?;

        let id = format!("mock-vm-{}", VM_ID_COUNTER.fetch_add(1, Ordering::SeqCst));

        Ok(Self {
            id,
            config,
            state: VmState::Created,
            boot_delay_ms: 0,
            simulate_boot_failure: false,
            simulate_exec_failure: false,
            architecture: std::env::consts::ARCH.to_string(),
        })
    }

    /// Set simulated boot delay
    pub fn with_boot_delay(mut self, delay_ms: u64) -> Self {
        self.boot_delay_ms = delay_ms;
        self
    }

    /// Configure to simulate boot failure
    pub fn with_boot_failure(mut self) -> Self {
        self.simulate_boot_failure = true;
        self
    }

    /// Configure to simulate exec failure
    pub fn with_exec_failure(mut self) -> Self {
        self.simulate_exec_failure = true;
        self
    }

    /// Set the simulated architecture
    pub fn with_architecture(mut self, arch: impl Into<String>) -> Self {
        self.architecture = arch.into();
        self
    }

    /// Get the simulated architecture
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    /// Helper to check if VM is in a valid state for operation
    fn check_state(&self, required: VmState) -> VmResult<()> {
        if self.state != required {
            match required {
                VmState::Running => {
                    return Err(VmError::NotRunning {
                        state: self.state.to_string(),
                    });
                }
                _ => {
                    return Err(VmError::Internal {
                        reason: format!("expected state {:?}, got {:?}", required, self.state),
                    });
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl VmInstance for MockVm {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> VmState {
        self.state
    }

    fn config(&self) -> &VmConfig {
        &self.config
    }

    async fn start(&mut self) -> VmResult<()> {
        // Precondition: must be in Created or Stopped state
        if self.state != VmState::Created && self.state != VmState::Stopped {
            if self.state == VmState::Running {
                return Err(VmError::AlreadyRunning);
            }
            return Err(VmError::Internal {
                reason: format!("cannot start from state {:?}", self.state),
            });
        }

        self.state = VmState::Starting;

        // Simulate boot delay
        if self.boot_delay_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(self.boot_delay_ms)).await;
        }

        // Simulate boot failure
        if self.simulate_boot_failure {
            self.state = VmState::Crashed;
            return Err(VmError::BootFailed {
                reason: "simulated boot failure".into(),
            });
        }

        self.state = VmState::Running;

        // Postcondition
        assert_eq!(self.state, VmState::Running);

        Ok(())
    }

    async fn stop(&mut self) -> VmResult<()> {
        // Can stop from Running or Paused state
        if self.state != VmState::Running && self.state != VmState::Paused {
            return Err(VmError::Internal {
                reason: format!("cannot stop from state {:?}", self.state),
            });
        }

        self.state = VmState::Stopped;

        // Postcondition
        assert_eq!(self.state, VmState::Stopped);

        Ok(())
    }

    async fn pause(&mut self) -> VmResult<()> {
        self.check_state(VmState::Running)?;

        self.state = VmState::Paused;

        // Postcondition
        assert_eq!(self.state, VmState::Paused);

        Ok(())
    }

    async fn resume(&mut self) -> VmResult<()> {
        self.check_state(VmState::Paused)?;

        self.state = VmState::Running;

        // Postcondition
        assert_eq!(self.state, VmState::Running);

        Ok(())
    }

    async fn exec(&self, cmd: &str, args: &[&str]) -> VmResult<ExecOutput> {
        self.exec_with_options(cmd, args, ExecOptions::default())
            .await
    }

    async fn exec_with_options(
        &self,
        cmd: &str,
        args: &[&str],
        options: ExecOptions,
    ) -> VmResult<ExecOutput> {
        // Preconditions
        self.check_state(VmState::Running)?;
        assert!(!cmd.is_empty(), "command cannot be empty");

        // Simulate exec failure
        if self.simulate_exec_failure {
            return Err(VmError::ExecFailed {
                reason: "simulated exec failure".into(),
            });
        }

        // Build simulated output based on command
        let (exit_code, stdout, stderr) = match cmd {
            "echo" => (0, args.join(" "), String::new()),
            "true" => (0, String::new(), String::new()),
            "false" => (1, String::new(), String::new()),
            "cat" => {
                // Simulate reading /etc/hostname
                if args.first() == Some(&"/etc/hostname") {
                    (0, "mock-vm".to_string(), String::new())
                } else {
                    (0, String::new(), String::new())
                }
            }
            "sleep" => {
                // Parse sleep duration and actually sleep (for timeout testing)
                if let Some(duration_str) = args.first() {
                    if let Ok(seconds) = duration_str.parse::<f64>() {
                        let sleep_ms = (seconds * 1000.0) as u64;
                        let timeout_ms = options.timeout_ms.unwrap_or(VM_EXEC_TIMEOUT_MS_DEFAULT);

                        if sleep_ms > timeout_ms {
                            return Err(VmError::ExecTimeout { timeout_ms });
                        }

                        tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
                    }
                }
                (0, String::new(), String::new())
            }
            "sh" => {
                // Simulate shell execution
                if args.first() == Some(&"-c") {
                    if let Some(shell_cmd) = args.get(1) {
                        // Very basic shell simulation
                        if shell_cmd.contains("echo") {
                            let output = shell_cmd.replace("echo ", "");
                            (0, output, String::new())
                        } else if shell_cmd.contains("exit 1") {
                            (1, String::new(), String::new())
                        } else {
                            (0, String::new(), String::new())
                        }
                    } else {
                        (0, String::new(), String::new())
                    }
                } else {
                    (0, String::new(), String::new())
                }
            }
            _ => {
                // Unknown command - return error
                (127, String::new(), format!("command not found: {}", cmd))
            }
        };

        Ok(ExecOutput::new(exit_code, stdout, stderr))
    }

    async fn snapshot(&self) -> VmResult<VmSnapshot> {
        // Can snapshot from Running or Paused state
        if self.state != VmState::Running && self.state != VmState::Paused {
            return Err(VmError::SnapshotFailed {
                reason: format!("cannot snapshot from state {:?}", self.state),
            });
        }

        // Create mock snapshot data
        let snapshot_data = format!(
            "mock-snapshot-{}-{}",
            self.id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );

        let data = Bytes::from(snapshot_data);
        let checksum = crc32fast::hash(&data);

        let metadata = VmSnapshotMetadata::new(
            format!("snap-{}", uuid::Uuid::new_v4()),
            self.id.clone(),
            data.len() as u64,
            checksum,
            self.architecture.clone(),
            self.config.vcpu_count,
            self.config.memory_mib,
        );

        VmSnapshot::new(metadata, data)
    }

    async fn restore(&mut self, snapshot: &VmSnapshot) -> VmResult<()> {
        // Can restore to Created or Stopped state
        if self.state != VmState::Created && self.state != VmState::Stopped {
            return Err(VmError::RestoreFailed {
                reason: format!("cannot restore to state {:?}", self.state),
            });
        }

        // Verify checksum
        if !snapshot.verify_checksum() {
            return Err(VmError::SnapshotCorrupted);
        }

        // Check architecture compatibility
        if !snapshot.metadata.is_compatible_with(&self.architecture) {
            return Err(VmError::RestoreFailed {
                reason: format!(
                    "architecture mismatch: snapshot is {} but VM is {}",
                    snapshot.metadata.architecture, self.architecture
                ),
            });
        }

        // After restore, VM is in Stopped state (must be started)
        self.state = VmState::Stopped;

        Ok(())
    }
}

/// Factory for creating mock VMs
#[derive(Debug, Clone, Default)]
pub struct MockVmFactory {
    /// Default boot delay for created VMs
    boot_delay_ms: u64,
}

impl MockVmFactory {
    /// Create a new factory
    pub fn new() -> Self {
        Self::default()
    }

    /// Set default boot delay for VMs created by this factory
    pub fn with_boot_delay(mut self, delay_ms: u64) -> Self {
        self.boot_delay_ms = delay_ms;
        self
    }

    /// Create a new mock VM
    pub fn create_vm(&self, config: VmConfig) -> VmResult<MockVm> {
        let vm = MockVm::new(config)?.with_boot_delay(self.boot_delay_ms);
        Ok(vm)
    }
}

#[async_trait]
impl crate::traits::VmFactory for MockVmFactory {
    async fn create(&self, config: VmConfig) -> VmResult<Box<dyn VmInstance>> {
        let vm = self.create_vm(config)?;
        Ok(Box::new(vm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> VmConfig {
        VmConfig::builder()
            .cpus(2)
            .memory_mib(512)
            .root_disk("/path/to/rootfs.ext4")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_mock_vm_lifecycle() {
        let config = test_config();
        let mut vm = MockVm::new(config).unwrap();

        assert_eq!(vm.state(), VmState::Created);

        vm.start().await.unwrap();
        assert_eq!(vm.state(), VmState::Running);

        vm.pause().await.unwrap();
        assert_eq!(vm.state(), VmState::Paused);

        vm.resume().await.unwrap();
        assert_eq!(vm.state(), VmState::Running);

        vm.stop().await.unwrap();
        assert_eq!(vm.state(), VmState::Stopped);
    }

    #[tokio::test]
    async fn test_mock_vm_exec() {
        let config = test_config();
        let mut vm = MockVm::new(config).unwrap();
        vm.start().await.unwrap();

        let output = vm.exec("echo", &["hello", "world"]).await.unwrap();
        assert!(output.success());
        assert_eq!(output.stdout_str(), "hello world");
    }

    #[tokio::test]
    async fn test_mock_vm_exec_not_running() {
        let config = test_config();
        let vm = MockVm::new(config).unwrap();

        let result = vm.exec("echo", &["test"]).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VmError::NotRunning { .. }));
    }

    #[tokio::test]
    async fn test_mock_vm_boot_failure() {
        let config = test_config();
        let mut vm = MockVm::new(config).unwrap().with_boot_failure();

        let result = vm.start().await;
        assert!(result.is_err());
        assert_eq!(vm.state(), VmState::Crashed);
    }

    #[tokio::test]
    async fn test_mock_vm_exec_failure() {
        let config = test_config();
        let mut vm = MockVm::new(config).unwrap().with_exec_failure();
        vm.start().await.unwrap();

        let result = vm.exec("echo", &["test"]).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VmError::ExecFailed { .. }));
    }

    #[tokio::test]
    async fn test_mock_vm_snapshot_restore() {
        let config = test_config();
        let mut vm = MockVm::new(config.clone()).unwrap();
        vm.start().await.unwrap();

        // Take snapshot
        let snapshot = vm.snapshot().await.unwrap();
        assert!(snapshot.verify_checksum());

        // Stop and restore
        vm.stop().await.unwrap();

        // Create new VM and restore
        let mut vm2 = MockVm::new(config).unwrap();
        vm2.restore(&snapshot).await.unwrap();
        assert_eq!(vm2.state(), VmState::Stopped);

        // Start restored VM
        vm2.start().await.unwrap();
        assert_eq!(vm2.state(), VmState::Running);
    }

    #[tokio::test]
    async fn test_mock_vm_already_running() {
        let config = test_config();
        let mut vm = MockVm::new(config).unwrap();
        vm.start().await.unwrap();

        let result = vm.start().await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VmError::AlreadyRunning));
    }

    #[tokio::test]
    async fn test_mock_vm_factory() {
        let factory = MockVmFactory::new().with_boot_delay(10);
        let config = test_config();

        let mut vm = factory.create_vm(config).unwrap();
        vm.start().await.unwrap();
        assert_eq!(vm.state(), VmState::Running);
    }
}
