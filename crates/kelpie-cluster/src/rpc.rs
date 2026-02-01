//! Inter-node RPC
//!
//! TigerStyle: Explicit message types with bounded payloads.

use crate::error::{ClusterError, ClusterResult};
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_core::runtime::Runtime;
// For future: RPC_MESSAGE_SIZE_BYTES_MAX for message validation
use kelpie_registry::{Heartbeat, NodeId};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// RPC request ID
pub type RequestId = u64;

/// RPC message types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcMessage {
    // =========================================================================
    // Heartbeat
    // =========================================================================
    /// Heartbeat message
    Heartbeat(Heartbeat),

    /// Heartbeat acknowledgement
    HeartbeatAck { node_id: NodeId, sequence: u64 },

    // =========================================================================
    // Actor Operations
    // =========================================================================
    /// Forward an actor invocation to another node
    ActorInvoke {
        request_id: RequestId,
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
    },

    /// Response to actor invocation
    ActorInvokeResponse {
        request_id: RequestId,
        result: Result<Bytes, String>,
    },

    /// Prepare to migrate an actor (pre-migration check)
    MigratePrepare {
        request_id: RequestId,
        actor_id: ActorId,
        from_node: NodeId,
    },

    /// Confirm migration readiness
    MigratePrepareResponse {
        request_id: RequestId,
        ready: bool,
        reason: Option<String>,
    },

    /// Transfer actor state during migration
    MigrateTransfer {
        request_id: RequestId,
        actor_id: ActorId,
        state: Bytes,
        from_node: NodeId,
    },

    /// Confirm state received
    MigrateTransferResponse {
        request_id: RequestId,
        success: bool,
        reason: Option<String>,
    },

    /// Complete migration (activate on new node)
    MigrateComplete {
        request_id: RequestId,
        actor_id: ActorId,
    },

    /// Confirm migration complete
    MigrateCompleteResponse {
        request_id: RequestId,
        success: bool,
        reason: Option<String>,
    },

    // =========================================================================
    // Cluster Management
    // =========================================================================
    /// Join request from a new node
    JoinRequest {
        request_id: RequestId,
        node_id: NodeId,
        rpc_addr: SocketAddr,
    },

    /// Join response with cluster info
    JoinResponse {
        request_id: RequestId,
        accepted: bool,
        cluster_nodes: Vec<(NodeId, SocketAddr)>,
        reason: Option<String>,
    },

    /// Leave notification
    LeaveNotification { node_id: NodeId },

    /// Request cluster state
    ClusterStateRequest { request_id: RequestId },

    /// Cluster state response
    ClusterStateResponse {
        request_id: RequestId,
        nodes: Vec<(NodeId, SocketAddr)>,
        leader: Option<NodeId>,
    },
}

impl RpcMessage {
    /// Get the request ID if this message has one
    pub fn request_id(&self) -> Option<RequestId> {
        match self {
            Self::Heartbeat(_) => None,
            Self::HeartbeatAck { .. } => None,
            Self::LeaveNotification { .. } => None,
            Self::ActorInvoke { request_id, .. } => Some(*request_id),
            Self::ActorInvokeResponse { request_id, .. } => Some(*request_id),
            Self::MigratePrepare { request_id, .. } => Some(*request_id),
            Self::MigratePrepareResponse { request_id, .. } => Some(*request_id),
            Self::MigrateTransfer { request_id, .. } => Some(*request_id),
            Self::MigrateTransferResponse { request_id, .. } => Some(*request_id),
            Self::MigrateComplete { request_id, .. } => Some(*request_id),
            Self::MigrateCompleteResponse { request_id, .. } => Some(*request_id),
            Self::JoinRequest { request_id, .. } => Some(*request_id),
            Self::JoinResponse { request_id, .. } => Some(*request_id),
            Self::ClusterStateRequest { request_id, .. } => Some(*request_id),
            Self::ClusterStateResponse { request_id, .. } => Some(*request_id),
        }
    }

