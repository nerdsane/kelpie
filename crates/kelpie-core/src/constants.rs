//! TigerStyle constants for Kelpie
//!
//! All limits are explicit, use big-endian naming (most significant first),
//! and include units in the name.

// =============================================================================
// Actor Limits
// =============================================================================

/// Maximum length of an actor ID in bytes
pub const ACTOR_ID_LENGTH_BYTES_MAX: usize = 256;

/// Maximum length of an actor namespace in bytes
pub const ACTOR_NAMESPACE_LENGTH_BYTES_MAX: usize = 128;

/// Maximum size of actor state in bytes (10 MB)
pub const ACTOR_STATE_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024;

/// Maximum size of actor KV key in bytes (10 KB)
pub const ACTOR_KV_KEY_SIZE_BYTES_MAX: usize = 10 * 1024;

/// Maximum size of actor KV value in bytes (1 MB)
pub const ACTOR_KV_VALUE_SIZE_BYTES_MAX: usize = 1024 * 1024;

/// Maximum duration for an actor invocation in milliseconds (2 min)
/// TigerStyle: LLM API calls (especially with tool use) can take 30-60+ seconds.
/// 120 seconds provides margin for slow API responses while preventing runaway tasks.
pub const ACTOR_INVOCATION_TIMEOUT_MS_MAX: u64 = 120 * 1000;

/// Default idle timeout before actor deactivation in milliseconds (5 min)
pub const ACTOR_IDLE_TIMEOUT_MS_DEFAULT: u64 = 5 * 60 * 1000;

/// Maximum idle timeout in milliseconds (1 hour)
pub const ACTOR_IDLE_TIMEOUT_MS_MAX: u64 = 60 * 60 * 1000;

/// Maximum number of concurrent actors per node
pub const ACTOR_CONCURRENT_COUNT_MAX: usize = 1_000_000;

// =============================================================================
// Transaction Limits (aligned with FoundationDB)
// =============================================================================

/// Maximum size of a transaction in bytes (10 MB - FDB limit)
pub const TRANSACTION_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024;

/// Maximum number of keys in a single transaction
pub const TRANSACTION_KEYS_COUNT_MAX: usize = 10_000;

/// Maximum key size in bytes
pub const TRANSACTION_KEY_SIZE_BYTES_MAX: usize = 10 * 1024;

/// Maximum value size in bytes
pub const TRANSACTION_VALUE_SIZE_BYTES_MAX: usize = 100 * 1024;

/// Maximum transaction timeout in milliseconds (5 sec)
pub const TRANSACTION_TIMEOUT_MS_MAX: u64 = 5 * 1000;

/// Default transaction timeout in milliseconds (2 sec)
pub const TRANSACTION_TIMEOUT_MS_DEFAULT: u64 = 2 * 1000;

// =============================================================================
// Cluster Limits
// =============================================================================

/// Maximum number of nodes in a cluster
pub const CLUSTER_NODES_COUNT_MAX: usize = 1000;

/// Heartbeat interval in milliseconds (1 sec)
pub const HEARTBEAT_INTERVAL_MS: u64 = 1000;

/// Heartbeat timeout before node is considered failed (5 sec)
pub const HEARTBEAT_TIMEOUT_MS: u64 = 5000;

/// Maximum rate of actor activations per second per node
pub const ACTOR_ACTIVATION_RATE_PER_SEC_MAX: u64 = 100_000;

/// Minimum time between migration attempts for same actor (10 sec)
pub const ACTOR_MIGRATION_COOLDOWN_MS: u64 = 10 * 1000;

// =============================================================================
// Message Limits
// =============================================================================

/// Maximum size of a message payload in bytes (1 MB)
pub const MESSAGE_SIZE_BYTES_MAX: usize = 1024 * 1024;

/// Maximum depth of actor mailbox
pub const MAILBOX_DEPTH_MAX: usize = 10_000;

/// Maximum number of pending invocations per actor
pub const INVOCATION_PENDING_COUNT_MAX: usize = 1000;

// =============================================================================
// Network Limits
// =============================================================================

/// Maximum number of concurrent RPC connections per node
pub const RPC_CONNECTIONS_COUNT_MAX: usize = 10_000;

/// RPC request timeout in milliseconds (30 sec)
pub const RPC_TIMEOUT_MS_DEFAULT: u64 = 30 * 1000;

