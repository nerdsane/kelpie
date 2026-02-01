//! Mock sandbox implementation for testing
//!
//! TigerStyle: In-memory simulation with deterministic behavior.

use crate::config::SandboxConfig;
use crate::error::{SandboxError, SandboxResult};
use crate::exec::{ExecOptions, ExecOutput};
use crate::snapshot::Snapshot;
use crate::traits::{Sandbox, SandboxFactory, SandboxState, SandboxStats};
use async_trait::async_trait;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Mock sandbox for testing
///
/// Simulates sandbox behavior without actual isolation.
/// Supports command handlers for test scenarios.
pub struct MockSandbox {
    id: String,
    config: SandboxConfig,
    state: RwLock<SandboxState>,
    /// Simulated filesystem
    filesystem: RwLock<HashMap<String, Bytes>>,
    /// Simulated environment variables
    env: RwLock<HashMap<String, String>>,
    /// Command handlers for simulation
    handlers: Arc<RwLock<HashMap<String, CommandHandler>>>,
    /// Simulated uptime
    start_time_ms: AtomicU64,
    /// Simulated memory usage
    memory_used: AtomicU64,
}

/// Handler for simulating command execution
pub type CommandHandler = Box<dyn Fn(&[&str], &ExecOptions) -> ExecOutput + Send + Sync>;

impl MockSandbox {
    /// Create a new mock sandbox
    pub fn new(config: SandboxConfig) -> Self {
        let id = Uuid::new_v4().to_string();

        // Initialize environment from config
        let mut env = HashMap::new();
        for (k, v) in &config.env {
            env.insert(k.clone(), v.clone());
        }

        Self {
            id,
            config,
            state: RwLock::new(SandboxState::Stopped),
            filesystem: RwLock::new(HashMap::new()),
            env: RwLock::new(env),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            start_time_ms: AtomicU64::new(0),
            memory_used: AtomicU64::new(0),
        }
    }

    /// Create with a specific ID
    pub fn with_id(id: impl Into<String>, config: SandboxConfig) -> Self {
        let mut sandbox = Self::new(config);
        sandbox.id = id.into();
        sandbox
    }

    /// Register a command handler for simulation
    pub async fn register_handler(&self, command: impl Into<String>, handler: CommandHandler) {
        let mut handlers = self.handlers.write().await;
        handlers.insert(command.into(), handler);
    }

    /// Write a file to the simulated filesystem
    pub async fn write_file(&self, path: impl Into<String>, content: impl Into<Bytes>) {
        let mut fs = self.filesystem.write().await;
        fs.insert(path.into(), content.into());
    }

    /// Read a file from the simulated filesystem
    pub async fn read_file(&self, path: &str) -> Option<Bytes> {
        let fs = self.filesystem.read().await;
        fs.get(path).cloned()
    }

    /// Set an environment variable
    pub async fn set_env(&self, key: impl Into<String>, value: impl Into<String>) {
        let mut env = self.env.write().await;
        env.insert(key.into(), value.into());
    }

    /// Get an environment variable
    pub async fn get_env(&self, key: &str) -> Option<String> {
        let env = self.env.read().await;
        env.get(key).cloned()
    }

    /// Simulate memory usage
    pub fn set_memory_used(&self, bytes: u64) {
        self.memory_used.store(bytes, Ordering::SeqCst);
    }

    /// Default command handler that echoes arguments
    fn default_handler(command: &str, args: &[&str], _options: &ExecOptions) -> ExecOutput {
        match command {
            "echo" => ExecOutput::success(format!("{}\n", args.join(" "))),
            "cat" => {
                // Simulate cat - just return the filename as content
                if args.is_empty() {
                    ExecOutput::failure(1, "cat: missing operand")
                } else {
                    ExecOutput::success(format!("contents of {}\n", args[0]))
                }
            }
            "ls" => ExecOutput::success("file1.txt\nfile2.txt\n"),
            "pwd" => ExecOutput::success("/workspace\n"),
            "true" => ExecOutput::success(""),
            "false" => ExecOutput::failure(1, ""),
            "sleep" => {
                // Simulate sleep (but don't actually sleep in tests)
                ExecOutput::success("")
            }
            _ => ExecOutput::failure(127, format!("{}: command not found\n", command)),
        }
    }
}

