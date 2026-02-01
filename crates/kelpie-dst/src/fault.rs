//! Fault injection for deterministic testing
//!
//! TigerStyle: Explicit fault types, probabilistic injection.

use crate::rng::DeterministicRng;
use std::sync::atomic::{AtomicU64, Ordering};

/// Types of faults that can be injected
///
/// Note: We use PartialEq only (not Eq) because some fault types contain f64
/// fields (e.g., NetworkPacketCorruption::corruption_rate) which don't implement Eq.
#[derive(Debug, Clone, PartialEq)]
pub enum FaultType {
    // Storage faults
    /// Storage write operation fails
    StorageWriteFail,
    /// Storage read operation fails
    StorageReadFail,
    /// Storage data is corrupted
    StorageCorruption,
    /// Storage operations are slow
    StorageLatency { min_ms: u64, max_ms: u64 },
    /// Disk is full
    DiskFull,

    // Crash faults
    /// Crash before completing a write
    CrashBeforeWrite,
    /// Crash after completing a write
    CrashAfterWrite,
    /// Crash during a transaction
    CrashDuringTransaction,

    // Network faults
    /// Network partition (node unreachable)
    NetworkPartition,
    /// Network messages are delayed
    NetworkDelay { min_ms: u64, max_ms: u64 },
    /// Network packets are lost
    NetworkPacketLoss,
    /// Network messages arrive out of order
    NetworkMessageReorder,

    // Time faults
    /// Clock skew between nodes
    ClockSkew { delta_ms: i64 },
    /// Clock jumps forward or backward
    ClockJump { delta_ms: i64 },

    // Resource faults
    /// Out of memory
    OutOfMemory,
    /// CPU starvation
    CPUStarvation,

    // MCP (Model Context Protocol) faults
    /// MCP server process crashes
    McpServerCrash,
    /// MCP server takes too long to start
    McpServerSlowStart { delay_ms: u64 },
    /// MCP tool call times out
    McpToolTimeout,
    /// MCP tool execution fails with error
    McpToolFail,

    // LLM (Language Model) faults
    /// LLM provider request times out
    LlmTimeout,
    /// LLM provider returns error
    LlmFailure,
    /// LLM provider rate limits the request
    LlmRateLimited,
    /// Agent loop panics during execution
    AgentLoopPanic,

    // Multi-agent communication faults (Issue #75)
    /// Called agent doesn't respond in time
    AgentCallTimeout { timeout_ms: u64 },
    /// Called agent refuses the call (e.g., busy, internal error)
    AgentCallRejected { reason: String },
    /// Target agent doesn't exist in the registry
    AgentNotFound { agent_id: String },
    /// Target agent is at max concurrent calls (backpressure)
    AgentBusy { agent_id: String },
    /// Network delay specific to agent-to-agent calls
    AgentCallNetworkDelay { delay_ms: u64 },

    // Sandbox faults (for VM/container isolation)
    /// Sandbox VM fails to boot
    SandboxBootFail,
    /// Sandbox VM crashes unexpectedly
    SandboxCrash,
    /// Sandbox pause operation fails
    SandboxPauseFail,
    /// Sandbox resume operation fails
    SandboxResumeFail,
    /// Sandbox exec operation fails
    SandboxExecFail,
    /// Sandbox exec operation times out
    SandboxExecTimeout { timeout_ms: u64 },

    // Per-agent sandbox isolation faults (for AgentSandboxManager)
    /// Agent's dedicated sandbox pool creation fails
    AgentSandboxPoolCreateFail { agent_id: String },
    /// Agent's dedicated pool is exhausted (min=max=1, sandbox already in use)
    AgentSandboxPoolExhausted { agent_id: String },
    /// Wrong agent receives another agent's sandbox (INVARIANT VIOLATION)
    /// This should never happen in correct code - used for testing isolation.
    AgentSandboxAffinityViolation {
        expected_agent: String,
        actual_agent: String,
    },
    /// Agent sandbox not cleaned up on agent termination
    AgentSandboxLeakOnTerminate { agent_id: String },

    // WASM runtime faults
    /// WASM module compilation fails
    WasmCompileFail,
    /// WASM module instantiation fails
    WasmInstantiateFail,
    /// WASM execution fails
    WasmExecFail,
    /// WASM execution times out (fuel exhausted)
    WasmExecTimeout { timeout_ms: u64 },
    /// WASM cache eviction (for testing cache behavior)
    WasmCacheEvict,

