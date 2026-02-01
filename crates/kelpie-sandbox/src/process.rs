//! Process-based sandbox for real command execution
//!
//! TigerStyle: OS process isolation with resource limits.

use crate::config::SandboxConfig;
use crate::error::{SandboxError, SandboxResult};
use crate::exec::{ExecOptions, ExecOutput, ExitStatus};
use crate::snapshot::Snapshot;
use crate::traits::{Sandbox, SandboxFactory, SandboxState, SandboxStats};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::Runtime;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Process-based sandbox that executes commands in real OS processes
///
/// Provides basic isolation through:
/// - Working directory restriction
/// - Environment variable control
/// - Timeout enforcement
/// - Output size limits
pub struct ProcessSandbox {
    id: String,
    config: SandboxConfig,
    state: RwLock<SandboxState>,
    workdir: String,
    start_time: RwLock<Option<Instant>>,
}

impl ProcessSandbox {
    /// Create a new process sandbox
    pub fn new(config: SandboxConfig) -> Self {
        let id = Uuid::new_v4().to_string();
        let workdir = config.workdir.clone();

        Self {
            id,
            config,
            state: RwLock::new(SandboxState::Stopped),
            workdir,
            start_time: RwLock::new(None),
        }
    }

    /// Create with a specific ID
    pub fn with_id(id: impl Into<String>, config: SandboxConfig) -> Self {
        let mut sandbox = Self::new(config);
        sandbox.id = id.into();
        sandbox
    }
}

#[async_trait]
impl Sandbox for ProcessSandbox {
    fn id(&self) -> &str {
        &self.id
    }

    fn state(&self) -> SandboxState {
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

        // Ensure workdir exists
        tokio::fs::create_dir_all(&self.workdir)
            .await
            .map_err(|e| SandboxError::Internal {
                message: format!(
                    "failed to start sandbox {}: failed to create workdir: {}",
                    self.id, e
                ),
            })?;

        *state = SandboxState::Running;
        *self.start_time.write().await = Some(Instant::now());

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

        let start = Instant::now();
        let timeout = Duration::from_millis(options.timeout_ms.unwrap_or(30_000));
        let max_output = options.max_output_bytes as usize;

        // Build command
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(options.workdir.as_deref().unwrap_or(&self.workdir))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        // Set environment from config
        cmd.env_clear();
        for (k, v) in &self.config.env {
            cmd.env(k, v);
        }
        // Add standard PATH
        cmd.env("PATH", "/usr/local/bin:/usr/bin:/bin");

        // Spawn process
        let mut child = cmd.spawn().map_err(|e| SandboxError::ExecFailed {
            command: command.to_string(),
            reason: e.to_string(),
        })?;

        // Wait with timeout
        let result = kelpie_core::current_runtime()
            .timeout(timeout, async {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();

                if let Some(mut child_stdout) = child.stdout.take() {
                    let mut buf = vec![0u8; max_output];
                    let n = child_stdout.read(&mut buf).await.unwrap_or(0);
                    stdout.extend_from_slice(&buf[..n.min(max_output)]);
                }

                if let Some(mut child_stderr) = child.stderr.take() {
                    let mut buf = vec![0u8; max_output];
                    let n = child_stderr.read(&mut buf).await.unwrap_or(0);
                    stderr.extend_from_slice(&buf[..n.min(max_output)]);
                }

                let status = child.wait().await;
                (stdout, stderr, status)
            })
            .await;

        let duration = start.elapsed();

        match result {
            Ok((stdout, stderr, status)) => {
                let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

                let stdout_truncated = stdout.len() >= max_output;
                let stderr_truncated = stderr.len() >= max_output;

                Ok(ExecOutput {
                    stdout: Bytes::from(stdout),
                    stderr: Bytes::from(stderr),
                    status: ExitStatus {
                        code: exit_code,
                        signal: None,
                    },
                    duration_ms: duration.as_millis() as u64,
                    stdout_truncated,
                    stderr_truncated,
                })
            }
            Err(_) => {
                // Timeout - kill the process
                let _ = child.kill().await;

                Ok(ExecOutput {
                    stdout: Bytes::new(),
                    stderr: Bytes::from(format!("Command timed out after {:?}", timeout)),
                    status: ExitStatus {
                        code: -1,
                        signal: Some(9), // SIGKILL
                    },
                    duration_ms: duration.as_millis() as u64,
                    stdout_truncated: false,
                    stderr_truncated: false,
                })
            }
        }
    }

