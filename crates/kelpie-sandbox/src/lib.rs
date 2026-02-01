//! Sandbox execution environment for Kelpie agents
//!
//! TigerStyle: Secure isolation with explicit lifecycle and state management.
//!
//! # Overview
//!
//! Sandboxes provide isolated execution environments for agent tools:
//! - Process/VM isolation for security
//! - Filesystem and network isolation
//! - State checkpointing (pause/resume)
//! - Resource limits (CPU, memory, disk)
//!
//! # Sandbox Types
//!
//! - **MockSandbox**: In-memory simulation for testing
//! - **ProcessSandbox**: OS process isolation (cross-platform)
//! - **FirecrackerSandbox**: MicroVM isolation (Linux only, feature-gated)
//!
//! # Usage
//!
//! ```ignore
//! use kelpie_sandbox::{SandboxPool, SandboxConfig};
//!
//! // Create a pool of pre-warmed sandboxes
//! let pool = SandboxPool::new(SandboxConfig::default(), 10).await?;
//!
//! // Acquire a sandbox
//! let sandbox = pool.acquire().await?;
//!
//! // Execute a command
//! let output = sandbox.exec("echo", &["hello"]).await?;
//!
//! // Release back to pool (or let it drop)
//! pool.release(sandbox).await;
//! ```

mod agent_manager;
mod config;
mod error;
mod exec;
pub mod io;
mod mock;
mod pool;
mod process;
mod snapshot;
mod traits;

#[cfg(feature = "firecracker")]
mod firecracker;

pub use agent_manager::{
    AgentSandboxManager, IsolationMode, AGENT_POOL_ACQUIRE_TIMEOUT_MS_DEFAULT,
};
pub use config::{ResourceLimits, SandboxConfig};
pub use error::{SandboxError, SandboxResult};
pub use exec::{ExecOptions, ExecOutput, ExitStatus};
pub use mock::{MockSandbox, MockSandboxFactory};
pub use pool::{PoolConfig, SandboxPool};
pub use process::{ProcessSandbox, ProcessSandboxFactory};
pub use snapshot::{
    Architecture, Snapshot, SnapshotKind, SnapshotMetadata, SnapshotValidationError,
    SNAPSHOT_CHECKPOINT_SIZE_BYTES_MAX, SNAPSHOT_FORMAT_VERSION, SNAPSHOT_SUSPEND_SIZE_BYTES_MAX,
    SNAPSHOT_TELEPORT_SIZE_BYTES_MAX,
};
pub use traits::{Sandbox, SandboxFactory, SandboxState, SandboxStats};

// I/O Abstraction for DST
pub use io::{GenericSandbox, SandboxIO, SnapshotData};

#[cfg(feature = "firecracker")]
pub use firecracker::{
    FirecrackerConfig, FirecrackerSandbox, FirecrackerSandboxFactory,
    FIRECRACKER_API_TIMEOUT_MS_DEFAULT, FIRECRACKER_BINARY_PATH_DEFAULT,
    FIRECRACKER_BOOT_TIMEOUT_MS_DEFAULT, FIRECRACKER_VSOCK_CID_DEFAULT,
    FIRECRACKER_VSOCK_PORT_DEFAULT,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_module_compiles() {
        // Smoke test
        let _config = SandboxConfig::default();
    }
}