    // Custom tool faults (for sandboxed tool execution)
    /// Custom tool execution fails
    CustomToolExecFail,
    /// Custom tool execution times out
    CustomToolExecTimeout { timeout_ms: u64 },
    /// Custom tool sandbox acquisition fails (pool exhausted)
    CustomToolSandboxAcquireFail,

    // HTTP client faults (for API-calling tools)
    /// HTTP connection fails
    HttpConnectionFail,
    /// HTTP request times out
    HttpTimeout { timeout_ms: u64 },
    /// HTTP server returns error status (4xx/5xx)
    HttpServerError { status: u16 },
    /// HTTP response is too large
    HttpResponseTooLarge { max_bytes: u64 },
    /// HTTP rate limited (429)
    HttpRateLimited { retry_after_ms: u64 },

    // Snapshot faults (for VM state capture)
    /// Snapshot creation fails
    SnapshotCreateFail,
    /// Snapshot data is corrupted
    SnapshotCorruption,
    /// Restore from snapshot fails
    SnapshotRestoreFail,
    /// Snapshot exceeds size limit
    SnapshotTooLarge { max_bytes: u64 },

    // Teleport faults (for cross-machine migration)
    /// Upload to teleport storage fails
    TeleportUploadFail,
    /// Download from teleport storage fails
    TeleportDownloadFail,
    /// Teleport transfer times out
    TeleportTimeout { timeout_ms: u64 },
    /// Architecture mismatch on restore
    TeleportArchMismatch,
    /// Base image version mismatch on restore
    TeleportImageMismatch,

    // ============================================================================
    // FoundationDB-Critical Fault Types (Issue #36)
    // These fault types are critical for testing production distributed systems
    // ============================================================================

    // Storage semantics faults (HIGH priority)
    /// Misdirected I/O - write goes to wrong address
    /// This simulates disk-level bugs where data is written to the wrong location.
    /// The target_key specifies where the data actually goes.
    StorageMisdirectedWrite { target_key: Vec<u8> },
    /// Partial write - only some bytes written before failure
    /// This simulates disk/SSD failures mid-write where only part of the data persists.
    StoragePartialWrite { bytes_written: usize },
    /// Fsync failure - metadata not persisted
    /// This simulates fsync() returning an error, meaning data may be buffered but not durable.
    StorageFsyncFail,
    /// Data loss on crash - unflushed buffers lost
    /// This simulates process crash where OS buffers haven't been flushed to disk.
    StorageUnflushedLoss,

    // Distributed coordination faults (HIGH priority)
    /// Split-brain - cluster partitions operate independently
    /// Both partition_a and partition_b nodes believe they are the primary cluster.
    ClusterSplitBrain {
        partition_a: Vec<String>,
        partition_b: Vec<String>,
    },
    /// Replication lag - replica falls behind primary
    /// The lag_ms indicates how far behind the replica is.
    ReplicationLag { lag_ms: u64 },
    /// Quorum loss - not enough nodes for consensus
    /// available_nodes is less than required for quorum operations.
    QuorumLoss {
        available_nodes: usize,
        required_nodes: usize,
    },

    // Infrastructure faults (MEDIUM priority)
    /// Packet corruption - data is corrupted in transit (not just lost)
    /// Unlike NetworkPacketLoss, the packet arrives but with corrupted bytes.
    NetworkPacketCorruption { corruption_rate: f64 },
    /// Network jitter - variance in delays (unpredictable latency)
    /// More realistic than uniform delay - uses normal distribution.
    NetworkJitter { mean_ms: u64, stddev_ms: u64 },
    /// Connection exhaustion - too many open connections
    /// Simulates running out of available network connections.
    NetworkConnectionExhaustion,
    /// File descriptor exhaustion - too many open files
    /// Simulates hitting the system fd limit.
    ResourceFdExhaustion,
}