    /// Check if this is a response message
    pub fn is_response(&self) -> bool {
        matches!(
            self,
            Self::HeartbeatAck { .. }
                | Self::ActorInvokeResponse { .. }
                | Self::MigratePrepareResponse { .. }
                | Self::MigrateTransferResponse { .. }
                | Self::MigrateCompleteResponse { .. }
                | Self::JoinResponse { .. }
                | Self::ClusterStateResponse { .. }
        )
    }

    /// Get the actor ID if this message involves an actor
    pub fn actor_id(&self) -> Option<&ActorId> {
        match self {
            Self::ActorInvoke { actor_id, .. } => Some(actor_id),
            Self::MigratePrepare { actor_id, .. } => Some(actor_id),
            Self::MigrateTransfer { actor_id, .. } => Some(actor_id),
            Self::MigrateComplete { actor_id, .. } => Some(actor_id),
            _ => None,
        }
    }
}

/// RPC transport trait
///
/// Abstracts the underlying transport mechanism (TCP, in-memory, etc.)
#[async_trait]
pub trait RpcTransport: Send + Sync {
    /// Send a message to a specific node
    async fn send(&self, target: &NodeId, message: RpcMessage) -> ClusterResult<()>;

    /// Send a message and wait for response
    async fn send_and_recv(
        &self,
        target: &NodeId,
        message: RpcMessage,
        timeout: Duration,
    ) -> ClusterResult<RpcMessage>;

    /// Broadcast a message to all known nodes
    async fn broadcast(&self, message: RpcMessage) -> ClusterResult<()>;

    /// Register a handler for incoming messages
    async fn set_handler(&self, handler: Box<dyn RpcHandler>) -> ClusterResult<()>;

    /// Start listening for connections
    async fn start(&self) -> ClusterResult<()>;

    /// Stop the transport
    async fn stop(&self) -> ClusterResult<()>;

    /// Get the local address
    fn local_addr(&self) -> SocketAddr;
}

/// Handler for incoming RPC messages
#[async_trait]
pub trait RpcHandler: Send + Sync {
    /// Handle an incoming message
    ///
    /// Returns an optional response message.
    async fn handle(&self, from: &NodeId, message: RpcMessage) -> Option<RpcMessage>;
}

/// In-memory RPC transport for testing
///
/// Messages are delivered directly through channels, simulating network behavior.
pub struct MemoryTransport<RT: Runtime> {
    /// Local node ID
    node_id: NodeId,
    /// Local address
    addr: SocketAddr,
    /// Runtime for task spawning
    runtime: RT,
    /// Sender channels to other nodes
    senders: std::sync::Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<NodeId, tokio::sync::mpsc::Sender<(NodeId, RpcMessage)>>,
        >,
    >,
    /// Receiver for incoming messages
    receiver: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<(NodeId, RpcMessage)>>>,
    /// Handler for incoming messages
    handler: tokio::sync::RwLock<Option<Box<dyn RpcHandler>>>,
    /// Pending requests waiting for responses
    pending: tokio::sync::RwLock<
        std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<RpcMessage>>,
    >,
    /// Next request ID
    next_request_id: std::sync::atomic::AtomicU64,
    /// Whether the transport is running
    running: std::sync::atomic::AtomicBool,
}

impl<RT: Runtime> MemoryTransport<RT> {
    /// Create a new in-memory transport
    pub fn new(node_id: NodeId, addr: SocketAddr, runtime: RT) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(1000);