    async fn snapshot(&self) -> SandboxResult<Snapshot> {
        // ProcessSandbox doesn't support snapshotting - use FirecrackerSandbox for that
        Err(SandboxError::SnapshotFailed {
            sandbox_id: self.id.clone(),
            reason: "ProcessSandbox does not support snapshotting. Use FirecrackerSandbox for snapshot/restore capabilities.".to_string(),
        })
    }

    async fn restore(&mut self, _snapshot: &Snapshot) -> SandboxResult<()> {
        // ProcessSandbox doesn't support restore - use FirecrackerSandbox for that
        Err(SandboxError::RestoreFailed {
            sandbox_id: self.id.clone(),
            reason: "ProcessSandbox does not support restore. Use FirecrackerSandbox for snapshot/restore capabilities.".to_string(),
        })
    }

    async fn destroy(&mut self) -> SandboxResult<()> {
        let mut state = self.state.write().await;
        *state = SandboxState::Destroying;
        Ok(())
    }

    async fn health_check(&self) -> SandboxResult<bool> {
        let state = self.state.read().await;
        Ok(*state == SandboxState::Running)
    }

    async fn stats(&self) -> SandboxResult<SandboxStats> {
        let start_time = self.start_time.read().await;
        let uptime = start_time
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);

        Ok(SandboxStats {
            memory_bytes_used: 0,
            cpu_percent: 0.0,
            disk_bytes_used: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            uptime_ms: uptime,
        })
    }
}

/// Factory for creating process sandboxes
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessSandboxFactory;

impl ProcessSandboxFactory {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SandboxFactory for ProcessSandboxFactory {
    type Sandbox = ProcessSandbox;

    async fn create(&self, config: SandboxConfig) -> SandboxResult<ProcessSandbox> {
        let mut sandbox = ProcessSandbox::new(config);
        sandbox.start().await?;
        Ok(sandbox)
    }

    async fn create_from_snapshot(
        &self,
        config: SandboxConfig,
        snapshot: &Snapshot,
    ) -> SandboxResult<ProcessSandbox> {
        let mut sandbox = ProcessSandbox::new(config);
        sandbox.restore(snapshot).await?;
        sandbox.start().await?;
        Ok(sandbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SandboxConfig {
        SandboxConfig::default().with_workdir("/tmp")
    }

    #[tokio::test]
    async fn test_process_sandbox_lifecycle() {
        let config = test_config();
        let mut sandbox = ProcessSandbox::new(config);

        assert_eq!(sandbox.state(), SandboxState::Stopped);

        sandbox.start().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Running);

        sandbox.stop().await.unwrap();
        assert_eq!(sandbox.state(), SandboxState::Stopped);
    }

    #[tokio::test]
    async fn test_process_sandbox_exec() {
        let config = test_config();
        let mut sandbox = ProcessSandbox::new(config);
        sandbox.start().await.unwrap();

        let output = sandbox
            .exec_simple("echo", &["hello", "world"])
            .await
            .unwrap();

        assert!(output.is_success());
        assert_eq!(output.stdout_string().trim(), "hello world");
    }

    #[tokio::test]
    async fn test_process_sandbox_exec_failure() {
        let config = test_config();
        let mut sandbox = ProcessSandbox::new(config);
        sandbox.start().await.unwrap();

        let output = sandbox.exec_simple("false", &[]).await.unwrap();
        assert!(!output.is_success());
    }

    #[tokio::test]
    async fn test_process_sandbox_invalid_state() {
        let config = test_config();
        let sandbox = ProcessSandbox::new(config);

        // Can't exec when stopped
        let result = sandbox.exec_simple("echo", &["test"]).await;
        assert!(matches!(result, Err(SandboxError::InvalidState { .. })));
    }
}