impl FaultType {
    /// Get a human-readable name for this fault type
    pub fn name(&self) -> &'static str {
        match self {
            FaultType::StorageWriteFail => "storage_write_fail",
            FaultType::StorageReadFail => "storage_read_fail",
            FaultType::StorageCorruption => "storage_corruption",
            FaultType::StorageLatency { .. } => "storage_latency",
            FaultType::DiskFull => "disk_full",
            FaultType::CrashBeforeWrite => "crash_before_write",
            FaultType::CrashAfterWrite => "crash_after_write",
            FaultType::CrashDuringTransaction => "crash_during_transaction",
            FaultType::NetworkPartition => "network_partition",
            FaultType::NetworkDelay { .. } => "network_delay",
            FaultType::NetworkPacketLoss => "network_packet_loss",
            FaultType::NetworkMessageReorder => "network_message_reorder",
            FaultType::ClockSkew { .. } => "clock_skew",
            FaultType::ClockJump { .. } => "clock_jump",
            FaultType::OutOfMemory => "out_of_memory",
            FaultType::CPUStarvation => "cpu_starvation",
            FaultType::McpServerCrash => "mcp_server_crash",
            FaultType::McpServerSlowStart { .. } => "mcp_server_slow_start",
            FaultType::McpToolTimeout => "mcp_tool_timeout",
            FaultType::McpToolFail => "mcp_tool_fail",
            FaultType::LlmTimeout => "llm_timeout",
            FaultType::LlmFailure => "llm_failure",
            FaultType::LlmRateLimited => "llm_rate_limited",
            FaultType::AgentLoopPanic => "agent_loop_panic",
            // Multi-agent communication faults
            FaultType::AgentCallTimeout { .. } => "agent_call_timeout",
            FaultType::AgentCallRejected { .. } => "agent_call_rejected",
            FaultType::AgentNotFound { .. } => "agent_not_found",
            FaultType::AgentBusy { .. } => "agent_busy",
            FaultType::AgentCallNetworkDelay { .. } => "agent_call_network_delay",
            // Sandbox faults
            FaultType::SandboxBootFail => "sandbox_boot_fail",
            FaultType::SandboxCrash => "sandbox_crash",
            FaultType::SandboxPauseFail => "sandbox_pause_fail",
            FaultType::SandboxResumeFail => "sandbox_resume_fail",
            FaultType::SandboxExecFail => "sandbox_exec_fail",
            FaultType::SandboxExecTimeout { .. } => "sandbox_exec_timeout",
            // Per-agent sandbox isolation faults
            FaultType::AgentSandboxPoolCreateFail { .. } => "agent_sandbox_pool_create_fail",
            FaultType::AgentSandboxPoolExhausted { .. } => "agent_sandbox_pool_exhausted",
            FaultType::AgentSandboxAffinityViolation { .. } => "agent_sandbox_affinity_violation",
            FaultType::AgentSandboxLeakOnTerminate { .. } => "agent_sandbox_leak_on_terminate",
            // WASM runtime faults
            FaultType::WasmCompileFail => "wasm_compile_fail",
            FaultType::WasmInstantiateFail => "wasm_instantiate_fail",
            FaultType::WasmExecFail => "wasm_exec_fail",
            FaultType::WasmExecTimeout { .. } => "wasm_exec_timeout",
            FaultType::WasmCacheEvict => "wasm_cache_evict",
            // Custom tool faults
            FaultType::CustomToolExecFail => "custom_tool_exec_fail",
            FaultType::CustomToolExecTimeout { .. } => "custom_tool_exec_timeout",
            FaultType::CustomToolSandboxAcquireFail => "custom_tool_sandbox_acquire_fail",
            // HTTP client faults
            FaultType::HttpConnectionFail => "http_connection_fail",
            FaultType::HttpTimeout { .. } => "http_timeout",
            FaultType::HttpServerError { .. } => "http_server_error",
            FaultType::HttpResponseTooLarge { .. } => "http_response_too_large",
            FaultType::HttpRateLimited { .. } => "http_rate_limited",
            // Snapshot faults
            FaultType::SnapshotCreateFail => "snapshot_create_fail",
            FaultType::SnapshotCorruption => "snapshot_corruption",
            FaultType::SnapshotRestoreFail => "snapshot_restore_fail",
            FaultType::SnapshotTooLarge { .. } => "snapshot_too_large",
            // Teleport faults
            FaultType::TeleportUploadFail => "teleport_upload_fail",
            FaultType::TeleportDownloadFail => "teleport_download_fail",
            FaultType::TeleportTimeout { .. } => "teleport_timeout",
            FaultType::TeleportArchMismatch => "teleport_arch_mismatch",
            FaultType::TeleportImageMismatch => "teleport_image_mismatch",
            // FoundationDB-critical storage semantics faults
            FaultType::StorageMisdirectedWrite { .. } => "storage_misdirected_write",
            FaultType::StoragePartialWrite { .. } => "storage_partial_write",
            FaultType::StorageFsyncFail => "storage_fsync_fail",
            FaultType::StorageUnflushedLoss => "storage_unflushed_loss",
            // FoundationDB-critical distributed coordination faults
            FaultType::ClusterSplitBrain { .. } => "cluster_split_brain",
            FaultType::ReplicationLag { .. } => "replication_lag",
            FaultType::QuorumLoss { .. } => "quorum_loss",
            // FoundationDB-critical infrastructure faults
            FaultType::NetworkPacketCorruption { .. } => "network_packet_corruption",
            FaultType::NetworkJitter { .. } => "network_jitter",
            FaultType::NetworkConnectionExhaustion => "network_connection_exhaustion",
            FaultType::ResourceFdExhaustion => "resource_fd_exhaustion",
        }
    }
}