        Self {
            node_id: node_id.clone(),
            addr,
            runtime,
            senders: std::sync::Arc::new(tokio::sync::RwLock::new({
                let mut map = std::collections::HashMap::new();
                map.insert(node_id, tx);
                map
            })),
            receiver: tokio::sync::Mutex::new(Some(rx)),
            handler: tokio::sync::RwLock::new(None),
            pending: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            next_request_id: std::sync::atomic::AtomicU64::new(1),
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Connect to another transport
    ///
    /// Note: This is a simplified implementation for testing.
    /// In production, actual TCP connections would be established.
    #[allow(dead_code)]
    pub async fn connect(&self, other: &MemoryTransport<RT>) {
        let mut senders = self.senders.write().await;
        let mut other_senders = other.senders.write().await;

        // Create channel from self to other
        let (tx_to_other, _rx_to_other) = tokio::sync::mpsc::channel(1000);
        senders.insert(other.node_id.clone(), tx_to_other);

        // Create channel from other to self
        let (tx_to_self, _rx_to_self) = tokio::sync::mpsc::channel(1000);
        other_senders.insert(self.node_id.clone(), tx_to_self);
    }

    /// Get next request ID
    pub fn next_request_id(&self) -> RequestId {
        self.next_request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Process incoming messages
    async fn process_messages(
        mut receiver: tokio::sync::mpsc::Receiver<(NodeId, RpcMessage)>,
        handler: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn RpcHandler>>>>,
        pending: std::sync::Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<RpcMessage>>,
            >,
        >,
        senders: std::sync::Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<NodeId, tokio::sync::mpsc::Sender<(NodeId, RpcMessage)>>,
            >,
        >,
        local_node_id: NodeId,
    ) {
        while let Some((from, message)) = receiver.recv().await {
            // Check if this is a response to a pending request
            if message.is_response() {
                if let Some(request_id) = message.request_id() {
                    let mut pending = pending.write().await;
                    if let Some(sender) = pending.remove(&request_id) {
                        let _ = sender.send(message);
                        continue;
                    }
                }
            }

            // Handle as incoming message
            let handler_guard = handler.read().await;
            if let Some(ref h) = *handler_guard {
                if let Some(response) = h.handle(&from, message).await {
                    // Send response back to the sender
                    let senders = senders.read().await;
                    if let Some(sender) = senders.get(&from) {
                        let _ = sender.send((local_node_id.clone(), response)).await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl<RT: Runtime + 'static> RpcTransport for MemoryTransport<RT> {
    async fn send(&self, target: &NodeId, message: RpcMessage) -> ClusterResult<()> {
        let senders = self.senders.read().await;
        let sender = senders
            .get(target)
            .ok_or_else(|| ClusterError::node_unreachable(target, "no connection"))?;

        sender
            .send((self.node_id.clone(), message))
            .await
            .map_err(|_| ClusterError::node_unreachable(target, "channel closed"))?;

        Ok(())
    }

    async fn send_and_recv(
        &self,
        target: &NodeId,
        message: RpcMessage,
        timeout: Duration,
    ) -> ClusterResult<RpcMessage> {
        let request_id = message.request_id().ok_or_else(|| ClusterError::Internal {
            message: "message has no request ID".into(),
        })?;

        // Set up response channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.write().await;
            pending.insert(request_id, tx);
        }

        // Send the message
        self.send(target, message).await?;

        // Wait for response with timeout
        // Note: We use self.runtime.timeout if available, but Runtime trait doesn't have timeout method that returns Result<T, Elapsed> easily compatible here without mapping.
        // Actually Runtime::timeout returns Result<T, ()>.
        // tokio::time::timeout returns Result<T, Elapsed>.
        // Let's use runtime.timeout and map error.

        match self.runtime.timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(ClusterError::rpc_failed(target, "response channel closed")),
            Err(_) => {
                // Remove pending request
                let mut pending = self.pending.write().await;
                pending.remove(&request_id);
                Err(ClusterError::rpc_timeout(
                    target,
                    timeout.as_millis() as u64,
                ))
            }
        }
    }

    async fn broadcast(&self, message: RpcMessage) -> ClusterResult<()> {
        let senders = self.senders.read().await;
        for (node_id, sender) in senders.iter() {
            if node_id != &self.node_id {
                let _ = sender.send((self.node_id.clone(), message.clone())).await;
            }
        }
        Ok(())
    }

    async fn set_handler(&self, handler: Box<dyn RpcHandler>) -> ClusterResult<()> {
        let mut h = self.handler.write().await;
        *h = Some(handler);
        Ok(())
    }

    async fn start(&self) -> ClusterResult<()> {
        if self.running.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ClusterError::AlreadyStarted);
        }

        // Start message processing task
        let receiver = {
            let mut rx = self.receiver.lock().await;
            rx.take().ok_or_else(|| ClusterError::Internal {
                message: "receiver already taken".into(),
            })?
        };

        let handler = std::sync::Arc::new(
            self.handler
                .write()
                .await
                .take()
                .map(|h| tokio::sync::RwLock::new(Some(h)))
                .unwrap_or_else(|| tokio::sync::RwLock::new(None)),
        );

        let pending =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

        let senders = self.senders.clone();
        let local_node_id = self.node_id.clone();

        // Fire-and-forget background task
        std::mem::drop(self.runtime.spawn(Self::process_messages(
            receiver,
            handler,
            pending,
            senders,
            local_node_id,
        )));

        Ok(())
    }

    async fn stop(&self) -> ClusterResult<()> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

/// TCP-based RPC transport for real network communication
///
/// Wire protocol: [4-byte big-endian length][JSON payload]
pub struct TcpTransport<RT: Runtime> {
    /// Local node ID
    node_id: NodeId,
    /// Local listening address
    local_addr: SocketAddr,
    /// Runtime for task spawning
    runtime: RT,
    /// Active connections to other nodes
    connections:
        std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<NodeId, TcpConnection>>>,
    /// Node ID to address mapping
    node_addrs: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<NodeId, SocketAddr>>>,
    /// Handler for incoming messages
    handler: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn RpcHandler>>>>,
    /// Pending requests waiting for responses
    pending: std::sync::Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<RpcMessage>>,
        >,
    >,
    /// Next request ID
    next_request_id: std::sync::atomic::AtomicU64,
    /// Whether the transport is running
    running: std::sync::atomic::AtomicBool,
    /// Shutdown signal sender
    shutdown_tx: tokio::sync::RwLock<Option<tokio::sync::broadcast::Sender<()>>>,
}

/// A TCP connection to another node
struct TcpConnection {
    /// Sender for outgoing messages
    sender: tokio::sync::mpsc::Sender<RpcMessage>,
}

impl<RT: Runtime + 'static> TcpTransport<RT> {
    /// Create a new TCP transport
    pub fn new(node_id: NodeId, local_addr: SocketAddr, runtime: RT) -> Self {
        Self {
            node_id,
            local_addr,
            runtime,
            connections: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            node_addrs: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            handler: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            pending: std::sync::Arc::new(
                tokio::sync::RwLock::new(std::collections::HashMap::new()),
            ),
            next_request_id: std::sync::atomic::AtomicU64::new(1),
            running: std::sync::atomic::AtomicBool::new(false),
            shutdown_tx: tokio::sync::RwLock::new(None),
        }
    }

    /// Register a node's address
    pub async fn register_node(&self, node_id: NodeId, addr: SocketAddr) {
        let mut addrs = self.node_addrs.write().await;
        addrs.insert(node_id, addr);
    }

    /// Get next request ID
    pub fn next_request_id(&self) -> RequestId {
        self.next_request_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Get or create connection to a node
    async fn get_or_create_connection(
        &self,
        target: &NodeId,
    ) -> ClusterResult<tokio::sync::mpsc::Sender<RpcMessage>> {
        // Check existing connection
        {
            let connections = self.connections.read().await;
            if let Some(conn) = connections.get(target) {
                return Ok(conn.sender.clone());
            }
        }

        // Get target address
        let addr = {
            let addrs = self.node_addrs.read().await;
            addrs
                .get(target)
                .cloned()
                .ok_or_else(|| ClusterError::node_unreachable(target, "address not registered"))?
        };

        // Create new connection
        let stream = tokio::net::TcpStream::connect(addr).await.map_err(|e| {
            ClusterError::node_unreachable(target, format!("connection failed: {}", e))
        })?;

        let (read_half, write_half) = stream.into_split();

        // Create channel for outgoing messages
        let (tx, rx) = tokio::sync::mpsc::channel::<RpcMessage>(100);

        // Spawn writer task (fire-and-forget)
        let target_clone = target.clone();
        std::mem::drop(
            self.runtime
                .spawn(Self::writer_task(write_half, rx, target_clone.clone())),
        );

        // Spawn reader task
        let pending = self.pending.clone();
        let node_id = self.node_id.clone();
        let connections = self.connections.clone();
        let handler = self.handler.clone();
        let response_sender = tx.clone();

        // Fire-and-forget background task
        std::mem::drop(self.runtime.spawn(async move {
            Self::reader_task(
                read_half,
                pending,
                target_clone.clone(),
                node_id,
                handler,
                response_sender,
            )
            .await;
            // Remove connection on disconnect
            let mut conns = connections.write().await;
            conns.remove(&target_clone);
        }));

        // Store connection
        {
            let mut connections = self.connections.write().await;
            connections.insert(target.clone(), TcpConnection { sender: tx.clone() });
        }

        Ok(tx)
    }

    /// Writer task - sends messages over TCP
    async fn writer_task(
        mut write_half: tokio::net::tcp::OwnedWriteHalf,
        mut rx: tokio::sync::mpsc::Receiver<RpcMessage>,
        target: NodeId,
    ) {
        use tokio::io::AsyncWriteExt;

        while let Some(message) = rx.recv().await {
            // Serialize message
            let json = match serde_json::to_vec(&message) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to serialize message");
                    continue;
                }
            };

            // Write length prefix (4 bytes big-endian)
            let len = json.len() as u32;
            let len_bytes = len.to_be_bytes();

            if let Err(e) = write_half.write_all(&len_bytes).await {
                tracing::error!(node = %target, error = %e, "Failed to write length");
                break;
            }

            // Write payload
            if let Err(e) = write_half.write_all(&json).await {
                tracing::error!(node = %target, error = %e, "Failed to write payload");
                break;
            }

            if let Err(e) = write_half.flush().await {
                tracing::error!(node = %target, error = %e, "Failed to flush");
                break;
            }
        }

        tracing::debug!(node = %target, "Writer task exiting");
    }

    /// Reader task - reads messages from TCP
    async fn reader_task(
        mut read_half: tokio::net::tcp::OwnedReadHalf,
        pending: std::sync::Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<RpcMessage>>,
            >,
        >,
        from_node: NodeId,
        _local_node: NodeId,
        handler: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn RpcHandler>>>>,
        response_sender: tokio::sync::mpsc::Sender<RpcMessage>,
    ) {
        use tokio::io::AsyncReadExt;

        loop {
            // Read length prefix (4 bytes)
            let mut len_bytes = [0u8; 4];
            if let Err(e) = read_half.read_exact(&mut len_bytes).await {
                if e.kind() != std::io::ErrorKind::UnexpectedEof {
                    tracing::error!(node = %from_node, error = %e, "Failed to read length");
                }
                break;
            }

            let len = u32::from_be_bytes(len_bytes) as usize;
            if len > 10 * 1024 * 1024 {
                // 10MB limit
                tracing::error!(node = %from_node, len = len, "Message too large");
                break;
            }

            // Read payload
            let mut buffer = vec![0u8; len];
            if let Err(e) = read_half.read_exact(&mut buffer).await {
                tracing::error!(node = %from_node, error = %e, "Failed to read payload");
                break;
            }

            // Parse message
            let message: RpcMessage = match serde_json::from_slice(&buffer) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(node = %from_node, error = %e, "Failed to parse message");
                    continue;
                }
            };

            tracing::debug!(node = %from_node, msg_type = ?std::mem::discriminant(&message), "Received message");

            // Check if this is a response to a pending request
            if message.is_response() {
                if let Some(request_id) = message.request_id() {
                    let mut pending = pending.write().await;
                    if let Some(sender) = pending.remove(&request_id) {
                        let _ = sender.send(message);
                        continue;
                    }
                }
            }

            // Handle incoming request via RpcHandler
            let handler_guard = handler.read().await;
            if let Some(ref h) = *handler_guard {
                if let Some(response) = h.handle(&from_node, message).await {
                    // Send response back to the sender
                    if let Err(e) = response_sender.send(response).await {
                        tracing::error!(node = %from_node, error = %e, "Failed to send response");
                    }
                }
            } else {
                tracing::debug!(node = %from_node, "No handler registered, ignoring request");
            }
        }

        tracing::debug!(node = %from_node, "Reader task exiting");
    }