/// Maximum RPC message size in bytes (16 MB)
pub const RPC_MESSAGE_SIZE_BYTES_MAX: usize = 16 * 1024 * 1024;

// =============================================================================
// WASM Limits
// =============================================================================

/// Maximum WASM module size in bytes (100 MB)
pub const WASM_MODULE_SIZE_BYTES_MAX: usize = 100 * 1024 * 1024;

/// Maximum WASM memory per actor in bytes (256 MB)
pub const WASM_MEMORY_SIZE_BYTES_MAX: usize = 256 * 1024 * 1024;

/// WASM execution timeout in milliseconds (30 sec)
pub const WASM_EXECUTION_TIMEOUT_MS_MAX: u64 = 30 * 1000;

// =============================================================================
// DST Limits
// =============================================================================

/// Maximum simulation steps before forced termination
pub const DST_STEPS_COUNT_MAX: u64 = 10_000_000;

/// Maximum simulated time in milliseconds (24 hours)
pub const DST_TIME_MS_MAX: u64 = 24 * 60 * 60 * 1000;

/// Default fault injection probability
pub const DST_FAULT_PROBABILITY_DEFAULT: f64 = 0.01;

// =============================================================================
// Observability - Metric Names (TigerStyle: explicit, with units)
// =============================================================================

/// Metric: Current number of active agents (gauge)
pub const METRIC_NAME_AGENTS_ACTIVE_COUNT: &str = "kelpie_agents_active_count";

/// Metric: Total number of agent activations (counter)
pub const METRIC_NAME_AGENTS_ACTIVATED_TOTAL: &str = "kelpie_agents_activated_total";

/// Metric: Total number of agent deactivations (counter)
pub const METRIC_NAME_AGENTS_DEACTIVATED_TOTAL: &str = "kelpie_agents_deactivated_total";

/// Metric: Total number of invocations (counter, labels: operation, status)
pub const METRIC_NAME_INVOCATIONS_TOTAL: &str = "kelpie_invocations_total";

/// Metric: Invocation duration in seconds (histogram)
pub const METRIC_NAME_INVOCATION_DURATION_SECONDS: &str = "kelpie_invocation_duration_seconds";

/// Metric: Current number of pending invocations (gauge)
pub const METRIC_NAME_INVOCATIONS_PENDING_COUNT: &str = "kelpie_invocations_pending_count";

/// Metric: Memory usage in bytes (gauge, labels: tier)
pub const METRIC_NAME_MEMORY_USAGE_BYTES: &str = "kelpie_memory_usage_bytes";

/// Metric: Total number of memory blocks (gauge)
pub const METRIC_NAME_MEMORY_BLOCKS_TOTAL: &str = "kelpie_memory_blocks_total";

/// Metric: Storage operation duration in seconds (histogram, labels: operation)
pub const METRIC_NAME_STORAGE_DURATION_SECONDS: &str = "kelpie_storage_duration_seconds";

/// Metric: Total storage operations (counter, labels: operation, status)
pub const METRIC_NAME_STORAGE_OPERATIONS_TOTAL: &str = "kelpie_storage_operations_total";

// Compile-time assertions for constant validity
const _: () = {
    assert!(ACTOR_ID_LENGTH_BYTES_MAX >= 64);
    assert!(ACTOR_STATE_SIZE_BYTES_MAX <= 100 * 1024 * 1024); // <= 100 MB
    assert!(TRANSACTION_SIZE_BYTES_MAX == 10 * 1024 * 1024); // FDB limit
    assert!(HEARTBEAT_TIMEOUT_MS > HEARTBEAT_INTERVAL_MS);
    assert!(ACTOR_INVOCATION_TIMEOUT_MS_MAX >= 1000);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_are_reasonable() {
        // Transaction limits aligned with FDB
        assert_eq!(TRANSACTION_SIZE_BYTES_MAX, 10 * 1024 * 1024);
    }

    #[test]
    fn test_limits_have_units_in_names() {
        // This test documents the naming convention
        // All byte limits end in _BYTES_
        // All time limits end in _MS_
        // All count limits end in _COUNT_

        // These are compile-time checks via naming convention
        let _: usize = ACTOR_ID_LENGTH_BYTES_MAX;
        let _: u64 = ACTOR_INVOCATION_TIMEOUT_MS_MAX;
        let _: usize = CLUSTER_NODES_COUNT_MAX;
    }
}