/// Configuration for a fault injection rule
#[derive(Debug, Clone)]
pub struct FaultConfig {
    /// The type of fault to inject
    pub fault_type: FaultType,
    /// Probability of injection (0.0 - 1.0)
    pub probability: f64,
    /// Optional filter for operation names
    pub operation_filter: Option<String>,
    /// Only trigger after this many operations
    pub after_operations: u64,
    /// Maximum number of times to trigger
    pub max_triggers: Option<u64>,
    /// Whether this fault is enabled
    pub enabled: bool,
}

impl FaultConfig {
    /// Create a new fault configuration
    pub fn new(fault_type: FaultType, probability: f64) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&probability),
            "probability must be in [0, 1]"
        );

        Self {
            fault_type,
            probability,
            operation_filter: None,
            after_operations: 0,
            max_triggers: None,
            enabled: true,
        }
    }

    /// Set an operation filter
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.operation_filter = Some(filter.into());
        self
    }

    /// Set the number of operations to wait before triggering
    pub fn after(mut self, operations: u64) -> Self {
        self.after_operations = operations;
        self
    }

    /// Set the maximum number of triggers
    pub fn max_triggers(mut self, max: u64) -> Self {
        self.max_triggers = Some(max);
        self
    }

    /// Disable this fault
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Fault injector for deterministic testing
#[derive(Debug)]
pub struct FaultInjector {
    /// Registered fault configurations
    faults: Vec<FaultState>,
    /// RNG for probabilistic injection
    rng: DeterministicRng,
    /// Total operations processed
    operation_count: AtomicU64,
}

/// State for a registered fault
#[derive(Debug)]
struct FaultState {
    config: FaultConfig,
    trigger_count: AtomicU64,
}

impl FaultInjector {
    /// Create a new fault injector with the given RNG
    pub fn new(rng: DeterministicRng) -> Self {
        Self {
            faults: Vec::new(),
            rng,
            operation_count: AtomicU64::new(0),
        }
    }

    /// Register a fault configuration
    pub fn register(&mut self, config: FaultConfig) {
        self.faults.push(FaultState {
            config,
            trigger_count: AtomicU64::new(0),
        });
    }

    /// Check if a fault should be injected for the given operation
    ///
    /// Returns the fault type if injection should occur, None otherwise.
    pub fn should_inject(&self, operation: &str) -> Option<FaultType> {
        let op_count = self.operation_count.fetch_add(1, Ordering::SeqCst);

        for fault_state in &self.faults {
            let config = &fault_state.config;

            // Skip disabled faults
            if !config.enabled {
                continue;
            }

            // Check operation filter
            if let Some(filter) = &config.operation_filter {
                if !operation.contains(filter) {
                    continue;
                }
            }

            // Check operation count threshold
            if op_count < config.after_operations {
                continue;
            }

            // Probabilistic check
            if self.rng.next_bool(config.probability) {
                // Use compare_exchange loop to atomically check max_triggers and increment
                // This avoids TOCTOU race between checking trigger_count and incrementing it
                if let Some(max) = config.max_triggers {
                    loop {
                        let current = fault_state.trigger_count.load(Ordering::SeqCst);
                        if current >= max {
                            // Already at max triggers, skip this fault
                            break;
                        }
                        // Try to atomically increment
                        match fault_state.trigger_count.compare_exchange(
                            current,
                            current + 1,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        ) {
                            Ok(_) => {
                                // Successfully incremented, trigger the fault
                                let trigger_count = current + 1;

                                tracing::debug!(
                                    fault = config.fault_type.name(),
                                    operation = operation,
                                    trigger_count = trigger_count,
                                    "Injecting fault"
                                );

                                return Some(config.fault_type.clone());
                            }
                            Err(_) => {
                                // Another thread modified it, retry
                                continue;
                            }
                        }
                    }
                    // If we broke out of the loop, we hit max triggers
                    continue;
                }

                // No max_triggers limit, just increment and trigger
                let trigger_count = fault_state.trigger_count.fetch_add(1, Ordering::SeqCst);

                tracing::debug!(
                    fault = config.fault_type.name(),
                    operation = operation,
                    trigger_count = trigger_count + 1,
                    "Injecting fault"
                );

                return Some(config.fault_type.clone());
            }
        }

        None
    }

