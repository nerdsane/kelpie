//! Simulated network for deterministic testing
//!
//! TigerStyle: Configurable network conditions with fault injection.

use crate::clock::SimClock;
use crate::fault::{FaultInjector, FaultType};
use crate::rng::DeterministicRng;
use bytes::Bytes;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A network message
#[derive(Debug, Clone)]
pub struct NetworkMessage {
    /// Source node ID
    pub from: String,
    /// Destination node ID
    pub to: String,
    /// Message payload
    pub payload: Bytes,
    /// Time when message should be delivered
    pub deliver_at_ms: u64,
}

/// Simulated network for DST
///
/// Provides configurable network conditions including:
/// - Message delays
/// - Packet loss
/// - Network partitions
/// - Message reordering
/// - Packet corruption (Issue #36)
/// - Network jitter with normal distribution (Issue #36)
/// - Connection exhaustion (Issue #36)
#[derive(Debug)]
pub struct SimNetwork {
    /// Pending messages per destination
    messages: Arc<RwLock<HashMap<String, VecDeque<NetworkMessage>>>>,
    /// Bidirectional network partitions (set of (node_a, node_b) pairs)
    /// Messages blocked in both directions
    bidirectional_partitions: Arc<RwLock<HashSet<(String, String)>>>,
    /// One-way network partitions (set of (from, to) pairs)
    /// Messages from `from` to `to` blocked, but `to` to `from` allowed
    one_way_partitions: Arc<RwLock<HashSet<(String, String)>>>,
    /// Simulation clock
    clock: SimClock,
    /// Fault injector
    fault_injector: Arc<FaultInjector>,
    /// RNG for delays
    rng: DeterministicRng,
    /// Base latency in milliseconds
    base_latency_ms: u64,
    /// Latency jitter in milliseconds
    latency_jitter_ms: u64,
    /// Active connection count (for connection exhaustion simulation)
    active_connections: Arc<std::sync::atomic::AtomicUsize>,
    /// Maximum connections allowed (0 = unlimited)
    max_connections: usize,
}

