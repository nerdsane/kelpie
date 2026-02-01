//! Node types and identification
//!
//! TigerStyle: Explicit node lifecycle with validated identifiers.

use crate::error::{RegistryError, RegistryResult};
use kelpie_core::constants::CLUSTER_NODES_COUNT_MAX;
use kelpie_core::io::{RngProvider, StdRngProvider, TimeProvider, WallClockTime};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;

/// Maximum length of a node ID in bytes
pub const NODE_ID_LENGTH_BYTES_MAX: usize = 128;

/// Unique identifier for a cluster node
///
/// Node IDs should be stable across restarts for the same physical node,
/// typically derived from hostname or configured explicitly.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeId(String);

impl NodeId {
    /// Create a new NodeId with validation
    ///
    /// # Arguments
    /// * `id` - The node identifier (alphanumeric, dashes, underscores, dots)
    ///
    /// # Errors
    /// Returns error if id is empty, too long, or contains invalid characters.
    pub fn new(id: impl Into<String>) -> RegistryResult<Self> {
        let id = id.into();

        // TigerStyle: Explicit validation
        if id.is_empty() {
            return Err(RegistryError::InvalidNodeId {
                id: id.clone(),
                reason: "node ID cannot be empty".into(),
            });
        }

        if id.len() > NODE_ID_LENGTH_BYTES_MAX {
            return Err(RegistryError::InvalidNodeId {
                id: id.clone(),
                reason: format!(
                    "node ID length {} exceeds limit {}",
                    id.len(),
                    NODE_ID_LENGTH_BYTES_MAX
                ),
            });
        }

        // Validate characters (alphanumeric, dash, underscore, dot)
        let valid = id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.');

        if !valid {
            return Err(RegistryError::InvalidNodeId {
                id: id.clone(),
                reason: "node ID contains invalid characters".into(),
            });
        }

        Ok(Self(id))
    }

    /// Create a NodeId without validation (for internal use)
    ///
    /// # Safety
    /// Caller must ensure the ID is valid.
    #[doc(hidden)]
    pub fn new_unchecked(id: String) -> Self {
        debug_assert!(!id.is_empty());
        debug_assert!(id.len() <= NODE_ID_LENGTH_BYTES_MAX);
        Self(id)
    }

    /// Get the node ID as a string
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Generate a unique node ID based on hostname and random suffix
    ///
    /// Uses production RNG. For DST, use `generate_with_rng`.
    pub fn generate() -> Self {
        Self::generate_with_rng(&StdRngProvider::new())
    }

    /// Generate a unique node ID with injected RNG (for DST)
    pub fn generate_with_rng(rng: &dyn RngProvider) -> Self {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let suffix: u32 = rng.next_u64() as u32;
        let id = format!("{}-{:08x}", hostname, suffix);

        // Truncate if too long
        let truncated = if id.len() > NODE_ID_LENGTH_BYTES_MAX {
            id[..NODE_ID_LENGTH_BYTES_MAX].to_string()
        } else {
            id
        };

        Self::new_unchecked(truncated)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for NodeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Node status in the cluster
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Node is joining the cluster
    Joining,
    /// Node is active and healthy
    Active,
    /// Node is suspected of failure (missed heartbeats)
    Suspect,
    /// Node has failed and is being drained
    Failed,
    /// Node is leaving the cluster gracefully
    Leaving,
    /// Node has left the cluster
    Left,
}

impl NodeStatus {
    /// Check if the node can accept new actor activations
    pub fn can_accept_actors(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the node is considered healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Active | Self::Joining)
    }

    /// Check if the node should be removed from the cluster
    pub fn should_remove(&self) -> bool {
        matches!(self, Self::Failed | Self::Left)
    }
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Joining => write!(f, "joining"),
            Self::Active => write!(f, "active"),
            Self::Suspect => write!(f, "suspect"),
            Self::Failed => write!(f, "failed"),
            Self::Leaving => write!(f, "leaving"),
            Self::Left => write!(f, "left"),
        }
    }
}