    /// Get the total number of operations processed
    pub fn operation_count(&self) -> u64 {
        self.operation_count.load(Ordering::SeqCst)
    }

    /// Get statistics for all registered faults
    pub fn stats(&self) -> Vec<FaultStats> {
        self.faults
            .iter()
            .map(|state| FaultStats {
                fault_type: state.config.fault_type.name().to_string(),
                probability: state.config.probability,
                trigger_count: state.trigger_count.load(Ordering::SeqCst),
                enabled: state.config.enabled,
            })
            .collect()
    }

    /// Reset all trigger counts
    pub fn reset(&self) {
        self.operation_count.store(0, Ordering::SeqCst);
        for fault_state in &self.faults {
            fault_state.trigger_count.store(0, Ordering::SeqCst);
        }
    }
}

/// Statistics for a fault configuration
#[derive(Debug, Clone)]
pub struct FaultStats {
    pub fault_type: String,
    pub probability: f64,
    pub trigger_count: u64,
    pub enabled: bool,
}

/// Builder for creating a FaultInjector with multiple faults
pub struct FaultInjectorBuilder {
    rng: DeterministicRng,
    faults: Vec<FaultConfig>,
}

impl FaultInjectorBuilder {
    /// Create a new builder
    pub fn new(rng: DeterministicRng) -> Self {
        Self {
            rng,
            faults: Vec::new(),
        }
    }

    /// Add a fault configuration
    pub fn with_fault(mut self, config: FaultConfig) -> Self {
        self.faults.push(config);
        self
    }