impl SimNetwork {
    /// Create a new simulated network
    pub fn new(clock: SimClock, rng: DeterministicRng, fault_injector: Arc<FaultInjector>) -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
            bidirectional_partitions: Arc::new(RwLock::new(HashSet::new())),
            one_way_partitions: Arc::new(RwLock::new(HashSet::new())),
            clock,
            fault_injector,
            rng,
            base_latency_ms: 1,
            latency_jitter_ms: 5,
            active_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_connections: 0, // 0 = unlimited
        }
    }

    /// Set base latency
    pub fn with_latency(mut self, base_ms: u64, jitter_ms: u64) -> Self {
        self.base_latency_ms = base_ms;
        self.latency_jitter_ms = jitter_ms;
        self
    }

    /// Set maximum connections for connection exhaustion testing
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Send a message from one node to another
    pub async fn send(&self, from: &str, to: &str, payload: Bytes) -> bool {
        // Check for bidirectional partition
        {
            let partitions = self.bidirectional_partitions.read().await;
            // Normalize to (min, max) for bidirectional lookup
            let (a, b) = if from < to {
                (from.to_string(), to.to_string())
            } else {
                (to.to_string(), from.to_string())
            };
            if partitions.contains(&(a, b)) {
                tracing::debug!(
                    from = from,
                    to = to,
                    "Message dropped: bidirectional partition"
                );
                return false;
            }
        }

        // Check for one-way partition (directional: from -> to)
        {
            let one_way = self.one_way_partitions.read().await;
            if one_way.contains(&(from.to_string(), to.to_string())) {
                tracing::debug!(from = from, to = to, "Message dropped: one-way partition");
                return false;
            }
        }

        // Check connection exhaustion limit
        if self.max_connections > 0 {
            let current = self
                .active_connections
                .load(std::sync::atomic::Ordering::SeqCst);
            if current >= self.max_connections {
                tracing::debug!(
                    from = from,
                    to = to,
                    current = current,
                    max = self.max_connections,
                    "Message dropped: connection exhaustion"
                );
                return false;
            }
        }

        // Check for network faults
        let mut actual_payload = payload;
        let mut extra_latency_ms: u64 = 0;

        if let Some(fault) = self.fault_injector.should_inject("network_send") {
            match fault {
                FaultType::NetworkPacketLoss => {
                    tracing::debug!(from = from, to = to, "Message dropped: packet loss fault");
                    return false;
                }
                FaultType::NetworkPartition => {
                    tracing::debug!(from = from, to = to, "Message dropped: partition fault");
                    return false;
                }
                // FoundationDB-critical network faults (Issue #36)
                FaultType::NetworkPacketCorruption { corruption_rate } => {
                    // Corrupt the payload bytes
                    actual_payload = self.corrupt_payload(actual_payload, corruption_rate);
                    tracing::debug!(
                        from = from,
                        to = to,
                        corruption_rate = corruption_rate,
                        "Packet corrupted in transit"
                    );
                }
                FaultType::NetworkJitter { mean_ms, stddev_ms } => {
                    // Add jitter using approximate normal distribution
                    extra_latency_ms = self.calculate_jitter(mean_ms, stddev_ms);
                    tracing::debug!(
                        from = from,
                        to = to,
                        jitter_ms = extra_latency_ms,
                        "Network jitter added"
                    );
                }
                FaultType::NetworkConnectionExhaustion => {
                    tracing::debug!(
                        from = from,
                        to = to,
                        "Message dropped: connection exhaustion fault"
                    );
                    return false;
                }
                _ => {}
            }
        }

        // Calculate delivery time with latency (including any jitter)
        let latency = self.calculate_latency() + extra_latency_ms;
        let deliver_at_ms = self.clock.now_ms() + latency;

        let message = NetworkMessage {
            from: from.to_string(),
            to: to.to_string(),
            payload: actual_payload,
            deliver_at_ms,
        };

        // Queue the message
        let mut messages = self.messages.write().await;
        messages
            .entry(to.to_string())
            .or_default()
            .push_back(message);

        // Track connection (for exhaustion simulation)
        if self.max_connections > 0 {
            self.active_connections
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        true
    }

    /// Corrupt payload bytes based on corruption rate
    fn corrupt_payload(&self, payload: Bytes, corruption_rate: f64) -> Bytes {
        if payload.is_empty() {
            return payload;
        }

        let mut corrupted = payload.to_vec();
        for byte in corrupted.iter_mut() {
            if self.rng.next_f64() < corruption_rate {
                // XOR with random byte to corrupt
                *byte ^= (self.rng.next_u64() & 0xFF) as u8;
            }
        }
        Bytes::from(corrupted)
    }

    /// Calculate jitter using Box-Muller transform for approximate normal distribution
    fn calculate_jitter(&self, mean_ms: u64, stddev_ms: u64) -> u64 {
        if stddev_ms == 0 {
            return mean_ms;
        }

        // Box-Muller transform for normal distribution
        let u1 = self.rng.next_f64().max(1e-10); // Avoid log(0)
        let u2 = self.rng.next_f64();
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();

        // Scale to desired mean and stddev
        let jitter = mean_ms as f64 + z * stddev_ms as f64;

        // Clamp to non-negative
        jitter.max(0.0) as u64
    }

    /// Receive messages for a node
    ///
    /// Returns all messages that have arrived (delivery time <= current time)
    pub async fn receive(&self, node_id: &str) -> Vec<NetworkMessage> {
        let current_time = self.clock.now_ms();
        let mut messages = self.messages.write().await;

        let queue = match messages.get_mut(node_id) {
            Some(q) => q,
            None => return Vec::new(),
        };

        // Collect messages ready for delivery
        let mut ready = Vec::new();
        let mut remaining = VecDeque::new();

        while let Some(msg) = queue.pop_front() {
            if msg.deliver_at_ms <= current_time {
                ready.push(msg);
            } else {
                remaining.push_back(msg);
            }
        }

        *queue = remaining;

        // Check for message reordering fault
        if !ready.is_empty() {
            if let Some(FaultType::NetworkMessageReorder) =
                self.fault_injector.should_inject("network_receive")
            {
                self.rng.shuffle(&mut ready);
                tracing::debug!(node_id = node_id, "Messages reordered by fault");
            }
        }

        ready
    }

    /// Create a bidirectional network partition between two nodes
    ///
    /// Messages in BOTH directions are blocked (node_a -> node_b and node_b -> node_a).
    pub async fn partition(&self, node_a: &str, node_b: &str) {
        let mut partitions = self.bidirectional_partitions.write().await;
        // Normalize to (min, max) for consistent storage
        let (a, b) = if node_a < node_b {
            (node_a.to_string(), node_b.to_string())
        } else {
            (node_b.to_string(), node_a.to_string())
        };
        partitions.insert((a, b));
        tracing::info!(
            node_a = node_a,
            node_b = node_b,
            "Bidirectional network partition created"
        );
    }

    /// Create a one-way network partition
    ///
    /// Messages from `from` to `to` are blocked, but messages from `to` to `from` are allowed.
    /// This models asymmetric network failures like:
    /// - Replication lag (writes go through, acks don't)
    /// - One-way network failures
    /// - Partial connectivity
    pub async fn partition_one_way(&self, from: &str, to: &str) {
        assert!(!from.is_empty(), "from node cannot be empty");
        assert!(!to.is_empty(), "to node cannot be empty");
        assert!(from != to, "cannot partition a node from itself");

        let mut partitions = self.one_way_partitions.write().await;
        partitions.insert((from.to_string(), to.to_string()));
        tracing::info!(
            from = from,
            to = to,
            "One-way network partition created (from -> to blocked)"
        );
    }

    /// Heal a bidirectional network partition between two nodes
    pub async fn heal(&self, node_a: &str, node_b: &str) {
        let mut partitions = self.bidirectional_partitions.write().await;
        // Normalize to (min, max) for lookup
        let (a, b) = if node_a < node_b {
            (node_a.to_string(), node_b.to_string())
        } else {
            (node_b.to_string(), node_a.to_string())
        };
        partitions.remove(&(a, b));
        tracing::info!(
            node_a = node_a,
            node_b = node_b,
            "Bidirectional network partition healed"
        );
    }

    /// Heal a one-way network partition
    ///
    /// Only removes the specific directional partition from `from` to `to`.
    pub async fn heal_one_way(&self, from: &str, to: &str) {
        let mut partitions = self.one_way_partitions.write().await;
        partitions.remove(&(from.to_string(), to.to_string()));
        tracing::info!(from = from, to = to, "One-way network partition healed");
    }

    /// Heal all network partitions (both bidirectional and one-way)
    pub async fn heal_all(&self) {
        {
            let mut partitions = self.bidirectional_partitions.write().await;
            partitions.clear();
        }
        {
            let mut partitions = self.one_way_partitions.write().await;
            partitions.clear();
        }
        tracing::info!("All network partitions healed");
    }

    /// Partition two groups of nodes completely
    ///
    /// All nodes in group_a become isolated from all nodes in group_b.
    /// This creates bidirectional partitions between every pair.
    pub async fn partition_group(&self, group_a: &[&str], group_b: &[&str]) {
        for a in group_a {
            for b in group_b {
                self.partition(a, b).await;
            }
        }
        tracing::info!(
            group_a = ?group_a,
            group_b = ?group_b,
            "Network group partition created"
        );
    }

    /// Check if there's a one-way partition from one node to another
    ///
    /// This checks ONLY one-way partitions, not bidirectional ones.
    pub async fn is_one_way_partitioned(&self, from: &str, to: &str) -> bool {
        let partitions = self.one_way_partitions.read().await;
        partitions.contains(&(from.to_string(), to.to_string()))
    }

    /// Check if messages from `from` to `to` are blocked by any partition
    ///
    /// This checks both bidirectional and one-way partitions.
    /// Note: This is directional - `is_partitioned(a, b)` may differ from `is_partitioned(b, a)`
    /// when one-way partitions are in effect.
    pub async fn is_partitioned(&self, from: &str, to: &str) -> bool {
        // Check bidirectional partition
        {
            let partitions = self.bidirectional_partitions.read().await;
            let (a, b) = if from < to {
                (from.to_string(), to.to_string())
            } else {
                (to.to_string(), from.to_string())
            };
            if partitions.contains(&(a, b)) {
                return true;
            }
        }

        // Check one-way partition
        {
            let partitions = self.one_way_partitions.read().await;
            if partitions.contains(&(from.to_string(), to.to_string())) {
                return true;
            }
        }

        false
    }

    /// Get count of pending messages for a node
    pub async fn pending_count(&self, node_id: &str) -> usize {
        let messages = self.messages.read().await;
        messages.get(node_id).map(|q| q.len()).unwrap_or(0)
    }

    /// Clear all pending messages
    pub async fn clear(&self) {
        let mut messages = self.messages.write().await;
        messages.clear();
    }

    /// Calculate latency with jitter
    fn calculate_latency(&self) -> u64 {
        let jitter = if self.latency_jitter_ms > 0 {
            self.rng.next_range(0, self.latency_jitter_ms)
        } else {
            0
        };
        self.base_latency_ms + jitter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::FaultInjectorBuilder;

    fn create_test_network(clock: SimClock) -> SimNetwork {
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
        SimNetwork::new(clock, rng, fault_injector).with_latency(0, 0)
    }

    #[tokio::test]
    async fn test_sim_network_basic() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Send message
        let sent = network.send("node-1", "node-2", Bytes::from("hello")).await;
        assert!(sent);

        // Receive message
        let messages = network.receive("node-2").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, Bytes::from("hello"));
    }

    #[tokio::test]
    async fn test_sim_network_bidirectional_partition() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create bidirectional partition
        network.partition("node-1", "node-2").await;

        // Messages blocked in BOTH directions
        let sent_1_to_2 = network.send("node-1", "node-2", Bytes::from("hello")).await;
        assert!(!sent_1_to_2, "node-1 -> node-2 should be blocked");

        let sent_2_to_1 = network.send("node-2", "node-1", Bytes::from("hello")).await;
        assert!(!sent_2_to_1, "node-2 -> node-1 should be blocked");

        // Heal partition
        network.heal("node-1", "node-2").await;

        // Messages go through in both directions
        let sent = network.send("node-1", "node-2", Bytes::from("hello")).await;
        assert!(sent);

        let sent = network.send("node-2", "node-1", Bytes::from("hello")).await;
        assert!(sent);
    }

    #[tokio::test]
    async fn test_sim_network_one_way_partition_basic() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create one-way partition: node-1 -> node-2 blocked
        network.partition_one_way("node-1", "node-2").await;

        // node-1 -> node-2 should be BLOCKED
        let sent_1_to_2 = network.send("node-1", "node-2", Bytes::from("hello")).await;
        assert!(!sent_1_to_2, "node-1 -> node-2 should be blocked");

        // node-2 -> node-1 should WORK (asymmetric!)
        let sent_2_to_1 = network.send("node-2", "node-1", Bytes::from("reply")).await;
        assert!(sent_2_to_1, "node-2 -> node-1 should work");

        // Verify message was delivered
        let messages = network.receive("node-1").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, Bytes::from("reply"));
    }

    #[tokio::test]
    async fn test_sim_network_one_way_partition_heal() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create one-way partition
        network.partition_one_way("node-1", "node-2").await;
        assert!(network.is_partitioned("node-1", "node-2").await);
        assert!(!network.is_partitioned("node-2", "node-1").await);

        // Heal the one-way partition
        network.heal_one_way("node-1", "node-2").await;

        // Both directions should work now
        let sent_1_to_2 = network.send("node-1", "node-2", Bytes::from("hello")).await;
        assert!(sent_1_to_2, "node-1 -> node-2 should work after heal");
    }

    #[tokio::test]
    async fn test_sim_network_one_way_vs_bidirectional_independence() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create one-way partition for node-1 -> node-2
        network.partition_one_way("node-1", "node-2").await;

        // Create bidirectional partition for node-3 <-> node-4
        network.partition("node-3", "node-4").await;

        // Test one-way behavior
        assert!(
            !network
                .send("node-1", "node-2", Bytes::from("blocked"))
                .await
        );
        assert!(
            network
                .send("node-2", "node-1", Bytes::from("allowed"))
                .await
        );

        // Test bidirectional behavior
        assert!(
            !network
                .send("node-3", "node-4", Bytes::from("blocked"))
                .await
        );
        assert!(
            !network
                .send("node-4", "node-3", Bytes::from("blocked"))
                .await
        );

        // Heal all
        network.heal_all().await;

        // Everything should work
        assert!(network.send("node-1", "node-2", Bytes::from("ok")).await);
        assert!(network.send("node-3", "node-4", Bytes::from("ok")).await);
    }

    #[tokio::test]
    async fn test_sim_network_is_partitioned_directional() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // No partitions initially
        assert!(!network.is_partitioned("node-1", "node-2").await);
        assert!(!network.is_partitioned("node-2", "node-1").await);

        // One-way partition: is_partitioned is directional
        network.partition_one_way("node-1", "node-2").await;
        assert!(
            network.is_partitioned("node-1", "node-2").await,
            "from -> to should be partitioned"
        );
        assert!(
            !network.is_partitioned("node-2", "node-1").await,
            "to -> from should NOT be partitioned"
        );

        // Bidirectional partition: is_partitioned is symmetric
        network.heal_all().await;
        network.partition("node-1", "node-2").await;
        assert!(
            network.is_partitioned("node-1", "node-2").await,
            "bidirectional: a -> b partitioned"
        );
        assert!(
            network.is_partitioned("node-2", "node-1").await,
            "bidirectional: b -> a partitioned"
        );
    }

    #[tokio::test]
    async fn test_sim_network_asymmetric_leader_isolation_scenario() {
        // Scenario: Leader can send heartbeats, but can't receive votes
        // This simulates followers being able to receive from leader but leader can't
        // receive from followers.
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        let leader = "leader";
        let follower1 = "follower-1";
        let follower2 = "follower-2";

        // Followers can't send to leader (leader isolated for incoming)
        network.partition_one_way(follower1, leader).await;
        network.partition_one_way(follower2, leader).await;

        // Leader CAN send heartbeats to followers
        assert!(
            network
                .send(leader, follower1, Bytes::from("heartbeat"))
                .await
        );
        assert!(
            network
                .send(leader, follower2, Bytes::from("heartbeat"))
                .await
        );

        // Followers CANNOT send votes/acks back to leader
        assert!(!network.send(follower1, leader, Bytes::from("vote")).await);
        assert!(!network.send(follower2, leader, Bytes::from("ack")).await);

        // Followers CAN communicate with each other (no partition between them)
        assert!(
            network
                .send(follower1, follower2, Bytes::from("peer-msg"))
                .await
        );
        assert!(
            network
                .send(follower2, follower1, Bytes::from("peer-msg"))
                .await
        );
    }

    #[tokio::test]
    async fn test_sim_network_asymmetric_replication_lag_scenario() {
        // Scenario: Primary can send writes to replica, but replica can't send acks
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        let primary = "primary";
        let replica = "replica";

        // Replica -> Primary blocked (acks can't get through)
        network.partition_one_way(replica, primary).await;

        // Primary CAN send writes to replica
        assert!(network.send(primary, replica, Bytes::from("write-1")).await);
        assert!(network.send(primary, replica, Bytes::from("write-2")).await);

        // Replica CANNOT send acks back
        assert!(!network.send(replica, primary, Bytes::from("ack-1")).await);
        assert!(!network.send(replica, primary, Bytes::from("ack-2")).await);

        // Verify writes were received by replica
        let messages = network.receive(replica).await;
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn test_sim_network_latency() {
        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());
        let network = SimNetwork::new(clock.clone(), rng, fault_injector).with_latency(100, 0);

        // Send message
        network.send("node-1", "node-2", Bytes::from("hello")).await;

        // Should not be delivered yet
        let messages = network.receive("node-2").await;
        assert!(messages.is_empty());

        // Advance time
        clock.advance_ms(100);

        // Now should be delivered
        let messages = network.receive("node-2").await;
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn test_sim_network_bidirectional_partition_order_independence() {
        // Verify that partition(a, b) and partition(b, a) have the same effect
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create partition with reversed order
        network.partition("node-2", "node-1").await;

        // Both directions should be blocked
        assert!(!network.send("node-1", "node-2", Bytes::from("test")).await);
        assert!(!network.send("node-2", "node-1", Bytes::from("test")).await);

        // Heal with reversed order should also work
        network.heal("node-1", "node-2").await;

        assert!(network.send("node-1", "node-2", Bytes::from("test")).await);
        assert!(network.send("node-2", "node-1", Bytes::from("test")).await);
    }

    #[tokio::test]
    async fn test_sim_network_partition_group() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Partition [node-1, node-2] from [node-3, node-4]
        network
            .partition_group(&["node-1", "node-2"], &["node-3", "node-4"])
            .await;

        // Messages within groups should work
        assert!(
            network
                .send("node-1", "node-2", Bytes::from("intra-1"))
                .await
        );
        assert!(
            network
                .send("node-3", "node-4", Bytes::from("intra-2"))
                .await
        );

        // Messages across groups should fail
        assert!(!network.send("node-1", "node-3", Bytes::from("cross")).await);
        assert!(!network.send("node-2", "node-4", Bytes::from("cross")).await);
        assert!(!network.send("node-3", "node-1", Bytes::from("cross")).await);
        assert!(!network.send("node-4", "node-2", Bytes::from("cross")).await);
    }

    #[tokio::test]
    async fn test_sim_network_is_one_way_partitioned() {
        let clock = SimClock::from_millis(0);
        let network = create_test_network(clock);

        // Create one-way partition: node-1 -> node-2 blocked
        network.partition_one_way("node-1", "node-2").await;

        // Check one-way partition detection
        assert!(network.is_one_way_partitioned("node-1", "node-2").await);
        assert!(!network.is_one_way_partitioned("node-2", "node-1").await);

        // Heal one-way partition
        network.heal_one_way("node-1", "node-2").await;
        assert!(!network.is_one_way_partitioned("node-1", "node-2").await);
    }

    // ============================================================================
    // FoundationDB-Critical Network Fault Tests (Issue #36)
    // ============================================================================

    #[tokio::test]
    async fn test_sim_network_packet_corruption() {
        use crate::fault::FaultConfig;

        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(
            FaultInjectorBuilder::new(rng.fork())
                .with_fault(FaultConfig::new(
                    FaultType::NetworkPacketCorruption {
                        corruption_rate: 1.0, // 100% of bytes corrupted
                    },
                    1.0, // Always trigger
                ))
                .build(),
        );
        let network = SimNetwork::new(clock, rng, fault_injector).with_latency(0, 0);

        // Send a message
        let original = Bytes::from("hello_world");
        let sent = network.send("node-1", "node-2", original.clone()).await;
        assert!(sent, "Message should be sent (with corruption)");

        // Receive message
        let messages = network.receive("node-2").await;
        assert_eq!(messages.len(), 1);

        // Message should be corrupted (different from original)
        assert_ne!(messages[0].payload, original, "Payload should be corrupted");
        assert_eq!(
            messages[0].payload.len(),
            original.len(),
            "Payload length should be unchanged"
        );
    }

    #[tokio::test]
    async fn test_sim_network_packet_corruption_partial() {
        use crate::fault::FaultConfig;

        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(
            FaultInjectorBuilder::new(rng.fork())
                .with_fault(FaultConfig::new(
                    FaultType::NetworkPacketCorruption {
                        corruption_rate: 0.0, // 0% corruption
                    },
                    1.0, // Always trigger fault check but no bytes corrupted
                ))
                .build(),
        );
        let network = SimNetwork::new(clock, rng, fault_injector).with_latency(0, 0);

        // Send a message
        let original = Bytes::from("hello_world");
        let sent = network.send("node-1", "node-2", original.clone()).await;
        assert!(sent);

        // Receive message
        let messages = network.receive("node-2").await;
        assert_eq!(messages.len(), 1);

        // With 0% corruption rate, message should be unchanged
        assert_eq!(
            messages[0].payload, original,
            "Payload should be unchanged with 0% corruption"
        );
    }

    #[tokio::test]
    async fn test_sim_network_jitter() {
        use crate::fault::FaultConfig;

        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(
            FaultInjectorBuilder::new(rng.fork())
                .with_fault(FaultConfig::new(
                    FaultType::NetworkJitter {
                        mean_ms: 100,
                        stddev_ms: 50,
                    },
                    1.0, // Always trigger
                ))
                .build(),
        );
        let network = SimNetwork::new(clock.clone(), rng, fault_injector).with_latency(10, 0);

        // Send multiple messages and check they have varying delivery times
        let mut delivery_times = Vec::new();
        for i in 0..10 {
            network
                .send(
                    "node-1",
                    &format!("node-{}", i),
                    Bytes::from(format!("msg-{}", i)),
                )
                .await;
        }

        // Check pending messages for different delivery times
        let messages = network.messages.read().await;
        for (_, queue) in messages.iter() {
            for msg in queue.iter() {
                delivery_times.push(msg.deliver_at_ms);
            }
        }

        // With jitter, we should see some variance in delivery times
        // (not all the same as base latency would produce)
        if delivery_times.len() > 1 {
            let min = delivery_times.iter().min().unwrap();
            let max = delivery_times.iter().max().unwrap();
            // With mean=100, stddev=50, we expect significant variance
            // This test just verifies jitter is being applied
            assert!(
                max > min || delivery_times.len() == 1,
                "Jitter should produce varying delivery times"
            );
        }
    }

    #[tokio::test]
    async fn test_sim_network_connection_exhaustion_limit() {
        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(FaultInjectorBuilder::new(rng.fork()).build());

        // Set max connections to 3
        let network = SimNetwork::new(clock, rng, fault_injector)
            .with_latency(0, 0)
            .with_max_connections(3);

        // First 3 connections should succeed
        assert!(network.send("a", "b", Bytes::from("1")).await);
        assert!(network.send("a", "c", Bytes::from("2")).await);
        assert!(network.send("a", "d", Bytes::from("3")).await);

        // 4th connection should fail due to exhaustion
        assert!(
            !network.send("a", "e", Bytes::from("4")).await,
            "Should fail due to connection exhaustion"
        );
    }

    #[tokio::test]
    async fn test_sim_network_connection_exhaustion_fault() {
        use crate::fault::FaultConfig;

        let clock = SimClock::from_millis(0);
        let rng = DeterministicRng::new(42);
        let fault_injector = Arc::new(
            FaultInjectorBuilder::new(rng.fork())
                .with_fault(FaultConfig::new(
                    FaultType::NetworkConnectionExhaustion,
                    1.0, // Always trigger
                ))
                .build(),
        );
        let network = SimNetwork::new(clock, rng, fault_injector).with_latency(0, 0);

        // All sends should fail due to connection exhaustion fault
        assert!(
            !network.send("a", "b", Bytes::from("1")).await,
            "Should fail due to connection exhaustion fault"
        );
    }

    #[tokio::test]
    async fn test_sim_network_jitter_determinism() {
        use crate::fault::FaultConfig;

        // Same seed should produce same jitter values
        for seed in [42u64, 123, 456] {
            let clock1 = SimClock::from_millis(0);
            let clock2 = SimClock::from_millis(0);
            let rng1 = DeterministicRng::new(seed);
            let rng2 = DeterministicRng::new(seed);

            let fi1 = Arc::new(
                FaultInjectorBuilder::new(rng1.fork())
                    .with_fault(FaultConfig::new(
                        FaultType::NetworkJitter {
                            mean_ms: 100,
                            stddev_ms: 50,
                        },
                        1.0,
                    ))
                    .build(),
            );
            let fi2 = Arc::new(
                FaultInjectorBuilder::new(rng2.fork())
                    .with_fault(FaultConfig::new(
                        FaultType::NetworkJitter {
                            mean_ms: 100,
                            stddev_ms: 50,
                        },
                        1.0,
                    ))
                    .build(),
            );

            let network1 = SimNetwork::new(clock1, rng1, fi1).with_latency(10, 0);
            let network2 = SimNetwork::new(clock2, rng2, fi2).with_latency(10, 0);

            // Send same sequence
            for i in 0..5 {
                network1
                    .send("a", "b", Bytes::from(format!("msg{}", i)))
                    .await;
                network2
                    .send("a", "b", Bytes::from(format!("msg{}", i)))
                    .await;
            }

            // Get delivery times
            let msgs1 = network1.messages.read().await;
            let msgs2 = network2.messages.read().await;

            let times1: Vec<_> = msgs1
                .get("b")
                .map(|q| q.iter().map(|m| m.deliver_at_ms).collect())
                .unwrap_or_default();
            let times2: Vec<_> = msgs2
                .get("b")
                .map(|q| q.iter().map(|m| m.deliver_at_ms).collect())
                .unwrap_or_default();

            assert_eq!(
                times1, times2,
                "seed {} should produce identical jitter patterns",
                seed
            );
        }
    }
}