#[async_trait]
impl Sandbox for MockSandbox {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> SandboxState {
        // Note: This is a blocking read for the sync interface
        // In real code, we'd need interior mutability or async state
        futures::executor::block_on(async { *self.state.read().await })
    }

    fn config(&self) -> &SandboxConfig {
        &self.config
    }

    async fn start(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;

        if !state.can_start() && *state != SandboxState::Creating {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "stopped or creating".to_string(),
            });
        }

        *state = SandboxState::Running;
        self.start_time_ms.store(
            chrono::Utc::now().timestamp_millis() as u64,
            Ordering::SeqCst,
        );

        Ok(())
    }

    async fn stop(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;

        if !state.can_stop() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "running or paused".to_string(),
            });
        }

        *state = SandboxState::Stopped;
        Ok(())
    }

    async fn pause(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;

        if !state.can_pause() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "running".to_string(),
            });
        }

        *state = SandboxState::Paused;
        Ok(())
    }

    async fn resume(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;

        if !state.can_resume() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "paused".to_string(),
            });
        }

        *state = SandboxState::Running;
        Ok(())
    }

    async fn exec(
        &self,
        command: &str,
        args: &[&str],
        options: ExecOptions,
    ) -> SandboxResult<ExecOutput> {
        let state = self.state.read().await;

        if !state.can_exec() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "running".to_string(),
            });
        }
        drop(state);

        // Check for registered handler
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(command) {
            return Ok(handler(args, &options));
        }
        drop(handlers);

        // Use default handler
        Ok(Self::default_handler(command, args, &options))
    }

    async fn snapshot(&self) -> SandboxResult<Snapshot> {
        let state = self.state.read().await;

        if !state.can_snapshot() {
            return Err(SandboxError::InvalidState {
                sandbox_id: self.id.clone(),
                current: state.to_string(),
                expected: "running or paused".to_string(),
            });
        }
        drop(state);

        // Serialize filesystem and env
        let fs = self.filesystem.read().await;
        let env = self.env.read().await;

        let fs_data = serde_json::to_vec(&*fs).map_err(|e| SandboxError::SnapshotFailed {
            sandbox_id: self.id.clone(),
            reason: e.to_string(),
        })?;

        // Mock sandbox uses Suspend snapshot (memory-only, same-host)
        let snapshot = Snapshot::suspend(&self.id)
            .with_memory(Bytes::from(fs_data))
            .with_env_state(env.iter().map(|(k, v)| (k.clone(), v.clone())).collect());

        Ok(snapshot)
    }

    async fn restore(&mut self, snapshot: &Snapshot) -> SandboxResult<()> {
        // Restore filesystem
        if let Some(memory) = &snapshot.memory {
            let fs: HashMap<String, Bytes> =
                serde_json::from_slice(memory).map_err(|e| SandboxError::RestoreFailed {
                    sandbox_id: self.id.clone(),
                    reason: e.to_string(),
                })?;

            let mut current_fs = self.filesystem.write().await;
            *current_fs = fs;
        }

        // Restore environment
        if let Some(env_state) = &snapshot.env_state {
            let mut env = self.env.write().await;
            env.clear();
            for (k, v) in env_state {
                env.insert(k.clone(), v.clone());
            }
        }

        Ok(())
    }

    async fn destroy(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;
        *state = SandboxState::Destroying;

        // Clear all state
        self.filesystem.write().await.clear();
        self.env.write().await.clear();

        Ok(())
    }

    async fn health_check(&self) -> SandboxResult<bool> {
        let state = self.state.read().await;
        Ok(*state == SandboxState::Running)
    }

    async fn stats(&self) -> SandboxResult<SandboxStats> {
        let start_time = self.start_time_ms.load(Ordering::SeqCst);
        let uptime = if start_time > 0 {
            (chrono::Utc::now().timestamp_millis() as u64).saturating_sub(start_time)
        } else {
            0
        };

        Ok(SandboxStats {
            memory_bytes_used: self.memory_used.load(Ordering::SeqCst),
            cpu_percent: 0.0,
            disk_bytes_used: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            uptime_ms: uptime,
        })
    }
}