    /// Add storage faults with default probabilities
    pub fn with_storage_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::StorageWriteFail, probability))
            .with_fault(FaultConfig::new(FaultType::StorageReadFail, probability))
    }

    /// Add network faults with default probabilities
    pub fn with_network_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::NetworkPacketLoss, probability))
            .with_fault(FaultConfig::new(
                FaultType::NetworkDelay {
                    min_ms: 10,
                    max_ms: 100,
                },
                probability,
            ))
    }

    /// Add crash faults with default probabilities
    ///
    /// TigerStyle: Crash faults MUST be filtered to write/transaction operations.
    /// CrashAfterWrite should never trigger on read operations.
    pub fn with_crash_faults(self, probability: f64) -> Self {
        self.with_fault(
            FaultConfig::new(FaultType::CrashAfterWrite, probability).with_filter("write"),
        )
        .with_fault(
            FaultConfig::new(FaultType::CrashDuringTransaction, probability)
                .with_filter("transaction"),
        )
    }

    /// Add MCP (Model Context Protocol) faults with default probabilities
    pub fn with_mcp_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::McpServerCrash, probability))
            .with_fault(FaultConfig::new(FaultType::McpToolFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::McpToolTimeout,
                probability / 2.0,
            ))
    }

    /// Add LLM (Language Model) faults with default probabilities
    pub fn with_llm_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::LlmTimeout, probability / 2.0))
            .with_fault(FaultConfig::new(FaultType::LlmFailure, probability))
            .with_fault(FaultConfig::new(
                FaultType::LlmRateLimited,
                probability / 3.0,
            ))
    }

    /// Add sandbox faults with default probabilities
    pub fn with_sandbox_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::SandboxBootFail, probability))
            .with_fault(FaultConfig::new(FaultType::SandboxCrash, probability))
            .with_fault(FaultConfig::new(FaultType::SandboxPauseFail, probability))
            .with_fault(FaultConfig::new(FaultType::SandboxResumeFail, probability))
            .with_fault(FaultConfig::new(FaultType::SandboxExecFail, probability))
    }

    /// Add per-agent sandbox isolation faults
    ///
    /// These faults test the AgentSandboxManager:
    /// - Pool creation failures
    /// - Pool exhaustion (dedicated mode with max=1)
    pub fn with_agent_sandbox_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(
            FaultType::AgentSandboxPoolCreateFail {
                agent_id: "any".to_string(),
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::AgentSandboxPoolExhausted {
                agent_id: "any".to_string(),
            },
            probability,
        ))
    }

    /// Add WASM runtime faults with default probabilities
    ///
    /// These faults simulate failures in WASM module execution:
    /// - Compilation failures (invalid WASM bytecode)
    /// - Instantiation failures (linking errors)
    /// - Execution failures (runtime errors)
    /// - Execution timeouts (fuel exhausted)
    /// - Cache evictions (for testing cache behavior)
    pub fn with_wasm_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::WasmCompileFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::WasmInstantiateFail,
                probability,
            ))
            .with_fault(FaultConfig::new(FaultType::WasmExecFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::WasmExecTimeout { timeout_ms: 30_000 },
                probability / 2.0,
            ))
            .with_fault(FaultConfig::new(
                FaultType::WasmCacheEvict,
                probability / 3.0,
            ))
    }

    /// Add custom tool execution faults with default probabilities
    ///
    /// These faults simulate failures in custom tool execution:
    /// - Execution failures (script errors)
    /// - Execution timeouts (tool takes too long)
    /// - Sandbox acquisition failures (pool exhausted)
    pub fn with_custom_tool_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::CustomToolExecFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::CustomToolExecTimeout { timeout_ms: 30_000 },
                probability / 2.0,
            ))
            .with_fault(FaultConfig::new(
                FaultType::CustomToolSandboxAcquireFail,
                probability / 3.0,
            ))
    }

    /// Add HTTP client faults with default probabilities
    ///
    /// These faults simulate failures in HTTP API calls:
    /// - Connection failures (network issues)
    /// - Timeouts (slow server)
    /// - Server errors (5xx responses)
    /// - Response too large
    /// - Rate limiting (429 responses)
    pub fn with_http_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::HttpConnectionFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::HttpTimeout { timeout_ms: 30_000 },
                probability / 2.0,
            ))
            .with_fault(FaultConfig::new(
                FaultType::HttpServerError { status: 500 },
                probability,
            ))
            .with_fault(FaultConfig::new(
                FaultType::HttpResponseTooLarge {
                    max_bytes: 10 * 1024 * 1024,
                },
                probability / 3.0,
            ))
            .with_fault(FaultConfig::new(
                FaultType::HttpRateLimited {
                    retry_after_ms: 60_000,
                },
                probability / 2.0,
            ))
    }

    /// Add snapshot faults with default probabilities
    pub fn with_snapshot_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::SnapshotCreateFail, probability))
            .with_fault(FaultConfig::new(FaultType::SnapshotCorruption, probability))
            .with_fault(FaultConfig::new(
                FaultType::SnapshotRestoreFail,
                probability,
            ))
    }

    /// Add teleport faults with default probabilities
    pub fn with_teleport_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(FaultType::TeleportUploadFail, probability))
            .with_fault(FaultConfig::new(
                FaultType::TeleportDownloadFail,
                probability,
            ))
            .with_fault(FaultConfig::new(
                FaultType::TeleportArchMismatch,
                probability / 2.0,
            ))
    }

    /// Add multi-agent communication faults (Issue #75)
    ///
    /// These faults simulate failures in agent-to-agent communication:
    /// - Timeout (called agent doesn't respond)
    /// - Rejection (called agent refuses the call)
    /// - Not found (target agent doesn't exist)
    /// - Busy (target agent at max concurrent calls)
    /// - Network delay (specific to agent calls)
    pub fn with_multi_agent_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(
            FaultType::AgentCallTimeout { timeout_ms: 30_000 },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::AgentCallRejected {
                reason: "simulated_rejection".to_string(),
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::AgentNotFound {
                agent_id: "simulated_missing".to_string(),
            },
            probability / 2.0,
        ))
        .with_fault(FaultConfig::new(
            FaultType::AgentBusy {
                agent_id: "simulated_busy".to_string(),
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::AgentCallNetworkDelay { delay_ms: 100 },
            probability,
        ))
    }

    // =========================================================================
    // FoundationDB-Critical Fault Builders (Issue #36)
    // =========================================================================

    /// Add storage semantics faults (FoundationDB-critical)
    ///
    /// These faults simulate disk-level failures that production databases must handle:
    /// - Misdirected writes (data goes to wrong location)
    /// - Partial writes (only some bytes written)
    /// - Fsync failures (metadata not persisted)
    pub fn with_storage_semantics_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(
            FaultType::StorageMisdirectedWrite {
                target_key: b"__misdirected__".to_vec(),
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::StoragePartialWrite { bytes_written: 0 },
            probability,
        ))
        .with_fault(FaultConfig::new(FaultType::StorageFsyncFail, probability))
        .with_fault(FaultConfig::new(
            FaultType::StorageUnflushedLoss,
            probability / 2.0,
        ))
    }

    /// Add distributed coordination faults (FoundationDB-critical)
    ///
    /// These faults simulate cluster-level failures:
    /// - Split-brain scenarios
    /// - Replication lag
    /// - Quorum loss
    ///
    /// Note: These are marker faults - actual implementation depends on
    /// your cluster simulation. Use them to trigger cluster-level behaviors.
    pub fn with_coordination_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(
            FaultType::ClusterSplitBrain {
                partition_a: vec!["node-1".into(), "node-2".into()],
                partition_b: vec!["node-3".into()],
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::ReplicationLag { lag_ms: 1000 },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::QuorumLoss {
                available_nodes: 1,
                required_nodes: 2,
            },
            probability,
        ))
    }

    /// Add infrastructure faults (FoundationDB-critical)
    ///
    /// These faults simulate infrastructure-level failures:
    /// - Packet corruption (not just loss)
    /// - Network jitter (unpredictable latency)
    /// - Connection exhaustion
    /// - File descriptor exhaustion
    pub fn with_infrastructure_faults(self, probability: f64) -> Self {
        self.with_fault(FaultConfig::new(
            FaultType::NetworkPacketCorruption {
                corruption_rate: 0.1,
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::NetworkJitter {
                mean_ms: 50,
                stddev_ms: 25,
            },
            probability,
        ))
        .with_fault(FaultConfig::new(
            FaultType::NetworkConnectionExhaustion,
            probability / 2.0,
        ))
        .with_fault(FaultConfig::new(
            FaultType::ResourceFdExhaustion,
            probability / 2.0,
        ))
    }

    /// Build the fault injector
    pub fn build(self) -> FaultInjector {
        let mut injector = FaultInjector::new(self.rng);
        for config in self.faults {
            injector.register(config);
        }
        injector
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_injection_probability() {
        let rng = DeterministicRng::new(42);
        let mut injector = FaultInjector::new(rng);

        // 100% probability should always inject
        injector.register(FaultConfig::new(FaultType::StorageWriteFail, 1.0));

        for _ in 0..10 {
            assert!(injector.should_inject("write").is_some());
        }
    }

    #[test]
    fn test_fault_injection_zero_probability() {
        let rng = DeterministicRng::new(42);
        let mut injector = FaultInjector::new(rng);

        // 0% probability should never inject
        injector.register(FaultConfig::new(FaultType::StorageWriteFail, 0.0));

        for _ in 0..100 {
            assert!(injector.should_inject("write").is_none());
        }
    }

    #[test]
    fn test_fault_injection_filter() {
        let rng = DeterministicRng::new(42);
        let mut injector = FaultInjector::new(rng);

        // Filter for "write" operations only
        injector.register(FaultConfig::new(FaultType::StorageWriteFail, 1.0).with_filter("write"));

        // Should inject for write
        assert!(injector.should_inject("storage_write").is_some());

        // Should not inject for read
        assert!(injector.should_inject("storage_read").is_none());
    }

    #[test]
    fn test_fault_injection_max_triggers() {
        let rng = DeterministicRng::new(42);
        let mut injector = FaultInjector::new(rng);

        injector.register(FaultConfig::new(FaultType::StorageWriteFail, 1.0).max_triggers(2));

        // First two should trigger
        assert!(injector.should_inject("write").is_some());
        assert!(injector.should_inject("write").is_some());

        // Third should not
        assert!(injector.should_inject("write").is_none());
    }

    #[test]
    fn test_fault_injector_builder() {
        let rng = DeterministicRng::new(42);
        let injector = FaultInjectorBuilder::new(rng)
            .with_storage_faults(0.1)
            .with_network_faults(0.05)
            .build();

        let stats = injector.stats();
        assert_eq!(stats.len(), 4); // 2 storage + 2 network
    }

    #[test]
    fn test_fault_type_names() {
        assert_eq!(FaultType::StorageWriteFail.name(), "storage_write_fail");
        assert_eq!(FaultType::NetworkPartition.name(), "network_partition");
        assert_eq!(FaultType::ClockSkew { delta_ms: 100 }.name(), "clock_skew");
    }

    #[test]
    fn test_fdb_critical_fault_type_names() {
        // Storage semantics faults
        assert_eq!(
            FaultType::StorageMisdirectedWrite {
                target_key: vec![1, 2, 3]
            }
            .name(),
            "storage_misdirected_write"
        );
        assert_eq!(
            FaultType::StoragePartialWrite { bytes_written: 10 }.name(),
            "storage_partial_write"
        );
        assert_eq!(FaultType::StorageFsyncFail.name(), "storage_fsync_fail");
        assert_eq!(
            FaultType::StorageUnflushedLoss.name(),
            "storage_unflushed_loss"
        );

        // Distributed coordination faults
        assert_eq!(
            FaultType::ClusterSplitBrain {
                partition_a: vec!["a".into()],
                partition_b: vec!["b".into()],
            }
            .name(),
            "cluster_split_brain"
        );
        assert_eq!(
            FaultType::ReplicationLag { lag_ms: 100 }.name(),
            "replication_lag"
        );
        assert_eq!(
            FaultType::QuorumLoss {
                available_nodes: 1,
                required_nodes: 3
            }
            .name(),
            "quorum_loss"
        );

        // Infrastructure faults
        assert_eq!(
            FaultType::NetworkPacketCorruption {
                corruption_rate: 0.1
            }
            .name(),
            "network_packet_corruption"
        );
        assert_eq!(
            FaultType::NetworkJitter {
                mean_ms: 50,
                stddev_ms: 25
            }
            .name(),
            "network_jitter"
        );
        assert_eq!(
            FaultType::NetworkConnectionExhaustion.name(),
            "network_connection_exhaustion"
        );
        assert_eq!(
            FaultType::ResourceFdExhaustion.name(),
            "resource_fd_exhaustion"
        );
    }

    #[test]
    fn test_fault_injector_builder_fdb_faults() {
        let rng = DeterministicRng::new(42);

        // Test storage semantics faults builder
        let injector = FaultInjectorBuilder::new(rng.fork())
            .with_storage_semantics_faults(0.1)
            .build();
        let stats = injector.stats();
        assert_eq!(stats.len(), 4); // misdirected, partial, fsync, unflushed

        // Test coordination faults builder
        let injector = FaultInjectorBuilder::new(rng.fork())
            .with_coordination_faults(0.1)
            .build();
        let stats = injector.stats();
        assert_eq!(stats.len(), 3); // split-brain, replication lag, quorum loss

        // Test infrastructure faults builder
        let injector = FaultInjectorBuilder::new(rng.fork())
            .with_infrastructure_faults(0.1)
            .build();
        let stats = injector.stats();
        assert_eq!(stats.len(), 4); // corruption, jitter, conn exhaustion, fd exhaustion
    }

    #[test]
    fn test_multi_agent_fault_type_names() {
        // Multi-agent communication faults (Issue #75)
        assert_eq!(
            FaultType::AgentCallTimeout { timeout_ms: 30_000 }.name(),
            "agent_call_timeout"
        );
        assert_eq!(
            FaultType::AgentCallRejected {
                reason: "test".into()
            }
            .name(),
            "agent_call_rejected"
        );
        assert_eq!(
            FaultType::AgentNotFound {
                agent_id: "test".into()
            }
            .name(),
            "agent_not_found"
        );
        assert_eq!(
            FaultType::AgentBusy {
                agent_id: "test".into()
            }
            .name(),
            "agent_busy"
        );
        assert_eq!(
            FaultType::AgentCallNetworkDelay { delay_ms: 100 }.name(),
            "agent_call_network_delay"
        );
    }

    #[test]
    fn test_fault_injector_builder_multi_agent_faults() {
        let rng = DeterministicRng::new(42);

        // Test multi-agent faults builder
        let injector = FaultInjectorBuilder::new(rng.fork())
            .with_multi_agent_faults(0.1)
            .build();
        let stats = injector.stats();
        assert_eq!(stats.len(), 5); // timeout, rejected, not_found, busy, network_delay
    }
}