/// Information about a cluster node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Unique node identifier
    pub id: NodeId,
    /// Node's RPC address
    pub rpc_addr: SocketAddr,
    /// Current node status
    pub status: NodeStatus,
    /// Number of active actors on this node
    pub actor_count: u64,
    /// Maximum actors this node can host
    pub actor_capacity: u64,
    /// Time when node joined (Unix timestamp ms)
    pub joined_at_ms: u64,
    /// Time of last heartbeat (Unix timestamp ms)
    pub last_heartbeat_ms: u64,
    /// Node version (for rolling upgrades)
    pub version: String,
    /// Custom metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl NodeInfo {
    /// Create new node info using production wall clock
    ///
    /// For DST, use `with_timestamp` or `new_with_time`.
    ///
    /// # Arguments
    /// * `id` - The node's unique identifier
    /// * `rpc_addr` - The node's RPC address for inter-node communication
    pub fn new(id: NodeId, rpc_addr: SocketAddr) -> Self {
        Self::new_with_time(id, rpc_addr, &WallClockTime::new())
    }

    /// Create new node info with injected time provider (for DST)
    pub fn new_with_time(id: NodeId, rpc_addr: SocketAddr, time: &dyn TimeProvider) -> Self {
        Self::with_timestamp(id, rpc_addr, time.now_ms())
    }

    /// Create new node info with a specific timestamp
    ///
    /// Useful for testing and simulation.
    pub fn with_timestamp(id: NodeId, rpc_addr: SocketAddr, timestamp_ms: u64) -> Self {
        Self {
            id,
            rpc_addr,
            status: NodeStatus::Joining,
            actor_count: 0,
            actor_capacity: kelpie_core::constants::ACTOR_CONCURRENT_COUNT_MAX as u64,
            joined_at_ms: timestamp_ms,
            last_heartbeat_ms: timestamp_ms,
            version: env!("CARGO_PKG_VERSION").to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Update heartbeat timestamp
    pub fn update_heartbeat(&mut self, timestamp_ms: u64) {
        // Accept the timestamp even if it's in the past (could be due to clock sync)
        // but only update if it's newer
        if timestamp_ms >= self.last_heartbeat_ms {
            self.last_heartbeat_ms = timestamp_ms;
        }
    }

    /// Check if heartbeat has timed out
    pub fn is_heartbeat_timeout(&self, now_ms: u64, timeout_ms: u64) -> bool {
        debug_assert!(timeout_ms > 0);
        now_ms.saturating_sub(self.last_heartbeat_ms) > timeout_ms
    }

    /// Check if node has capacity for more actors
    pub fn has_capacity(&self) -> bool {
        self.actor_count < self.actor_capacity
    }

    /// Calculate available capacity
    pub fn available_capacity(&self) -> u64 {
        self.actor_capacity.saturating_sub(self.actor_count)
    }

    /// Calculate load as a percentage (0-100)
    pub fn load_percent(&self) -> u8 {
        if self.actor_capacity == 0 {
            return 100;
        }
        let load = (self.actor_count as f64 / self.actor_capacity as f64) * 100.0;
        load.min(100.0) as u8
    }

    /// Increment actor count
    pub fn increment_actor_count(&mut self) {
        self.actor_count = self.actor_count.saturating_add(1);
    }

    /// Decrement actor count
    pub fn decrement_actor_count(&mut self) {
        self.actor_count = self.actor_count.saturating_sub(1);
    }

    /// Set status with validation
    pub fn set_status(&mut self, status: NodeStatus) {
        // Valid transitions
        let valid = matches!(
            (&self.status, &status),
            (NodeStatus::Joining, NodeStatus::Active)
                | (NodeStatus::Active, NodeStatus::Suspect)
                | (NodeStatus::Active, NodeStatus::Leaving)
                | (NodeStatus::Suspect, NodeStatus::Active)
                | (NodeStatus::Suspect, NodeStatus::Failed)
                | (NodeStatus::Leaving, NodeStatus::Left)
                | (NodeStatus::Failed, NodeStatus::Left)
        );

        debug_assert!(
            valid,
            "invalid status transition from {} to {}",
            self.status, status
        );

        self.status = status;
    }
}

/// Compile-time assertion for cluster limit
const _: () = {
    assert!(CLUSTER_NODES_COUNT_MAX >= 1);
    assert!(CLUSTER_NODES_COUNT_MAX <= 10_000);
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_addr() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080)
    }

    #[test]
    fn test_node_id_valid() {
        let id = NodeId::new("node-1").unwrap();
        assert_eq!(id.as_str(), "node-1");
        assert_eq!(format!("{}", id), "node-1");
    }

    #[test]
    fn test_node_id_invalid_empty() {
        let result = NodeId::new("");
        assert!(matches!(result, Err(RegistryError::InvalidNodeId { .. })));
    }

    #[test]
    fn test_node_id_invalid_chars() {
        let result = NodeId::new("node/1");
        assert!(matches!(result, Err(RegistryError::InvalidNodeId { .. })));
    }

    #[test]
    fn test_node_id_too_long() {
        let long = "a".repeat(NODE_ID_LENGTH_BYTES_MAX + 1);
        let result = NodeId::new(long);
        assert!(matches!(result, Err(RegistryError::InvalidNodeId { .. })));
    }

    #[test]
    fn test_node_id_generate() {
        let id1 = NodeId::generate();
        let id2 = NodeId::generate();
        assert_ne!(id1, id2);
        assert!(id1.as_str().len() <= NODE_ID_LENGTH_BYTES_MAX);
    }

    #[test]
    fn test_node_status_transitions() {
        assert!(NodeStatus::Active.can_accept_actors());
        assert!(!NodeStatus::Joining.can_accept_actors());
        assert!(!NodeStatus::Failed.can_accept_actors());

        assert!(NodeStatus::Active.is_healthy());
        assert!(NodeStatus::Joining.is_healthy());
        assert!(!NodeStatus::Failed.is_healthy());

        assert!(NodeStatus::Failed.should_remove());
        assert!(NodeStatus::Left.should_remove());
        assert!(!NodeStatus::Active.should_remove());
    }

    #[test]
    fn test_node_info_new() {
        let id = NodeId::new("node-1").unwrap();
        let info = NodeInfo::new(id.clone(), test_addr());

        assert_eq!(info.id, id);
        assert_eq!(info.status, NodeStatus::Joining);
        assert_eq!(info.actor_count, 0);
        assert!(info.has_capacity());
    }

    #[test]
    fn test_node_info_heartbeat() {
        let id = NodeId::new("node-1").unwrap();
        let mut info = NodeInfo::new(id, test_addr());

        let now_ms = info.last_heartbeat_ms;
        assert!(!info.is_heartbeat_timeout(now_ms + 1000, 5000));
        assert!(info.is_heartbeat_timeout(now_ms + 6000, 5000));

        info.update_heartbeat(now_ms + 3000);
        assert!(!info.is_heartbeat_timeout(now_ms + 6000, 5000));
    }

    #[test]
    fn test_node_info_capacity() {
        let id = NodeId::new("node-1").unwrap();
        let mut info = NodeInfo::new(id, test_addr());
        info.actor_capacity = 100;

        assert!(info.has_capacity());
        assert_eq!(info.available_capacity(), 100);
        assert_eq!(info.load_percent(), 0);

        info.actor_count = 50;
        assert!(info.has_capacity());
        assert_eq!(info.available_capacity(), 50);
        assert_eq!(info.load_percent(), 50);

        info.actor_count = 100;
        assert!(!info.has_capacity());
        assert_eq!(info.available_capacity(), 0);
        assert_eq!(info.load_percent(), 100);
    }

    #[test]
    fn test_node_info_actor_count() {
        let id = NodeId::new("node-1").unwrap();
        let mut info = NodeInfo::new(id, test_addr());

        info.increment_actor_count();
        assert_eq!(info.actor_count, 1);

        info.increment_actor_count();
        assert_eq!(info.actor_count, 2);

        info.decrement_actor_count();
        assert_eq!(info.actor_count, 1);
    }
}