/// Factory for creating mock sandboxes
#[derive(Debug, Clone, Copy, Default)]
pub struct MockSandboxFactory;

impl MockSandboxFactory {
    /// Create a new factory
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SandboxFactory for MockSandboxFactory {
    type Sandbox = MockSandbox;

    async fn create(&self, config: SandboxConfig) -> SandboxResult<MockSandbox> {
        let mut sandbox = MockSandbox::new(config);
        sandbox.start().await?;
        Ok(sandbox)
    }

    async fn create_from_snapshot(
        &self,
        config: SandboxConfig,
        snapshot: &Snapshot,
    ) -> SandboxResult<MockSandbox> {
        let mut sandbox = MockSandbox::new(config);
        sandbox.restore(snapshot).await?;
        sandbox.start().await?;
        Ok(sandbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_sandbox_lifecycle() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config);

        assert_eq!(sandbox.state(), SandboxState::Stopped);

        sandbox.start().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        sandbox.pause().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Paused);

        sandbox.resume().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Stopped);
    }

    #[tokio::test]
    async fn test_mock_sandbox_exec() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config);
        sandbox.start().await.unwrap();

        let output = sandbox
            .exec_simple("echo", &["hello", "world"])
            .await
            .unwrap();
        assert!(output.is_success());
        assert_eq!(output.stdout_string(), "hello world\n");
    }

    #[tokio::test]
    async fn test_mock_sandbox_exec_failure() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config);
        sandbox.start().await.unwrap();

        let output = sandbox.exec_simple("false", &[]).await.unwrap();
        assert!(!output.is_success());
        assert_eq!(output.status.code, 1);
    }

    #[tokio::test]
    async fn test_mock_sandbox_custom_handler() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config);
        sandbox.start().await.unwrap();

        // Register a custom handler
        sandbox
            .register_handler(
                "custom",
                Box::new(|args, _| ExecOutput::success(format!("custom: {:?}", args))),
            )
            .await;

        let output = sandbox
            .exec_simple("custom", &["arg1", "arg2"])
            .await
            .unwrap();
        assert!(output.is_success());
        assert!(output.stdout_string().contains("arg1"));
    }

    #[tokio::test]
    async fn test_mock_sandbox_filesystem() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config);
        sandbox.start().await.unwrap();

        sandbox.write_file("/test.txt", "hello").await;
        let content = sandbox.read_file("/test.txt").await.unwrap();
        assert_eq!(content.as_ref(), b"hello");
    }

    #[tokio::test]
    async fn test_mock_sandbox_snapshot_restore() {
        let config = SandboxConfig::default();
        let mut sandbox = MockSandbox::new(config.clone());
        sandbox.start().await.unwrap();

        // Write some data
        sandbox.write_file("/data.txt", "important data").await;
        sandbox.set_env("MY_VAR", "my_value").await;

        // Create snapshot
        let snapshot = sandbox.snapshot().await.unwrap();

        // Create new sandbox and restore
        let mut sandbox2 = MockSandbox::new(config);
        sandbox2.restore(&snapshot).await.unwrap();
        sandbox2.start().await.unwrap();

        // Verify data restored
        let content = sandbox2.read_file("/data.txt").await.unwrap();
        assert_eq!(content.as_ref(), b"important data");

        let env_value = sandbox2.get_env("MY_VAR").await.unwrap();
        assert_eq!(env_value, "my_value");
    }

    #[tokio::test]
    async fn test_mock_sandbox_invalid_state() {
        let config = SandboxConfig::default();
        let sandbox = MockSandbox::new(config);

        // Can't exec when stopped
        let result = sandbox.exec_simple("echo", &["test"]).await;
        assert!(matches!(result, Err(SandboxError::InvalidState { .. })));
    }

    #[tokio::test]
    async fn test_mock_sandbox_factory() {
        let factory = MockSandboxFactory::new();
        let sandbox = factory.create(SandboxConfig::default()).await.unwrap();

        assert_eq!(sandbox.state(), SandboxState::Running);
    }
}