    /// Accept task - handles incoming connections
    async fn accept_task(
        listener: tokio::net::TcpListener,
        connections: std::sync::Arc<
            tokio::sync::RwLock<std::collections::HashMap<NodeId, TcpConnection>>,
        >,
        pending: std::sync::Arc<
            tokio::sync::RwLock<
                std::collections::HashMap<RequestId, tokio::sync::oneshot::Sender<RpcMessage>>,
            >,
        >,
        handler: std::sync::Arc<tokio::sync::RwLock<Option<Box<dyn RpcHandler>>>>,
        local_node: NodeId,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer_addr)) => {
                            tracing::info!(peer = %peer_addr, "Accepted connection");

                            let (read_half, write_half) = stream.into_split();

                            // Create a temporary node ID based on peer address
                            // In a real implementation, we'd do a handshake to get the node ID
                            let temp_node_id = NodeId::new(format!("peer-{}", peer_addr)).unwrap();

                            // Create channel for outgoing messages
                            let (tx, rx) = tokio::sync::mpsc::channel::<RpcMessage>(100);

                            // Spawn writer task (fire-and-forget)
                            let node_clone = temp_node_id.clone();
                            std::mem::drop(kelpie_core::current_runtime()
                                .spawn(Self::writer_task(write_half, rx, node_clone)));

                            // Spawn reader task
                            let pending_clone = pending.clone();
                            let local_node_clone = local_node.clone();
                            let node_clone = temp_node_id.clone();
                            let connections_clone = connections.clone();
                            let handler_clone = handler.clone();
                            let response_sender = tx.clone();

                            // Fire-and-forget background task
                            std::mem::drop(kelpie_core::current_runtime().spawn(async move {
                                Self::reader_task(
                                    read_half,
                                    pending_clone,
                                    node_clone.clone(),
                                    local_node_clone,
                                    handler_clone,
                                    response_sender,
                                ).await;
                                // Remove connection on disconnect
                                let mut conns = connections_clone.write().await;
                                conns.remove(&node_clone);
                            }));

                            // Store connection
                            let mut conns = connections.write().await;
                            conns.insert(temp_node_id, TcpConnection { sender: tx });
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Accept failed");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Accept task shutting down");
                    break;
                }
            }
        }
    }
}

#[async_trait]
impl<RT: Runtime + 'static> RpcTransport for TcpTransport<RT> {
    async fn send(&self, target: &NodeId, message: RpcMessage) -> ClusterResult<()> {
        let sender = self.get_or_create_connection(target).await?;

        sender
            .send(message)
            .await
            .map_err(|_| ClusterError::node_unreachable(target, "send channel closed"))?;

        Ok(())
    }

    async fn send_and_recv(
        &self,
        target: &NodeId,
        message: RpcMessage,
        timeout: Duration,
    ) -> ClusterResult<RpcMessage> {
        let request_id = message.request_id().ok_or_else(|| ClusterError::Internal {
            message: "message has no request ID".into(),
        })?;

        // Set up response channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.write().await;
            pending.insert(request_id, tx);
        }

        // Send the message
        if let Err(e) = self.send(target, message).await {
            let mut pending = self.pending.write().await;
            pending.remove(&request_id);
            return Err(e);
        }

        // Wait for response with timeout
        match self.runtime.timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                let mut pending = self.pending.write().await;
                pending.remove(&request_id);
                Err(ClusterError::rpc_failed(target, "response channel closed"))
            }
            Err(_) => {
                let mut pending = self.pending.write().await;
                pending.remove(&request_id);
                Err(ClusterError::rpc_timeout(
                    target,
                    timeout.as_millis() as u64,
                ))
            }
        }
    }

    async fn broadcast(&self, message: RpcMessage) -> ClusterResult<()> {
        let connections = self.connections.read().await;
        for (node_id, conn) in connections.iter() {
            if node_id != &self.node_id {
                let _ = conn.sender.send(message.clone()).await;
            }
        }
        Ok(())
    }

    async fn set_handler(&self, handler: Box<dyn RpcHandler>) -> ClusterResult<()> {
        let mut h = self.handler.write().await;
        *h = Some(handler);
        Ok(())
    }

    async fn start(&self) -> ClusterResult<()> {
        if self.running.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ClusterError::AlreadyStarted);
        }

        // Start TCP listener
        let listener = tokio::net::TcpListener::bind(self.local_addr)
            .await
            .map_err(|e| ClusterError::Internal {
                message: format!("failed to bind to {}: {}", self.local_addr, e),
            })?;

        tracing::info!(addr = %self.local_addr, node = %self.node_id, "TCP transport started");

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        {
            let mut tx = self.shutdown_tx.write().await;
            *tx = Some(shutdown_tx);
        }

        // Spawn accept task
        let connections =
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let pending = self.pending.clone();
        let handler = self.handler.clone();
        let local_node = self.node_id.clone();

        // Fire-and-forget background task
        std::mem::drop(self.runtime.spawn(Self::accept_task(
            listener,
            connections,
            pending,
            handler,
            local_node,
            shutdown_rx,
        )));

        Ok(())
    }

    async fn stop(&self) -> ClusterResult<()> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }

        // Close all connections
        let mut connections = self.connections.write().await;
        connections.clear();

        tracing::info!(node = %self.node_id, "TCP transport stopped");

        Ok(())
    }

    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelpie_core::TokioRuntime;
    use kelpie_registry::NodeStatus;

    fn test_node_id(n: u32) -> NodeId {
        NodeId::new(format!("node-{}", n)).unwrap()
    }

    fn test_addr(port: u16) -> SocketAddr {
        use std::net::{IpAddr, Ipv4Addr};
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    fn test_runtime() -> TokioRuntime {
        TokioRuntime
    }

    #[test]
    fn test_rpc_message_request_id() {
        let actor_id = ActorId::new("test", "actor-1").unwrap();
        let msg = RpcMessage::ActorInvoke {
            request_id: 42,
            actor_id,
            operation: "test".into(),
            payload: Bytes::new(),
        };
        assert_eq!(msg.request_id(), Some(42));

        let hb = RpcMessage::Heartbeat(Heartbeat::new(
            test_node_id(1),
            1000,
            NodeStatus::Active,
            0,
            100,
            1,
        ));
        assert_eq!(hb.request_id(), None);
    }

    #[test]
    fn test_rpc_message_is_response() {
        let resp = RpcMessage::ActorInvokeResponse {
            request_id: 1,
            result: Ok(Bytes::new()),
        };
        assert!(resp.is_response());

        let req = RpcMessage::ActorInvoke {
            request_id: 1,
            actor_id: ActorId::new("test", "a").unwrap(),
            operation: "op".into(),
            payload: Bytes::new(),
        };
        assert!(!req.is_response());
    }

    #[test]
    fn test_rpc_message_actor_id() {
        let actor_id = ActorId::new("test", "actor-1").unwrap();
        let msg = RpcMessage::ActorInvoke {
            request_id: 1,
            actor_id: actor_id.clone(),
            operation: "test".into(),
            payload: Bytes::new(),
        };
        assert_eq!(msg.actor_id(), Some(&actor_id));

        let hb = RpcMessage::HeartbeatAck {
            node_id: test_node_id(1),
            sequence: 1,
        };
        assert!(hb.actor_id().is_none());
    }

    #[tokio::test]
    async fn test_memory_transport_create() {
        let transport = MemoryTransport::new(test_node_id(1), test_addr(9001), test_runtime());
        assert_eq!(transport.local_addr(), test_addr(9001));
    }

    #[tokio::test]
    async fn test_memory_transport_request_id() {
        let transport = MemoryTransport::new(test_node_id(1), test_addr(9001), test_runtime());
        let id1 = transport.next_request_id();
        let id2 = transport.next_request_id();
        assert_eq!(id1 + 1, id2);
    }

    #[tokio::test]
    async fn test_tcp_transport_create() {
        let transport = TcpTransport::new(test_node_id(1), test_addr(19001), test_runtime());
        assert_eq!(transport.local_addr(), test_addr(19001));
    }

    #[tokio::test]
    async fn test_tcp_transport_request_id() {
        let transport = TcpTransport::new(test_node_id(1), test_addr(19002), test_runtime());
        let id1 = transport.next_request_id();
        let id2 = transport.next_request_id();
        assert_eq!(id1 + 1, id2);
    }

    #[tokio::test]
    async fn test_tcp_transport_start_stop() {
        let transport = TcpTransport::new(test_node_id(1), test_addr(19003), test_runtime());

        // Start the transport
        if let Err(e) = transport.start().await {
            if let ClusterError::Internal { message } = &e {
                if message.contains("Operation not permitted") {
                    eprintln!("Skipping TCP transport start/stop test: {}", message);
                    return;
                }
            }
            panic!("Failed to start transport: {:?}", e);
        }

        // Should fail if already started
        let result = transport.start().await;
        assert!(matches!(result, Err(ClusterError::AlreadyStarted)));

        // Stop the transport
        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_tcp_transport_two_nodes() {
        // Create two transports on different ports
        let node1_id = test_node_id(1);
        let node2_id = test_node_id(2);
        let addr1 = test_addr(19004);
        let addr2 = test_addr(19005);

        let transport1 = TcpTransport::new(node1_id.clone(), addr1, test_runtime());
        let transport2 = TcpTransport::new(node2_id.clone(), addr2, test_runtime());

        // Start both
        if let Err(e) = transport1.start().await {
            if let ClusterError::Internal { message } = &e {
                if message.contains("Operation not permitted") {
                    eprintln!("Skipping TCP transport two-nodes test: {}", message);
                    return;
                }
            }
            panic!("Failed to start transport1: {:?}", e);
        }
        if let Err(e) = transport2.start().await {
            if let ClusterError::Internal { message } = &e {
                if message.contains("Operation not permitted") {
                    eprintln!("Skipping TCP transport two-nodes test: {}", message);
                    transport1.stop().await.ok();
                    return;
                }
            }
            panic!("Failed to start transport2: {:?}", e);
        }

        // Register each other's addresses
        transport1.register_node(node2_id.clone(), addr2).await;
        transport2.register_node(node1_id.clone(), addr1).await;

        // Give the listeners time to start
        kelpie_core::current_runtime()
            .sleep(std::time::Duration::from_millis(10))
            .await;

        // Send a heartbeat from node1 to node2
        let heartbeat = RpcMessage::Heartbeat(Heartbeat::new(
            node1_id.clone(),
            1000,
            NodeStatus::Active,
            0,
            100,
            1,
        ));

        transport1.send(&node2_id, heartbeat).await.unwrap();

        // Give time for message to be received
        kelpie_core::current_runtime()
            .sleep(std::time::Duration::from_millis(10))
            .await;

        // Stop both
        transport1.stop().await.unwrap();
        transport2.stop().await.unwrap();
    }
}
