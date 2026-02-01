//! Server state management
//!
//! TigerStyle: Thread-safe shared state with explicit locking.
//!
//! DST Support: Optional fault injection for deterministic simulation testing.
//!
//! Storage Integration: AgentStorage backend is REQUIRED for all operations.
//! Storage is the single source of truth - FDB for production, SimStorage for tests.
//! In-memory HashMaps are deprecated and will be removed (Issue #74).

use crate::actor::{AgentActor, RealLlmAdapter};
use crate::llm::LlmClient;
use crate::models::ArchivalEntry;
use crate::models::{AgentGroup, AgentState, BatchStatus, Block, Job, Message, Project};
use crate::security::audit::{new_shared_log, SharedAuditLog};
use crate::service::AgentService;
use crate::storage::{AgentStorage, SimStorage, StorageError};
use crate::tools::UnifiedToolRegistry;
use chrono::Utc;
use kelpie_runtime::{CloneFactory, Dispatcher, DispatcherConfig, DispatcherHandle};
use kelpie_storage::memory::MemoryKV;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use uuid::Uuid;

use kelpie_core::io::{TimeProvider, WallClockTime};

#[cfg(feature = "dst")]
use kelpie_dst::fault::FaultInjector;

/// Maximum agents per server instance
pub const AGENTS_COUNT_MAX: usize = 100_000;

/// Maximum messages per agent
pub const MESSAGES_PER_AGENT_MAX: usize = 10_000;

/// Maximum archival entries per agent
pub const ARCHIVAL_ENTRIES_PER_AGENT_MAX: usize = 100_000;

/// Maximum standalone blocks (configurable via KELPIE_BLOCKS_COUNT_MAX env var)
pub fn blocks_count_max() -> usize {
    use std::sync::OnceLock;
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("KELPIE_BLOCKS_COUNT_MAX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100_000)
    })
}

/// Tool information for API responses
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// Unique tool ID
    pub id: String,
    /// Tool name (used for invocation)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON schema for tool input
    pub input_schema: serde_json::Value,
    /// Source code (for custom tools, None for client-side tools)
    pub source: Option<String>,
    /// Whether tool requires user approval before execution
    pub default_requires_approval: bool,
    /// Tool type: "builtin", "custom", "client"
    pub tool_type: String,
    /// Tags for categorization (Letta compatibility)
    pub tags: Option<Vec<String>>,
    /// Character limit for return value (Letta compatibility)
    pub return_char_limit: Option<u32>,
}

/// Server-wide shared state
#[derive(Clone)]
pub struct AppState<R: kelpie_core::Runtime> {
    inner: Arc<AppStateInner<R>>,
}

struct AppStateInner<R: kelpie_core::Runtime> {
    /// NEW Phase 5: Actor-based agent service (None for backward compat)
    agent_service: Option<AgentService<R>>,
    /// NEW Phase 5: Actor runtime dispatcher handle (None for backward compat)
    dispatcher: Option<DispatcherHandle<R>>,
    /// Runtime for spawning tasks
    runtime: R,
    /// Time provider for DST compatibility
    time: Arc<dyn TimeProvider>,
    /// NEW Phase 5: Shutdown coordination channel
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,

    /// Agent storage by ID (in-memory hot cache)
    /// TODO Phase 6: Remove after HTTP handlers migrated to agent_service
    agents: RwLock<HashMap<String, AgentState>>,
    /// Messages by agent ID (in-memory hot cache)
    messages: RwLock<HashMap<String, Vec<Message>>>,
    /// Unified tool registry for execution
    tool_registry: Arc<UnifiedToolRegistry>,
    /// Client-side tools (tools that require approval or execute client-side)
    client_tools: RwLock<HashMap<String, ToolInfo>>,
    /// Archival memory entries by agent ID
    archival: RwLock<HashMap<String, Vec<ArchivalEntry>>>,
    /// Standalone blocks by ID (for letta-code compatibility)
    blocks: RwLock<HashMap<String, Block>>,
    /// MCP servers by ID (Letta compatibility)
    mcp_servers: RwLock<HashMap<String, crate::models::MCPServer>>,
    /// Scheduled jobs by ID (Phase 5: Scheduling)
    jobs: RwLock<HashMap<String, Job>>,
    /// Projects by ID (Phase 6: Projects)
    projects: RwLock<HashMap<String, Project>>,
    /// Batch message statuses by ID (Phase 7)
    batches: RwLock<HashMap<String, crate::models::BatchStatus>>,
    /// Agent groups by ID (Phase 8)
    agent_groups: RwLock<HashMap<String, crate::models::AgentGroup>>,
    /// Identities by ID
    identities: RwLock<HashMap<String, crate::models::Identity>>,
    /// Audit log for forensics and compliance
    audit_log: SharedAuditLog,
    /// Server start time for uptime calculation (monotonic ms)
    start_time_ms: u64,
    /// LLM client (None if no API key configured)
    llm: Option<LlmClient>,
    /// Storage backend (always set - SimStorage for dev/tests, FDB for production)
    /// Issue #74: Now always initialized - storage is single source of truth
    storage: Option<Arc<dyn AgentStorage>>,
    /// Prometheus metrics registry (None if metrics disabled or otel feature not enabled)
    #[cfg(feature = "otel")]
    prometheus_registry: Option<Arc<prometheus::Registry>>,
    /// Fault injector for DST testing (None in production)
    #[cfg(feature = "dst")]
    fault_injector: Option<Arc<FaultInjector>>,
}

impl<R: kelpie_core::Runtime + 'static> AppState<R> {
    /// Create new server state with runtime
    pub fn new(runtime: R) -> Self {
        Self::with_registry(runtime, None)
    }

    /// Create new server state with runtime and optional Prometheus registry
    #[cfg(feature = "otel")]
    pub fn with_registry(runtime: R, registry: Option<&prometheus::Registry>) -> Self {
        let llm = LlmClient::from_env();
        if llm.is_some() {
            tracing::info!("LLM integration enabled");
        } else {
            tracing::warn!(
                "No LLM API key found. Set ANTHROPIC_API_KEY or OPENAI_API_KEY for real responses."
            );
        }

        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());
        let audit_log = new_shared_log();

        // Phase 6.4: Create AgentService and Dispatcher for production
        let (agent_service, dispatcher, shutdown_tx) = if let Some(ref llm_client) = llm {
            tracing::info!("Initializing actor-based agent service");

            // Create LLM adapter for actor
            let llm_adapter: Arc<dyn crate::actor::LlmClient> =
                Arc::new(RealLlmAdapter::new(llm_client.clone()));

            // Create AgentActor with audit logging
            let actor = AgentActor::new(llm_adapter, tool_registry.clone())
                .with_audit_log(audit_log.clone());

            // Create CloneFactory for dispatcher
            let factory = Arc::new(CloneFactory::new(actor));

            // Use MemoryKV for actor storage (TODO: production will use FDB)
            let kv = Arc::new(MemoryKV::new());

            // Create Dispatcher
            let mut dispatcher =
                Dispatcher::new(factory, kv, DispatcherConfig::default(), runtime.clone());
            let handle = dispatcher.handle();

            // Spawn dispatcher runtime
            drop(runtime.spawn(async move {
                dispatcher.run().await;
            }));

            // Create service
            let service = AgentService::with_tool_registry_and_audit(
                handle.clone(),
                tool_registry.clone(),
                audit_log.clone(),
            );

            // Create shutdown channel
            let (tx, _rx) = tokio::sync::broadcast::channel(1);

            (Some(service), Some(handle), Some(tx))
        } else {
            tracing::warn!("Actor service disabled - no LLM client configured");
            (None, None, None)
        };

        tracing::info!("Audit logging enabled");

        Self {
            inner: Arc::new(AppStateInner {
                agent_service,
                dispatcher,
                runtime,
                time: time.clone(),
                shutdown_tx,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log,
                start_time_ms: time.monotonic_ms(),
                llm,
                // Issue #74: Always create storage - SimStorage for in-memory mode
                storage: Some(Arc::new(SimStorage::new())),
                prometheus_registry: registry.map(|r| Arc::new(r.clone())),
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Create new server state without Prometheus registry (when otel feature not enabled)
    #[cfg(not(feature = "otel"))]
    pub fn with_registry(runtime: R, _registry: Option<()>) -> Self {
        let llm = LlmClient::from_env();
        if llm.is_some() {
            tracing::info!("LLM integration enabled");
        } else {
            tracing::warn!(
                "No LLM API key found. Set ANTHROPIC_API_KEY or OPENAI_API_KEY for real responses."
            );
        }

        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());
        let audit_log = new_shared_log();

        // Phase 6.4: Create AgentService and Dispatcher for production
        let (agent_service, dispatcher, shutdown_tx) = if let Some(ref llm_client) = llm {
            tracing::info!("Initializing actor-based agent service");

            // Create LLM adapter for actor
            let llm_adapter: Arc<dyn crate::actor::LlmClient> =
                Arc::new(RealLlmAdapter::new(llm_client.clone()));

            // Create AgentActor with audit logging
            let actor = AgentActor::new(llm_adapter, tool_registry.clone())
                .with_audit_log(audit_log.clone());

            // Create CloneFactory for dispatcher
            let factory = Arc::new(CloneFactory::new(actor));

            // Use MemoryKV for actor storage (TODO: production will use FDB)
            let kv = Arc::new(MemoryKV::new());

            // Create Dispatcher
            let mut dispatcher =
                Dispatcher::new(factory, kv, DispatcherConfig::default(), runtime.clone());
            let handle = dispatcher.handle();

            // Spawn dispatcher runtime
            drop(runtime.spawn(async move {
                dispatcher.run().await;
            }));

            // Create service
            let service = AgentService::with_tool_registry_and_audit(
                handle.clone(),
                tool_registry.clone(),
                audit_log.clone(),
            );

            // Create shutdown channel
            let (tx, _rx) = tokio::sync::broadcast::channel(1);

            (Some(service), Some(handle), Some(tx))
        } else {
            tracing::warn!("Actor service disabled - no LLM client configured");
            (None, None, None)
        };

        tracing::info!("Audit logging enabled");

        Self {
            inner: Arc::new(AppStateInner {
                agent_service,
                dispatcher,
                runtime,
                time: time.clone(),
                shutdown_tx,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log,
                start_time_ms: time.monotonic_ms(),
                llm,
                // Issue #74: Always create storage - SimStorage for in-memory mode
                storage: Some(Arc::new(SimStorage::new())),
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Create server state with durable storage backend
    ///
    /// TigerStyle: Storage enables persistence for crash recovery.
    pub fn with_storage(runtime: R, storage: Arc<dyn AgentStorage>) -> Self {
        let llm = LlmClient::from_env();
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());
        let audit_log = new_shared_log();

        // Phase 6.4: Create AgentService and Dispatcher for production
        let (agent_service, dispatcher, shutdown_tx) = if let Some(ref llm_client) = llm {
            tracing::info!("Initializing actor-based agent service");

            // Create LLM adapter for actor
            let llm_adapter: Arc<dyn crate::actor::LlmClient> =
                Arc::new(RealLlmAdapter::new(llm_client.clone()));

            // Create AgentActor with audit logging
            let actor = AgentActor::new(llm_adapter, tool_registry.clone())
                .with_audit_log(audit_log.clone());

            // Create CloneFactory for dispatcher
            let factory = Arc::new(CloneFactory::new(actor));

            // Use MemoryKV for actor storage (TODO: production will use FDB)
            let kv = Arc::new(MemoryKV::new());

            // Create Dispatcher
            let mut dispatcher =
                Dispatcher::new(factory, kv, DispatcherConfig::default(), runtime.clone());
            let handle = dispatcher.handle();

            // Spawn dispatcher runtime
            drop(runtime.spawn(async move {
                dispatcher.run().await;
            }));

            // Create service
            let service = AgentService::with_tool_registry_and_audit(
                handle.clone(),
                tool_registry.clone(),
                audit_log.clone(),
            );

            // Create shutdown channel
            let (tx, _rx) = tokio::sync::broadcast::channel(1);

            (Some(service), Some(handle), Some(tx))
        } else {
            tracing::warn!("Actor service disabled - no LLM client configured");
            (None, None, None)
        };

        tracing::info!("Audit logging enabled");

        Self {
            inner: Arc::new(AppStateInner {
                agent_service,
                dispatcher,
                runtime,
                time: time.clone(),
                shutdown_tx,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log,
                start_time_ms: time.monotonic_ms(),
                llm,
                storage: Some(storage),
                #[cfg(feature = "otel")]
                prometheus_registry: None,
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Create server state with persistent storage and prometheus registry
    #[cfg(feature = "otel")]
    pub fn with_storage_and_registry(
        runtime: R,
        storage: Arc<dyn AgentStorage>,
        registry: Option<prometheus::Registry>,
    ) -> Self {
        let llm = LlmClient::from_env();
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());
        let audit_log = new_shared_log();

        // Phase 6.4: Create AgentService and Dispatcher for production (otel)
        let (agent_service, dispatcher, shutdown_tx) = if let Some(ref llm_client) = llm {
            tracing::info!("Initializing actor-based agent service (with otel)");

            // Create LLM adapter for actor
            let llm_adapter: Arc<dyn crate::actor::LlmClient> =
                Arc::new(RealLlmAdapter::new(llm_client.clone()));

            // Create AgentActor with audit logging
            let actor = AgentActor::new(llm_adapter, tool_registry.clone())
                .with_audit_log(audit_log.clone());

            // Create CloneFactory for dispatcher
            let factory = Arc::new(CloneFactory::new(actor));

            // Use MemoryKV for actor storage (TODO: production will use FDB)
            let kv = Arc::new(MemoryKV::new());

            // Create Dispatcher
            let mut dispatcher =
                Dispatcher::new(factory, kv, DispatcherConfig::default(), runtime.clone());
            let handle = dispatcher.handle();

            // Spawn dispatcher runtime
            drop(runtime.spawn(async move {
                dispatcher.run().await;
            }));

            // Create service
            let service = AgentService::with_tool_registry_and_audit(
                handle.clone(),
                tool_registry.clone(),
                audit_log.clone(),
            );

            // Create shutdown channel
            let (tx, _rx) = tokio::sync::broadcast::channel(1);

            (Some(service), Some(handle), Some(tx))
        } else {
            tracing::warn!("Actor service disabled - no LLM client configured (otel mode)");
            (None, None, None)
        };

        tracing::info!("Audit logging enabled");

        Self {
            inner: Arc::new(AppStateInner {
                agent_service,
                dispatcher,
                runtime,
                time: time.clone(),
                shutdown_tx,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log,
                start_time_ms: time.monotonic_ms(),
                llm,
                storage: Some(storage),
                prometheus_registry: registry.map(Arc::new),
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Create server state with an explicit LLM client (test helper)
    pub fn with_llm(runtime: R, llm: LlmClient) -> Self {
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());

        Self {
            inner: Arc::new(AppStateInner {
                agent_service: None,
                dispatcher: None,
                runtime,
                time: time.clone(),
                shutdown_tx: None,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log: new_shared_log(),
                start_time_ms: time.monotonic_ms(),
                llm: Some(llm),
                storage: Some(Arc::new(SimStorage::new())),
                #[cfg(feature = "otel")]
                prometheus_registry: None,
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Create server state with fault injector for DST testing
    #[cfg(feature = "dst")]
    pub fn with_fault_injector(runtime: R, fault_injector: Arc<FaultInjector>) -> Self {
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());

        Self {
            inner: Arc::new(AppStateInner {
                agent_service: None,
                dispatcher: None,
                runtime,
                time: time.clone(),
                shutdown_tx: None,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log: new_shared_log(),
                start_time_ms: time.monotonic_ms(),
                llm: None,
                storage: Some(Arc::new(SimStorage::new())),
                #[cfg(feature = "otel")]
                prometheus_registry: None,
                fault_injector: Some(fault_injector),
            }),
        }
    }

    /// Create server state with both storage and fault injector for DST testing
    #[cfg(feature = "dst")]
    pub fn with_storage_and_faults(
        runtime: R,
        storage: Arc<dyn AgentStorage>,
        fault_injector: Arc<FaultInjector>,
    ) -> Self {
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());

        Self {
            inner: Arc::new(AppStateInner {
                agent_service: None,
                dispatcher: None,
                runtime,
                time: time.clone(),
                shutdown_tx: None,
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log: new_shared_log(),
                start_time_ms: time.monotonic_ms(),
                llm: None,
                storage: Some(storage),
                #[cfg(feature = "otel")]
                prometheus_registry: None,
                fault_injector: Some(fault_injector),
            }),
        }
    }

    /// Create AppState with AgentService and Dispatcher integration
    ///
    /// TigerStyle: This constructor enables actor-based agent management (Phase 5).
    ///
    /// # Arguments
    /// * `runtime` - Runtime for spawning tasks
    /// * `agent_service` - Service layer for agent operations
    /// * `dispatcher` - Dispatcher handle for shutdown coordination
    ///
    /// Note: This constructor is used for DST testing and will eventually
    /// replace the HashMap-based constructors after Phase 6 migration.
    pub fn with_agent_service(
        runtime: R,
        agent_service: AgentService<R>,
        dispatcher: DispatcherHandle<R>,
    ) -> Self {
        let tool_registry = Arc::new(UnifiedToolRegistry::new());
        let (shutdown_tx, _rx) = tokio::sync::broadcast::channel(1);
        let time: Arc<dyn TimeProvider> = Arc::new(WallClockTime::new());

        Self {
            inner: Arc::new(AppStateInner {
                agent_service: Some(agent_service),
                dispatcher: Some(dispatcher),
                runtime,
                time: time.clone(),
                shutdown_tx: Some(shutdown_tx),
                agents: RwLock::new(HashMap::new()),
                messages: RwLock::new(HashMap::new()),
                tool_registry,
                client_tools: RwLock::new(HashMap::new()),
                archival: RwLock::new(HashMap::new()),
                blocks: RwLock::new(HashMap::new()),
                mcp_servers: RwLock::new(HashMap::new()),
                jobs: RwLock::new(HashMap::new()),
                projects: RwLock::new(HashMap::new()),
                batches: RwLock::new(HashMap::new()),
                agent_groups: RwLock::new(HashMap::new()),
                identities: RwLock::new(HashMap::new()),
                audit_log: new_shared_log(),
                start_time_ms: time.monotonic_ms(),
                llm: None,
                // Issue #74: Always create storage - SimStorage for tests
                storage: Some(Arc::new(SimStorage::new())),
                #[cfg(feature = "otel")]
                prometheus_registry: None,
                #[cfg(feature = "dst")]
                fault_injector: None,
            }),
        }
    }

    /// Get reference to the agent service (if configured)
    ///
    /// Returns None if AppState was created without actor-based service.
    /// After Phase 6 migration, this will always return Some.
    pub fn agent_service(&self) -> Option<&AgentService<R>> {
        self.inner.agent_service.as_ref()
    }

    /// Gracefully shutdown the actor system
    ///
    /// TigerStyle: Waits for in-flight requests to complete (up to timeout).
    ///
    /// # Arguments
    /// * `timeout` - Maximum time to wait for in-flight requests
    ///
    /// # Errors
    /// Returns error if dispatcher shutdown fails
    pub async fn shutdown(&self, timeout: Duration) -> kelpie_core::Result<()> {
        // Signal shutdown to any listeners
        if let Some(tx) = &self.inner.shutdown_tx {
            let _ = tx.send(());
        }

        // Wait for in-flight requests (up to timeout)
        self.inner.runtime.sleep(timeout).await;

        // Shutdown dispatcher if present
        if let Some(dispatcher) = &self.inner.dispatcher {
            dispatcher.shutdown().await?;
        }

        Ok(())
    }

    /// Check if fault should be injected for an operation
    #[cfg(feature = "dst")]
    fn should_inject_fault(&self, operation: &str) -> Option<kelpie_dst::fault::FaultType> {
        self.inner
            .fault_injector
            .as_ref()
            .and_then(|fi| fi.should_inject(operation))
    }

    /// No fault injection in non-DST builds
    #[cfg(not(feature = "dst"))]
    fn should_inject_fault(&self, _operation: &str) -> Option<()> {
        None
    }

    /// Get reference to the LLM client (if configured)
    pub fn llm(&self) -> Option<&LlmClient> {
        self.inner.llm.as_ref()
    }

    /// Get reference to the unified tool registry
    pub fn tool_registry(&self) -> &UnifiedToolRegistry {
        &self.inner.tool_registry
    }

    /// Get the audit log
    pub fn audit_log(&self) -> &SharedAuditLog {
        &self.inner.audit_log
    }

    /// Get server uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        let now_ms = self.inner.time.monotonic_ms();
        now_ms.saturating_sub(self.inner.start_time_ms) / 1000
    }

    /// Get reference to the Prometheus registry (if configured)
    #[cfg(feature = "otel")]
    pub fn prometheus_registry(&self) -> Option<&prometheus::Registry> {
        self.inner.prometheus_registry.as_ref().map(|r| r.as_ref())
    }

    /// Check if durable storage is configured
    pub fn has_storage(&self) -> bool {
        self.inner.storage.is_some()
    }

    /// Get reference to the storage backend (if configured)
    pub fn storage(&self) -> Option<&dyn AgentStorage> {
        self.inner.storage.as_ref().map(|s| s.as_ref())
    }

    /// Get reference to the dispatcher handle (if configured)
    /// TigerStyle: Needed for agent-to-agent communication (Issue #75)
    pub fn dispatcher(&self) -> Option<&DispatcherHandle<R>> {
        self.inner.dispatcher.as_ref()
    }

    // =========================================================================
    // Async Agent Operations (Single Source of Truth)
    // =========================================================================
    //
    // All operations require AgentService (actor system). No HashMap fallback.

    /// Get an agent by ID
    ///
    /// Single source of truth: Requires AgentService (actor system).
    pub async fn get_agent_async(&self, id: &str) -> Result<Option<AgentState>, StateError> {
        let service = self.agent_service().ok_or_else(|| StateError::Internal {
            message: "AgentService not configured".to_string(),
        })?;

        match service.get_agent(id).await {
            Ok(agent) => Ok(Some(agent)),
            Err(kelpie_core::Error::ActorNotFound { .. }) => {
                // Actor not found is not an error, just means agent doesn't exist
                Ok(None)
            }
            Err(kelpie_core::Error::Internal { message })
                if message.contains("Agent not created") =>
            {
                // Actor was activated but has no agent state (never called create)
                Ok(None)
            }
            Err(e) => Err(StateError::Internal {
                message: format!("Service error: {}", e),
            }),
        }
    }

    /// Create an agent (async)
    ///
    /// Single source of truth: Requires AgentService (actor system).
    pub async fn create_agent_async(
        &self,
        request: crate::models::CreateAgentRequest,
    ) -> Result<AgentState, StateError> {
        let service = self.agent_service().ok_or_else(|| StateError::Internal {
            message: "AgentService not configured".to_string(),
        })?;

        // Single source of truth: Actor system handles all state
        let agent = service
            .create_agent(request)
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Service error: {}", e),
            })?;

        Ok(agent)
    }

    /// Update an agent
    ///
    /// Single source of truth: Requires AgentService (actor system).
    pub async fn update_agent_async(
        &self,
        id: &str,
        update: serde_json::Value,
    ) -> Result<AgentState, StateError> {
        let service = self.agent_service().ok_or_else(|| StateError::Internal {
            message: "AgentService not configured".to_string(),
        })?;

        // Single source of truth: Actor system handles all state
        let agent = service
            .update_agent(id, update)
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Service error: {}", e),
            })?;

        Ok(agent)
    }

    /// Delete an agent (async)
    ///
    /// Single source of truth: Requires AgentService (actor system).
    pub async fn delete_agent_async(&self, id: &str) -> Result<(), StateError> {
        let service = self.agent_service().ok_or_else(|| StateError::Internal {
            message: "AgentService not configured".to_string(),
        })?;

        // Single source of truth: Actor system handles deletion
        service
            .delete_agent(id)
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Service error: {}", e),
            })?;

        Ok(())
    }

    /// List agents from durable storage
    ///
    /// TigerStyle: Single source of truth - all data flows through storage.
    /// Requires storage to be configured.
    ///
    /// # Arguments
    /// * `limit` - Maximum number of agents to return
    /// * `cursor` - Pagination cursor (agent ID to start after)
    /// * `name_filter` - Optional exact name match filter (applied before pagination)
    pub async fn list_agents_async(
        &self,
        limit: usize,
        cursor: Option<&str>,
        name_filter: Option<&str>,
    ) -> Result<(Vec<AgentState>, Option<String>), StateError> {
        // Require storage - no HashMap fallback
        let storage = self
            .inner
            .storage
            .as_ref()
            .ok_or_else(|| StateError::Internal {
                message: "Storage not configured".to_string(),
            })?;

        // Load all agents from storage
        let agent_metadatas = storage
            .list_agents()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Failed to list agents from storage: {}", e),
            })?;

        // Convert AgentMetadata to AgentState
        let mut agents: Vec<AgentState> = Vec::with_capacity(agent_metadatas.len());
        for metadata in agent_metadatas {
            // Load blocks for each agent
            let blocks =
                storage
                    .load_blocks(&metadata.id)
                    .await
                    .map_err(|e| StateError::Internal {
                        message: format!("Failed to load blocks: {}", e),
                    })?;

            agents.push(AgentState {
                id: metadata.id,
                name: metadata.name,
                agent_type: metadata.agent_type,
                model: metadata.model,
                embedding: metadata.embedding,
                system: metadata.system,
                description: metadata.description,
                blocks,
                tool_ids: metadata.tool_ids,
                tags: metadata.tags,
                metadata: metadata.metadata,
                project_id: None, // TODO: Add project_id to AgentMetadata
                user_id: None,    // TODO: Add user_id to AgentMetadata
                org_id: None,     // TODO: Add org_id to AgentMetadata
                created_at: metadata.created_at,
                updated_at: metadata.updated_at,
            });
        }

        // Sort by created_at descending (newest first)
        agents.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // TigerStyle: Apply name filter BEFORE pagination to ensure correct results
        if let Some(name) = name_filter {
            agents.retain(|agent| agent.name == name);
        }

        // Apply cursor (skip until we find the cursor ID)
        let start_idx = if let Some(cursor_id) = cursor {
            agents
                .iter()
                .position(|a| a.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        // Paginate
        let page: Vec<_> = agents.into_iter().skip(start_idx).take(limit + 1).collect();
        let (items, next_cursor) = if page.len() > limit {
            let items: Vec<_> = page.into_iter().take(limit).collect();
            let next_cursor = items.last().map(|a| a.id.clone());
            (items, next_cursor)
        } else {
            (page, None)
        };

        Ok((items, next_cursor))
    }

    // =========================================================================
    // Async Persistence Operations (for durable storage)
    // =========================================================================

    /// Persist agent metadata to durable storage
    ///
    /// TigerStyle: Async operation for storage backend writes.
    /// Returns Ok(()) if no storage configured (in-memory only mode).
    pub async fn persist_agent(&self, agent: &AgentState) -> Result<(), StorageError> {
        tracing::debug!(agent_id = %agent.id, name = %agent.name, "persist_agent called");

        if let Some(storage) = &self.inner.storage {
            use crate::storage::AgentMetadata;

            let metadata = AgentMetadata {
                id: agent.id.clone(),
                name: agent.name.clone(),
                agent_type: agent.agent_type.clone(),
                model: agent.model.clone(),
                embedding: agent.embedding.clone(),
                system: agent.system.clone(),
                description: agent.description.clone(),
                tool_ids: agent.tool_ids.clone(),
                tags: agent.tags.clone(),
                metadata: agent.metadata.clone(),
                created_at: agent.created_at,
                updated_at: agent.updated_at,
            };
            tracing::debug!(agent_id = %agent.id, "calling storage.save_agent");
            storage.save_agent(&metadata).await?;
            tracing::info!(agent_id = %agent.id, "agent metadata persisted to storage");

            // Also persist blocks
            storage.save_blocks(&agent.id, &agent.blocks).await?;
        } else {
            tracing::debug!(agent_id = %agent.id, "no storage configured, skipping persist");
        }
        Ok(())
    }

    /// Persist a message to durable storage
    pub async fn persist_message(
        &self,
        agent_id: &str,
        message: &Message,
    ) -> Result<(), StorageError> {
        if let Some(storage) = &self.inner.storage {
            storage.append_message(agent_id, message).await?;
        }
        Ok(())
    }

    /// Persist a block update to durable storage
    pub async fn persist_block(&self, agent_id: &str, block: &Block) -> Result<(), StorageError> {
        if let Some(storage) = &self.inner.storage {
            storage
                .update_block(agent_id, &block.label, &block.value)
                .await?;
        }
        Ok(())
    }

    /// Load agent from storage and populate in-memory cache
    ///
    /// TigerStyle: Loads from durable storage on cache miss.
    pub async fn load_agent_from_storage(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentState>, StorageError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(None);
        };

        // Load agent metadata
        let Some(metadata) = storage.load_agent(agent_id).await? else {
            return Ok(None);
        };

        // Load blocks
        let blocks = storage.load_blocks(agent_id).await?;

        // Reconstruct AgentState from metadata + blocks
        let agent = AgentState {
            id: metadata.id,
            name: metadata.name,
            agent_type: metadata.agent_type,
            model: metadata.model,
            embedding: metadata.embedding,
            system: metadata.system,
            description: metadata.description,
            project_id: None, // Phase 6: Projects not stored in legacy storage yet
            user_id: None,    // TODO: Store user_id in AgentMetadata
            org_id: None,     // TODO: Store org_id in AgentMetadata
            blocks,
            tool_ids: metadata.tool_ids,
            tags: metadata.tags,
            metadata: metadata.metadata,
            created_at: metadata.created_at,
            updated_at: metadata.updated_at,
        };

        // HashMap cache population removed - storage is single source of truth

        Ok(Some(agent))
    }

    /// Load messages from storage for an agent
    pub async fn load_messages_from_storage(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, StorageError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(vec![]);
        };

        storage.load_messages(agent_id, limit).await
    }

    // =========================================================================
    // Agent operations (HashMap-based - DEPRECATED)
    // =========================================================================
    //
    // Phase 6.11: These methods use in-memory HashMap and are DEPRECATED.
    // Production code should use *_async() methods with AgentService instead.
    //
    // These methods are kept for backward compatibility with existing tests only.
    // They will be removed in a future phase once all tests migrate to AgentService.

    /// Create a new agent (DEPRECATED - use create_agent_async)
    #[deprecated(since = "0.1.0", note = "Use create_agent_async with AgentService")]
    pub fn create_agent(&self, agent: AgentState) -> Result<AgentState, StateError> {
        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if agents.len() >= AGENTS_COUNT_MAX {
            return Err(StateError::LimitExceeded {
                resource: "agents",
                limit: AGENTS_COUNT_MAX,
            });
        }

        if agents.contains_key(&agent.id) {
            return Err(StateError::AlreadyExists {
                resource: "agent",
                id: agent.id.clone(),
            });
        }

        let result = agent.clone();
        agents.insert(agent.id.clone(), agent);

        // Initialize empty message list for the agent
        let mut messages = self
            .inner
            .messages
            .write()
            .map_err(|_| StateError::LockPoisoned)?;
        messages.insert(result.id.clone(), Vec::new());

        // Initialize empty archival list for the agent
        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| StateError::LockPoisoned)?;
        archival.insert(result.id.clone(), Vec::new());

        Ok(result)
    }

    /// Get an agent by ID
    pub fn get_agent(&self, id: &str) -> Result<Option<AgentState>, StateError> {
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(agents.get(id).cloned())
    }

    /// List all agents with pagination
    pub fn list_agents(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<(Vec<AgentState>, Option<String>), StateError> {
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let mut all_agents: Vec<_> = agents.values().cloned().collect();
        all_agents.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        // Apply cursor (skip until we find the cursor ID)
        let start_idx = if let Some(cursor_id) = cursor {
            all_agents
                .iter()
                .position(|a| a.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let page: Vec<_> = all_agents
            .into_iter()
            .skip(start_idx)
            .take(limit + 1)
            .collect();

        // Determine next cursor
        let (items, next_cursor) = if page.len() > limit {
            let items: Vec<_> = page.into_iter().take(limit).collect();
            let next_cursor = items.last().map(|a| a.id.clone());
            (items, next_cursor)
        } else {
            (page, None)
        };

        Ok((items, next_cursor))
    }

    /// Update an agent
    pub fn update_agent(
        &self,
        id: &str,
        update: impl FnOnce(&mut AgentState),
    ) -> Result<AgentState, StateError> {
        // DST: Check for fault injection on agent write
        if self.should_inject_fault("agent_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_write".to_string(),
            });
        }

        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents.get_mut(id).ok_or_else(|| StateError::NotFound {
            resource: "agent",
            id: id.to_string(),
        })?;

        update(agent);
        Ok(agent.clone())
    }

    /// Delete an agent
    pub fn delete_agent(&self, id: &str) -> Result<(), StateError> {
        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if agents.remove(id).is_none() {
            return Err(StateError::NotFound {
                resource: "agent",
                id: id.to_string(),
            });
        }

        // Also delete messages
        let mut messages = self
            .inner
            .messages
            .write()
            .map_err(|_| StateError::LockPoisoned)?;
        messages.remove(id);

        // Also delete archival entries
        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| StateError::LockPoisoned)?;
        archival.remove(id);

        Ok(())
    }

    /// Get agent count
    pub fn agent_count(&self) -> Result<usize, StateError> {
        // TigerStyle: Read from storage when configured to maintain consistency
        // In-memory count may be stale in dual-mode operation
        if let Some(storage) = &self.inner.storage {
            // Use blocking task to call async storage method
            let storage = storage.clone();
            let count = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async move {
                    storage
                        .list_agents()
                        .await
                        .map(|agents| agents.len())
                        .map_err(|e| StateError::Internal {
                            message: format!("Failed to count agents from storage: {}", e),
                        })
                })
            })?;
            return Ok(count);
        }

        // Fallback to in-memory count
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        let count = agents.len();

        // Record metric (this is called frequently from /metrics endpoint)
        // No need to record here - we'll record in the endpoint directly

        Ok(count)
    }

    /// Calculate and record memory usage metrics
    pub fn record_memory_metrics(&self) -> Result<(), StateError> {
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let messages = self
            .inner
            .messages
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        // Calculate memory usage by tier
        let mut core_memory_bytes = 0u64;
        let mut working_memory_bytes = 0u64;
        let mut archival_memory_bytes = 0u64;
        let mut total_blocks = 0u64;

        // Core memory: agent blocks
        for agent in agents.values() {
            for block in &agent.blocks {
                let block_size = block.value.len() + block.label.len();
                core_memory_bytes += block_size as u64;
                total_blocks += 1;
            }
        }

        // Working memory: messages
        for msgs in messages.values() {
            for msg in msgs {
                let msg_size = msg.content.len();
                working_memory_bytes += msg_size as u64;
            }
        }

        // Archival memory: archival entries
        for entries in archival.values() {
            for entry in entries {
                let entry_size = entry.content.len();
                archival_memory_bytes += entry_size as u64;
            }
        }

        // Note: Memory metrics (core, working, archival) are not yet implemented
        // because they require observable gauges with callbacks, which need proper
        // lifecycle management. For now, we just calculate the values here for future use.
        let _ = (
            core_memory_bytes,
            working_memory_bytes,
            archival_memory_bytes,
            total_blocks,
        );

        Ok(())
    }

    // =========================================================================
    // Block operations
    // =========================================================================

    /// Get a memory block by agent ID and block ID
    pub fn get_block(&self, agent_id: &str, block_id: &str) -> Result<Option<Block>, StateError> {
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents.get(agent_id).ok_or_else(|| StateError::NotFound {
            resource: "agent",
            id: agent_id.to_string(),
        })?;

        Ok(agent.blocks.iter().find(|b| b.id == block_id).cloned())
    }

    /// Update a memory block
    pub fn update_block(
        &self,
        agent_id: &str,
        block_id: &str,
        update: impl FnOnce(&mut Block),
    ) -> Result<Block, StateError> {
        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?;

        let block = agent
            .blocks
            .iter_mut()
            .find(|b| b.id == block_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "block",
                id: block_id.to_string(),
            })?;

        update(block);
        Ok(block.clone())
    }

    /// List blocks for an agent
    pub fn list_blocks(&self, agent_id: &str) -> Result<Vec<Block>, StateError> {
        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents.get(agent_id).ok_or_else(|| StateError::NotFound {
            resource: "agent",
            id: agent_id.to_string(),
        })?;

        Ok(agent.blocks.clone())
    }

    /// Get a memory block by agent ID and label (for letta-code compatibility)
    pub fn get_block_by_label(
        &self,
        agent_id: &str,
        label: &str,
    ) -> Result<Option<Block>, StateError> {
        // DST: Check for fault injection
        if self.should_inject_fault("block_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "block_read".to_string(),
            });
        }

        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents.get(agent_id).ok_or_else(|| StateError::NotFound {
            resource: "agent",
            id: agent_id.to_string(),
        })?;

        Ok(agent.blocks.iter().find(|b| b.label == label).cloned())
    }

    /// Update a memory block by label (for letta-code compatibility)
    pub fn update_block_by_label(
        &self,
        agent_id: &str,
        label: &str,
        update: impl FnOnce(&mut Block),
    ) -> Result<Block, StateError> {
        // DST: Check for fault injection
        if self.should_inject_fault("block_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "block_write".to_string(),
            });
        }

        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?;

        let block = agent
            .blocks
            .iter_mut()
            .find(|b| b.label == label)
            .ok_or_else(|| StateError::NotFound {
                resource: "block",
                id: label.to_string(),
            })?;

        update(block);
        Ok(block.clone())
    }

    /// Atomically append to a block or create it if it doesn't exist.
    ///
    /// BUG-001 FIX: This method eliminates the TOCTOU race condition in core_memory_append
    /// by holding the write lock for the entire check-and-update/create operation.
    ///
    /// TigerStyle: Atomic operation prevents race between check and modification.
    pub fn append_or_create_block_by_label(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<Block, StateError> {
        // DST: Check for fault injection
        if self.should_inject_fault("block_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "block_write".to_string(),
            });
        }

        // TigerStyle: Single write lock for entire operation (atomicity)
        let mut agents = self
            .inner
            .agents
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent = agents
            .get_mut(agent_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?;

        // Find existing block or create new one - all within single lock
        if let Some(block) = agent.blocks.iter_mut().find(|b| b.label == label) {
            // Append to existing block
            block.value.push('\n');
            block.value.push_str(content);
            Ok(block.clone())
        } else {
            // Create new block and add to agent
            let block = Block::new(label, content);
            let result = block.clone();
            agent.blocks.push(block);
            Ok(result)
        }
    }

    /// Atomic append or create block
    ///
    /// Single source of truth: Requires AgentService (actor system).
    ///
    /// TigerStyle: Atomic operation prevents race between check and modification.
    pub async fn append_or_create_block_by_label_async(
        &self,
        agent_id: &str,
        label: &str,
        content: &str,
    ) -> Result<(), StateError> {
        let service = self.agent_service().ok_or_else(|| StateError::Internal {
            message: "AgentService not configured".to_string(),
        })?;

        // Use agent service (actor-based)
        service
            .core_memory_append(agent_id, label, content)
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Service error: {}", e),
            })?;
        Ok(())
    }

    // =========================================================================
    // Standalone block operations (for letta-code compatibility)
    // =========================================================================

    /// Create a standalone block
    pub fn create_standalone_block(&self, block: Block) -> Result<Block, StateError> {
        let mut blocks = self
            .inner
            .blocks
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if blocks.len() >= blocks_count_max() {
            return Err(StateError::LimitExceeded {
                resource: "blocks",
                limit: blocks_count_max(),
            });
        }

        if blocks.contains_key(&block.id) {
            return Err(StateError::AlreadyExists {
                resource: "block",
                id: block.id.clone(),
            });
        }

        let result = block.clone();
        blocks.insert(block.id.clone(), block);
        Ok(result)
    }

    /// Get a standalone block by ID
    pub fn get_standalone_block(&self, id: &str) -> Result<Option<Block>, StateError> {
        let blocks = self
            .inner
            .blocks
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(blocks.get(id).cloned())
    }

    /// List all standalone blocks with pagination
    pub fn list_standalone_blocks(
        &self,
        limit: usize,
        cursor: Option<&str>,
        label: Option<&str>,
    ) -> Result<(Vec<Block>, Option<String>), StateError> {
        let blocks = self
            .inner
            .blocks
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let mut all_blocks: Vec<_> = blocks.values().cloned().collect();

        // Filter by label if provided
        if let Some(l) = label {
            all_blocks.retain(|b| b.label == l);
        }

        all_blocks.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        // Apply cursor (skip until we find the cursor ID)
        let start_idx = if let Some(cursor_id) = cursor {
            all_blocks
                .iter()
                .position(|b| b.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let page: Vec<_> = all_blocks
            .into_iter()
            .skip(start_idx)
            .take(limit + 1)
            .collect();

        // Determine next cursor
        let (items, next_cursor) = if page.len() > limit {
            let items: Vec<_> = page.into_iter().take(limit).collect();
            let next_cursor = items.last().map(|b| b.id.clone());
            (items, next_cursor)
        } else {
            (page, None)
        };

        Ok((items, next_cursor))
    }

    /// Update a standalone block
    pub fn update_standalone_block(
        &self,
        id: &str,
        update: impl FnOnce(&mut Block),
    ) -> Result<Block, StateError> {
        let mut blocks = self
            .inner
            .blocks
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let block = blocks.get_mut(id).ok_or_else(|| StateError::NotFound {
            resource: "block",
            id: id.to_string(),
        })?;

        update(block);
        Ok(block.clone())
    }

    /// Delete a standalone block
    pub fn delete_standalone_block(&self, id: &str) -> Result<(), StateError> {
        let mut blocks = self
            .inner
            .blocks
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if blocks.remove(id).is_none() {
            return Err(StateError::NotFound {
                resource: "block",
                id: id.to_string(),
            });
        }

        Ok(())
    }

    /// Get standalone block count
    pub fn standalone_block_count(&self) -> Result<usize, StateError> {
        let blocks = self
            .inner
            .blocks
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(blocks.len())
    }

    // =========================================================================
    // Message operations
    // =========================================================================

    /// Add a message to an agent's history
    pub fn add_message(&self, agent_id: &str, message: Message) -> Result<Message, StateError> {
        // DST: Check for fault injection on message write
        if self.should_inject_fault("message_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "message_write".to_string(),
            });
        }

        let mut messages = self
            .inner
            .messages
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent_messages = messages
            .get_mut(agent_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?;

        if agent_messages.len() >= MESSAGES_PER_AGENT_MAX {
            return Err(StateError::LimitExceeded {
                resource: "messages",
                limit: MESSAGES_PER_AGENT_MAX,
            });
        }

        let result = message.clone();
        agent_messages.push(message);
        Ok(result)
    }

    /// Add a message to an agent's history (async version with storage persistence)
    ///
    /// NOTE: For the actor-based flow, messages are stored in actor state via
    /// handle_message_full operation. This method is kept for direct storage writes.
    ///
    /// Single source of truth: Writes to storage only. HashMap cache removed.
    pub async fn add_message_async(
        &self,
        agent_id: &str,
        message: Message,
    ) -> Result<Message, StateError> {
        // DST: Check for fault injection on message write
        if self.should_inject_fault("message_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "message_write".to_string(),
            });
        }

        // Single source of truth: Write to storage only
        if let Some(storage) = &self.inner.storage {
            storage
                .append_message(agent_id, &message)
                .await
                .map_err(|e| StateError::StorageError {
                    message: format!("Failed to persist message: {}", e),
                })?;
        }

        // HashMap writes removed - storage is single source of truth

        Ok(message)
    }

    /// List messages for an agent with pagination
    pub fn list_messages(
        &self,
        agent_id: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<Vec<Message>, StateError> {
        // DST: Check for fault injection on message read
        if self.should_inject_fault("message_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "message_read".to_string(),
            });
        }

        let messages = self
            .inner
            .messages
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        // Return empty vec if agent has no messages yet (not an error)
        let agent_messages = match messages.get(agent_id) {
            Some(msgs) => msgs,
            None => return Ok(Vec::new()),
        };

        // If before is specified, find messages before that ID
        let end_idx = if let Some(before_id) = before {
            agent_messages
                .iter()
                .position(|m| m.id == before_id)
                .unwrap_or(agent_messages.len())
        } else {
            agent_messages.len()
        };

        let start_idx = end_idx.saturating_sub(limit);
        Ok(agent_messages[start_idx..end_idx].to_vec())
    }

    // =========================================================================
    // Tool operations
    // =========================================================================

    /// Register a custom tool
    pub async fn register_tool(
        &self,
        name: String,
        description: String,
        input_schema: serde_json::Value,
        source_code: String,
        runtime: String,
        requirements: Vec<String>,
    ) -> Result<ToolInfo, StateError> {
        if name.is_empty() {
            return Err(StateError::Internal {
                message: "tool name cannot be empty".to_string(),
            });
        }

        if source_code.trim().is_empty() {
            return Err(StateError::Internal {
                message: "source_code cannot be empty".to_string(),
            });
        }

        self.tool_registry()
            .register_custom_tool(
                name.clone(),
                description.clone(),
                input_schema.clone(),
                source_code.clone(),
                runtime.clone(),
                requirements.clone(),
            )
            .await;

        if let Some(storage) = &self.inner.storage {
            let now = chrono::Utc::now();
            let record = crate::storage::CustomToolRecord {
                name: name.clone(),
                description: description.clone(),
                input_schema: input_schema.clone(),
                source_code,
                runtime,
                requirements,
                created_at: now,
                updated_at: now,
            };
            storage
                .save_custom_tool(&record)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(ToolInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description,
            input_schema,
            source: Some("custom".to_string()),
            default_requires_approval: false,
            tool_type: "custom".to_string(),
            tags: None,
            return_char_limit: None,
        })
    }

    /// Upsert a tool (create or update)
    ///
    /// This is the primary method for tool registration, supporting both
    /// server-side and client-side tools.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_tool(
        &self,
        id: String,
        name: String,
        description: String,
        input_schema: serde_json::Value,
        source: Option<String>,
        default_requires_approval: bool,
        tool_type: String,
        tags: Option<Vec<String>>,
        return_char_limit: Option<u32>,
    ) -> Result<ToolInfo, StateError> {
        if name.is_empty() {
            return Err(StateError::Internal {
                message: "tool name cannot be empty".to_string(),
            });
        }

        // TigerStyle: Construct ToolInfo once to avoid duplication
        let tool_info = ToolInfo {
            id,
            name: name.clone(),
            description: description.clone(),
            input_schema: input_schema.clone(),
            source: source.clone(),
            default_requires_approval,
            tool_type: tool_type.clone(),
            tags,
            return_char_limit,
        };

        // For client-side tools, we store metadata but don't register executable code
        if tool_type == "client" || default_requires_approval {
            // Store in client tools map
            self.inner
                .client_tools
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(name, tool_info.clone());

            return Ok(tool_info);
        }

        // For server-side custom tools, register with the tool registry and persist to storage
        if let Some(source_code) = &source {
            self.tool_registry()
                .register_custom_tool(
                    tool_info.name.clone(),
                    description,
                    input_schema,
                    source_code.clone(),
                    "python".to_string(),
                    vec![],
                )
                .await;

            // Persist to durable storage (if configured)
            if let Some(storage) = &self.inner.storage {
                tracing::info!(name = %tool_info.name, "persisting custom tool to storage");
                let now = chrono::Utc::now();
                let record = crate::storage::CustomToolRecord {
                    name: tool_info.name.clone(),
                    description: tool_info.description.clone(),
                    input_schema: tool_info.input_schema.clone(),
                    source_code: source_code.clone(),
                    runtime: "python".to_string(),
                    requirements: vec![],
                    created_at: now,
                    updated_at: now,
                };
                match storage.save_custom_tool(&record).await {
                    Ok(_) => {
                        tracing::info!(name = %tool_info.name, "custom tool persisted successfully")
                    }
                    Err(e) => {
                        tracing::warn!(name = %tool_info.name, error = %e, "failed to persist custom tool to storage")
                    }
                }
            } else {
                tracing::debug!(name = %tool_info.name, "no storage configured, custom tool not persisted");
            }
        }

        // Store the tool info in client_tools map for ID-based lookup
        // This allows get_tool_by_id to work for all tool types
        self.inner
            .client_tools
            .write()
            .map_err(|_| StateError::LockPoisoned)?
            .insert(name, tool_info.clone());

        Ok(tool_info)
    }

    /// Get a tool by name
    pub async fn get_tool(&self, name: &str) -> Option<ToolInfo> {
        // First check client tools
        if let Ok(client_tools) = self.inner.client_tools.read() {
            if let Some(tool) = client_tools.get(name) {
                return Some(tool.clone());
            }
        }

        // Then check registered tools
        let tool = self.tool_registry().get_tool(name).await?;
        Some(ToolInfo {
            id: uuid::Uuid::new_v4().to_string(), // Generate ID for legacy tools
            name: tool.definition.name,
            description: tool.definition.description,
            input_schema: tool.definition.input_schema,
            source: Some(tool.source.to_string()),
            default_requires_approval: false,
            tool_type: tool.source.to_string(),
            tags: None,
            return_char_limit: None,
        })
    }

    /// Get a tool by ID
    pub async fn get_tool_by_id(&self, id: &str) -> Option<ToolInfo> {
        // Check client tools
        if let Ok(client_tools) = self.inner.client_tools.read() {
            for tool in client_tools.values() {
                if tool.id == id {
                    return Some(tool.clone());
                }
            }
        }

        // Check registered tools using deterministic IDs
        let tools = self.tool_registry().list_registered_tools().await;
        for tool in tools {
            let tool_id = Self::tool_name_to_uuid(&tool.definition.name).to_string();
            if tool_id == id {
                return Some(ToolInfo {
                    id: tool_id,
                    name: tool.definition.name,
                    description: tool.definition.description,
                    input_schema: tool.definition.input_schema,
                    source: Some(tool.source.to_string()),
                    default_requires_approval: false,
                    tool_type: tool.source.to_string(),
                    tags: None,
                    return_char_limit: None,
                });
            }
        }

        None
    }

    /// List all tools
    pub async fn list_tools(&self) -> Vec<ToolInfo> {
        let mut result = Vec::new();

        // Add client tools
        if let Ok(client_tools) = self.inner.client_tools.read() {
            result.extend(client_tools.values().cloned());
        }

        // Add registered tools
        let tools = self.tool_registry().list_registered_tools().await;
        for tool in tools {
            // Skip if already in client tools
            if result.iter().any(|t| t.name == tool.definition.name) {
                continue;
            }
            result.push(ToolInfo {
                // Use deterministic UUID based on tool name for stable pagination
                id: Self::tool_name_to_uuid(&tool.definition.name).to_string(),
                name: tool.definition.name,
                description: tool.definition.description,
                input_schema: tool.definition.input_schema,
                source: Some(tool.source.to_string()),
                default_requires_approval: false,
                tool_type: tool.source.to_string(),
                tags: None,
                return_char_limit: None,
            });
        }

        result
    }

    /// Generate a deterministic UUID from a tool name
    /// Uses UUID v5 (name-based with SHA-1) to ensure the same name always produces the same ID
    fn tool_name_to_uuid(name: &str) -> uuid::Uuid {
        // Use the DNS namespace as a standard namespace
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes())
    }

    /// Delete a tool
    pub async fn delete_tool(&self, name: &str) -> Result<(), StateError> {
        // Try to remove from tool registry
        let removed_from_registry = self.tool_registry().unregister(name).await;

        // Try to remove from client tools
        let removed_from_client = self
            .inner
            .client_tools
            .write()
            .map_err(|_| StateError::LockPoisoned)?
            .remove(name)
            .is_some();

        // Tool must exist in at least one location
        if !removed_from_registry && !removed_from_client {
            return Err(StateError::NotFound {
                resource: "tool",
                id: name.to_string(),
            });
        }

        // Remove from persistent storage if configured
        if let Some(storage) = &self.inner.storage {
            let _ = storage.delete_custom_tool(name).await;
        }

        Ok(())
    }

    /// Execute a tool via the unified registry
    pub async fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, StateError> {
        let result = self.tool_registry().execute(name, &arguments).await;
        if result.success {
            Ok(result.output)
        } else {
            Err(StateError::Internal {
                message: result.error.unwrap_or(result.output),
            })
        }
    }

    /// Load custom tools from storage into the registry
    pub async fn load_custom_tools(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            tracing::debug!("no storage configured, skipping custom tool loading");
            return Ok(());
        };

        tracing::info!("loading custom tools from storage...");
        let tools = storage
            .list_custom_tools()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        tracing::info!(count = tools.len(), "found custom tools in storage");

        for tool in tools {
            tracing::info!(name = %tool.name, "loading custom tool from storage");
            self.tool_registry()
                .register_custom_tool(
                    tool.name,
                    tool.description,
                    tool.input_schema,
                    tool.source_code,
                    tool.runtime,
                    tool.requirements,
                )
                .await;
        }

        Ok(())
    }

    /// Load agents from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted agents.
    pub async fn load_agents_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let agents = storage
            .list_agents()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = agents.len();
        for metadata in agents {
            // Load full agent state from storage
            if let Ok(Some(agent)) = self.load_agent_from_storage(&metadata.id).await {
                // Insert into in-memory state
                self.inner
                    .agents
                    .write()
                    .map_err(|_| StateError::LockPoisoned)?
                    .insert(agent.id.clone(), agent);
            }
        }

        tracing::info!(count = count, "loaded agents from storage");
        Ok(())
    }

    /// Load MCP servers from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted MCP servers.
    pub async fn load_mcp_servers_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let servers = storage
            .list_mcp_servers()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = servers.len();
        for server in servers {
            self.inner
                .mcp_servers
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(server.id.clone(), server);
        }

        tracing::info!(count = count, "loaded MCP servers from storage");
        Ok(())
    }

    /// Load agent groups from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted agent groups.
    pub async fn load_agent_groups_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let groups = storage
            .list_agent_groups()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = groups.len();
        for group in groups {
            self.inner
                .agent_groups
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(group.id.clone(), group);
        }

        tracing::info!(count = count, "loaded agent groups from storage");
        Ok(())
    }

    /// Load identities from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted identities.
    pub async fn load_identities_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let identities = storage
            .list_identities()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = identities.len();
        for identity in identities {
            self.inner
                .identities
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(identity.id.clone(), identity);
        }

        tracing::info!(count = count, "loaded identities from storage");
        Ok(())
    }

    /// Load projects from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted projects.
    pub async fn load_projects_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let projects = storage
            .list_projects()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = projects.len();
        for project in projects {
            self.inner
                .projects
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(project.id.clone(), project);
        }

        tracing::info!(count = count, "loaded projects from storage");
        Ok(())
    }

    /// Load jobs from storage into the in-memory state
    ///
    /// Called on server startup to restore persisted jobs.
    pub async fn load_jobs_from_storage(&self) -> Result<(), StateError> {
        let Some(storage) = &self.inner.storage else {
            return Ok(());
        };

        let jobs = storage
            .list_jobs()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("storage error: {}", e),
            })?;

        let count = jobs.len();
        for job in jobs {
            self.inner
                .jobs
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(job.id.clone(), job);
        }

        tracing::info!(count = count, "loaded jobs from storage");
        Ok(())
    }

    // =========================================================================
    // Archival memory operations
    // =========================================================================

    /// Add entry to archival memory
    pub fn add_archival(
        &self,
        agent_id: &str,
        content: String,
        metadata: Option<serde_json::Value>,
    ) -> Result<ArchivalEntry, StateError> {
        // DST: Check for fault injection on archival write
        if self.should_inject_fault("archival_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "archival_write".to_string(),
            });
        }

        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        // Auto-create archival storage for agent if it doesn't exist yet
        let entries = archival
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new);

        if entries.len() >= ARCHIVAL_ENTRIES_PER_AGENT_MAX {
            return Err(StateError::LimitExceeded {
                resource: "archival_entries",
                limit: ARCHIVAL_ENTRIES_PER_AGENT_MAX,
            });
        }

        let entry = ArchivalEntry {
            id: Uuid::new_v4().to_string(),
            content,
            metadata,
            created_at: Utc::now().to_rfc3339(),
        };

        let result = entry.clone();
        entries.push(entry);
        Ok(result)
    }

    /// Search archival memory
    pub fn search_archival(
        &self,
        agent_id: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ArchivalEntry>, StateError> {
        // DST: Check for fault injection on archival read
        if self.should_inject_fault("archival_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "archival_read".to_string(),
            });
        }

        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        // Return empty list if agent has no archival entries yet (not an error)
        let entries = match archival.get(agent_id) {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        // Simple text search if query is provided
        let results: Vec<_> = if let Some(q) = query {
            let q_lower = q.to_lowercase();
            entries
                .iter()
                .filter(|e| e.content.to_lowercase().contains(&q_lower))
                .take(limit)
                .cloned()
                .collect()
        } else {
            entries.iter().take(limit).cloned().collect()
        };

        Ok(results)
    }

    /// Get a specific archival entry
    pub fn get_archival_entry(
        &self,
        agent_id: &str,
        entry_id: &str,
    ) -> Result<Option<ArchivalEntry>, StateError> {
        let archival = self
            .inner
            .archival
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let entries = archival.get(agent_id).ok_or_else(|| StateError::NotFound {
            resource: "agent",
            id: agent_id.to_string(),
        })?;

        Ok(entries.iter().find(|e| e.id == entry_id).cloned())
    }

    /// Delete an archival entry
    pub fn delete_archival_entry(&self, agent_id: &str, entry_id: &str) -> Result<(), StateError> {
        let mut archival = self
            .inner
            .archival
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        let entries = archival
            .get_mut(agent_id)
            .ok_or_else(|| StateError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?;

        let initial_len = entries.len();
        entries.retain(|e| e.id != entry_id);

        if entries.len() == initial_len {
            return Err(StateError::NotFound {
                resource: "archival_entry",
                id: entry_id.to_string(),
            });
        }

        Ok(())
    }

    // =========================================================================
    // Job operations (Phase 5: Scheduling)
    // =========================================================================

    /// Add a scheduled job
    pub fn add_job(&self, job: Job) -> Result<(), StateError> {
        if self.should_inject_fault("job_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_write".to_string(),
            });
        }

        let mut jobs = self
            .inner
            .jobs
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if jobs.contains_key(&job.id) {
            return Err(StateError::AlreadyExists {
                resource: "job",
                id: job.id.clone(),
            });
        }

        jobs.insert(job.id.clone(), job);
        Ok(())
    }

    /// Get a job by ID
    pub fn get_job(&self, job_id: &str) -> Result<Option<Job>, StateError> {
        if self.should_inject_fault("job_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_read".to_string(),
            });
        }

        let jobs = self
            .inner
            .jobs
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(jobs.get(job_id).cloned())
    }

    /// List jobs for a specific agent
    pub fn list_jobs_for_agent(&self, agent_id: &str) -> Result<Vec<Job>, StateError> {
        if self.should_inject_fault("job_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_read".to_string(),
            });
        }

        let jobs = self
            .inner
            .jobs
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let agent_jobs: Vec<_> = jobs
            .values()
            .filter(|j| j.agent_id == agent_id)
            .cloned()
            .collect();

        Ok(agent_jobs)
    }

    /// List all jobs with optional agent filter
    pub fn list_all_jobs(&self, agent_id: Option<&str>) -> Result<Vec<Job>, StateError> {
        if self.should_inject_fault("job_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_read".to_string(),
            });
        }

        let jobs = self
            .inner
            .jobs
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let filtered_jobs: Vec<_> = if let Some(agent_id) = agent_id {
            jobs.values()
                .filter(|j| j.agent_id == agent_id)
                .cloned()
                .collect()
        } else {
            jobs.values().cloned().collect()
        };

        Ok(filtered_jobs)
    }

    /// Update a job
    pub fn update_job(&self, job: Job) -> Result<(), StateError> {
        if self.should_inject_fault("job_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_write".to_string(),
            });
        }

        let mut jobs = self
            .inner
            .jobs
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if !jobs.contains_key(&job.id) {
            return Err(StateError::NotFound {
                resource: "job",
                id: job.id.clone(),
            });
        }

        jobs.insert(job.id.clone(), job);
        Ok(())
    }

    /// Delete a job
    pub fn delete_job(&self, job_id: &str) -> Result<(), StateError> {
        if self.should_inject_fault("job_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "job_write".to_string(),
            });
        }

        let mut jobs = self
            .inner
            .jobs
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if jobs.remove(job_id).is_none() {
            return Err(StateError::NotFound {
                resource: "job",
                id: job_id.to_string(),
            });
        }

        Ok(())
    }

    // =========================================================================
    // Project operations (Phase 6: Projects)
    // =========================================================================

    /// Add a project
    pub fn add_project(&self, project: Project) -> Result<(), StateError> {
        if self.should_inject_fault("project_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_write".to_string(),
            });
        }

        let mut projects = self
            .inner
            .projects
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if projects.contains_key(&project.id) {
            return Err(StateError::AlreadyExists {
                resource: "project",
                id: project.id.clone(),
            });
        }

        projects.insert(project.id.clone(), project);
        Ok(())
    }

    /// Get a project by ID
    pub fn get_project(&self, project_id: &str) -> Result<Option<Project>, StateError> {
        if self.should_inject_fault("project_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_read".to_string(),
            });
        }

        let projects = self
            .inner
            .projects
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(projects.get(project_id).cloned())
    }

    /// List all projects with pagination
    pub fn list_projects(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<Project>, Option<String>), StateError> {
        if self.should_inject_fault("project_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_read".to_string(),
            });
        }

        let projects = self
            .inner
            .projects
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let mut all_projects: Vec<_> = projects.values().cloned().collect();
        all_projects.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        // Apply cursor
        let start_idx = if let Some(cursor_id) = cursor {
            all_projects
                .iter()
                .position(|p| p.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let remaining: Vec<_> = all_projects.into_iter().skip(start_idx).collect();

        // Return all remaining items and last item ID as next cursor
        let next_cursor = remaining.last().map(|p| p.id.clone());

        Ok((remaining, next_cursor))
    }

    /// Update a project
    pub fn update_project(&self, project: Project) -> Result<(), StateError> {
        if self.should_inject_fault("project_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_write".to_string(),
            });
        }

        let mut projects = self
            .inner
            .projects
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if !projects.contains_key(&project.id) {
            return Err(StateError::NotFound {
                resource: "project",
                id: project.id.clone(),
            });
        }

        projects.insert(project.id.clone(), project);
        Ok(())
    }

    /// Delete a project
    pub fn delete_project(&self, project_id: &str) -> Result<(), StateError> {
        if self.should_inject_fault("project_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_write".to_string(),
            });
        }

        let mut projects = self
            .inner
            .projects
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if projects.remove(project_id).is_none() {
            return Err(StateError::NotFound {
                resource: "project",
                id: project_id.to_string(),
            });
        }

        Ok(())
    }

    /// List agents by project ID
    pub fn list_agents_by_project(&self, project_id: &str) -> Result<Vec<AgentState>, StateError> {
        if self.should_inject_fault("project_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "project_read".to_string(),
            });
        }

        let agents = self
            .inner
            .agents
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let project_agents: Vec<_> = agents
            .values()
            .filter(|a| a.project_id.as_deref() == Some(project_id))
            .cloned()
            .collect();

        Ok(project_agents)
    }

    // =========================================================================
    // Batch Operations (Phase 7)
    // =========================================================================

    /// Store a batch status
    pub fn add_batch_status(&self, status: BatchStatus) -> Result<(), StateError> {
        if self.should_inject_fault("batch_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "batch_write".to_string(),
            });
        }

        let mut batches = self
            .inner
            .batches
            .write()
            .map_err(|_| StateError::LockPoisoned)?;
        batches.insert(status.id.clone(), status);
        Ok(())
    }

    /// Update a batch status
    pub fn update_batch_status(&self, status: BatchStatus) -> Result<(), StateError> {
        if self.should_inject_fault("batch_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "batch_write".to_string(),
            });
        }

        let mut batches = self
            .inner
            .batches
            .write()
            .map_err(|_| StateError::LockPoisoned)?;

        if !batches.contains_key(&status.id) {
            return Err(StateError::NotFound {
                resource: "batch",
                id: status.id.clone(),
            });
        }

        batches.insert(status.id.clone(), status);
        Ok(())
    }

    /// Get batch status by ID
    pub fn get_batch_status(&self, batch_id: &str) -> Result<Option<BatchStatus>, StateError> {
        if self.should_inject_fault("batch_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "batch_read".to_string(),
            });
        }

        let batches = self
            .inner
            .batches
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(batches.get(batch_id).cloned())
    }

    // =========================================================================
    // Agent Group Operations (Phase 8)
    // =========================================================================

    /// Add a new agent group
    pub async fn add_agent_group(&self, group: AgentGroup) -> Result<(), StateError> {
        if self.should_inject_fault("agent_group_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_group_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut groups = self
                .inner
                .agent_groups
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if groups.contains_key(&group.id) {
                return Err(StateError::AlreadyExists {
                    resource: "agent_group",
                    id: group.id.clone(),
                });
            }

            groups.insert(group.id.clone(), group.clone());
        } // Lock dropped here

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_agent_group(&group)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }

    /// Get agent group by ID
    pub fn get_agent_group(&self, group_id: &str) -> Result<Option<AgentGroup>, StateError> {
        if self.should_inject_fault("agent_group_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_group_read".to_string(),
            });
        }

        let groups = self
            .inner
            .agent_groups
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(groups.get(group_id).cloned())
    }

    /// List agent groups with pagination
    pub fn list_agent_groups(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<AgentGroup>, Option<String>), StateError> {
        if self.should_inject_fault("agent_group_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_group_read".to_string(),
            });
        }

        let groups = self
            .inner
            .agent_groups
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let mut all_groups: Vec<_> = groups.values().cloned().collect();
        all_groups.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let start_idx = if let Some(cursor_id) = cursor {
            all_groups
                .iter()
                .position(|g| g.id == cursor_id)
                .map(|i| i + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let remaining: Vec<_> = all_groups.into_iter().skip(start_idx).collect();
        let next_cursor = remaining.last().map(|g| g.id.clone());

        Ok((remaining, next_cursor))
    }

    /// Update an agent group
    pub async fn update_agent_group(&self, group: AgentGroup) -> Result<(), StateError> {
        if self.should_inject_fault("agent_group_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_group_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut groups = self
                .inner
                .agent_groups
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if !groups.contains_key(&group.id) {
                return Err(StateError::NotFound {
                    resource: "agent_group",
                    id: group.id.clone(),
                });
            }

            groups.insert(group.id.clone(), group.clone());
        } // Lock dropped here

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_agent_group(&group)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }

    /// Delete an agent group
    pub async fn delete_agent_group(&self, group_id: &str) -> Result<(), StateError> {
        if self.should_inject_fault("agent_group_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "agent_group_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut groups = self
                .inner
                .agent_groups
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if groups.remove(group_id).is_none() {
                return Err(StateError::NotFound {
                    resource: "agent_group",
                    id: group_id.to_string(),
                });
            }
        } // Lock dropped here

        // Persist deletion to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .delete_agent_group(group_id)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }

    // =========================================================================
    // Identities
    // =========================================================================

    /// Add a new identity
    pub async fn add_identity(&self, identity: crate::models::Identity) -> Result<(), StateError> {
        if self.should_inject_fault("identity_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "identity_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut identities = self
                .inner
                .identities
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if identities.contains_key(&identity.id) {
                return Err(StateError::AlreadyExists {
                    resource: "identity",
                    id: identity.id.clone(),
                });
            }

            identities.insert(identity.id.clone(), identity.clone());
        } // Lock dropped here

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_identity(&identity)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }

    /// Get identity by ID
    pub fn get_identity(
        &self,
        identity_id: &str,
    ) -> Result<Option<crate::models::Identity>, StateError> {
        if self.should_inject_fault("identity_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "identity_read".to_string(),
            });
        }

        let identities = self
            .inner
            .identities
            .read()
            .map_err(|_| StateError::LockPoisoned)?;
        Ok(identities.get(identity_id).cloned())
    }

    /// List identities with pagination
    pub fn list_identities(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<crate::models::Identity>, Option<String>), StateError> {
        if self.should_inject_fault("identity_read").is_some() {
            return Err(StateError::FaultInjected {
                operation: "identity_read".to_string(),
            });
        }

        let identities = self
            .inner
            .identities
            .read()
            .map_err(|_| StateError::LockPoisoned)?;

        let mut all_identities: Vec<_> = identities.values().cloned().collect();
        all_identities.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let start_idx = if let Some(cursor_id) = cursor {
            all_identities
                .iter()
                .position(|i| i.id == cursor_id)
                .map(|idx| idx + 1)
                .unwrap_or(0)
        } else {
            0
        };

        let remaining: Vec<_> = all_identities.into_iter().skip(start_idx).collect();
        let next_cursor = remaining.last().map(|i| i.id.clone());

        Ok((remaining, next_cursor))
    }

    /// Update an identity
    pub async fn update_identity(
        &self,
        identity: crate::models::Identity,
    ) -> Result<(), StateError> {
        if self.should_inject_fault("identity_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "identity_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut identities = self
                .inner
                .identities
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if !identities.contains_key(&identity.id) {
                return Err(StateError::NotFound {
                    resource: "identity",
                    id: identity.id.clone(),
                });
            }

            identities.insert(identity.id.clone(), identity.clone());
        } // Lock dropped here

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_identity(&identity)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }

    /// Delete an identity
    pub async fn delete_identity(&self, identity_id: &str) -> Result<(), StateError> {
        if self.should_inject_fault("identity_write").is_some() {
            return Err(StateError::FaultInjected {
                operation: "identity_write".to_string(),
            });
        }

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut identities = self
                .inner
                .identities
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            if identities.remove(identity_id).is_none() {
                return Err(StateError::NotFound {
                    resource: "identity",
                    id: identity_id.to_string(),
                });
            }
        } // Lock dropped here

        // Persist deletion to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .delete_identity(identity_id)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        Ok(())
    }
}

impl Default for AppState<kelpie_core::TokioRuntime> {
    fn default() -> Self {
        Self::new(kelpie_core::TokioRuntime)
    }
}

/// State operation errors
#[derive(Debug, Clone)]
pub enum StateError {
    /// Resource not found
    NotFound { resource: &'static str, id: String },
    /// Resource already exists
    AlreadyExists { resource: &'static str, id: String },
    /// Limit exceeded
    LimitExceeded {
        resource: &'static str,
        limit: usize,
    },
    /// Lock poisoned (shouldn't happen in practice)
    LockPoisoned,
    /// Fault injected (DST testing only)
    FaultInjected { operation: String },
    /// Internal error (service errors, etc.)
    Internal { message: String },
    /// Storage error (from AgentStorage operations)
    StorageError { message: String },
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::NotFound { resource, id } => {
                write!(f, "{} with id '{}' not found", resource, id)
            }
            StateError::AlreadyExists { resource, id } => {
                write!(f, "{} with id '{}' already exists", resource, id)
            }
            StateError::LimitExceeded { resource, limit } => {
                write!(f, "{} limit ({}) exceeded", resource, limit)
            }
            StateError::LockPoisoned => write!(f, "internal lock error"),
            StateError::FaultInjected { operation } => {
                write!(f, "fault injected during operation: {}", operation)
            }
            StateError::Internal { message } => {
                write!(f, "internal error: {}", message)
            }
            StateError::StorageError { message } => {
                write!(f, "storage error: {}", message)
            }
        }
    }
}

impl std::error::Error for StateError {}

// =============================================================================
// MCP Server Management (Letta Compatibility)
// =============================================================================

impl<R: kelpie_core::Runtime> AppState<R> {
    /// Create a new MCP server
    pub async fn create_mcp_server(
        &self,
        server_name: impl Into<String>,
        config: crate::models::MCPServerConfig,
    ) -> Result<crate::models::MCPServer, StateError> {
        let server = crate::models::MCPServer::new(server_name, config);

        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let server_id = server.id.clone();
            self.inner
                .mcp_servers
                .write()
                .map_err(|_| StateError::LockPoisoned)?
                .insert(server_id.clone(), server.clone());
        } // Lock dropped here

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_mcp_server(&server)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        tracing::debug!(server_id = %server.id, "Created MCP server");
        Ok(server)
    }

    /// Get an MCP server by ID
    pub async fn get_mcp_server(&self, server_id: &str) -> Option<crate::models::MCPServer> {
        self.inner.mcp_servers.read().ok()?.get(server_id).cloned()
    }

    /// List all MCP servers
    pub async fn list_mcp_servers(&self) -> Vec<crate::models::MCPServer> {
        self.inner
            .mcp_servers
            .read()
            .map(|servers| servers.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Update an MCP server
    pub async fn update_mcp_server(
        &self,
        server_id: &str,
        server_name: Option<String>,
        config: Option<crate::models::MCPServerConfig>,
    ) -> Result<crate::models::MCPServer, StateError> {
        let updated_server = {
            let mut servers = self
                .inner
                .mcp_servers
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            let server = servers
                .get_mut(server_id)
                .ok_or_else(|| StateError::NotFound {
                    resource: "MCP server",
                    id: server_id.to_string(),
                })?;

            if let Some(name) = server_name {
                server.server_name = name;
            }
            if let Some(cfg) = config {
                server.config = cfg;
            }
            server.updated_at = chrono::Utc::now();

            server.clone()
        };

        // Persist to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .save_mcp_server(&updated_server)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        tracing::debug!(server_id = %server_id, "Updated MCP server");
        Ok(updated_server)
    }

    /// Delete an MCP server
    pub async fn delete_mcp_server(&self, server_id: &str) -> Result<(), StateError> {
        // Update in-memory state (lock scope ensures guard is dropped before await)
        {
            let mut servers = self
                .inner
                .mcp_servers
                .write()
                .map_err(|_| StateError::LockPoisoned)?;

            servers
                .remove(server_id)
                .ok_or_else(|| StateError::NotFound {
                    resource: "MCP server",
                    id: server_id.to_string(),
                })?;
        } // Lock dropped here

        // Persist deletion to storage if configured
        if let Some(storage) = &self.inner.storage {
            storage
                .delete_mcp_server(server_id)
                .await
                .map_err(|e| StateError::Internal {
                    message: format!("storage error: {}", e),
                })?;
        }

        tracing::debug!(server_id = %server_id, "Deleted MCP server");
        Ok(())
    }

    // =========================================================================
    // MCP Config Helpers (DRY - reduce cognitive complexity)
    // =========================================================================

    /// Convert MCPServerConfig to McpConfig
    ///
    /// TigerStyle: Single responsibility - converts between config types.
    /// Reduces nesting depth by extracting repeated match logic.
    /// Public to allow reuse in mcp_servers API module.
    pub fn mcp_server_config_to_mcp_config(
        server_name: &str,
        config: &crate::models::MCPServerConfig,
    ) -> kelpie_tools::mcp::McpConfig {
        use kelpie_tools::mcp::McpConfig;

        match config {
            crate::models::MCPServerConfig::Stdio { command, args, env } => {
                let mut mcp_config = McpConfig::stdio(server_name, command, args.clone());
                if let Some(env_map) = env {
                    for (k, v) in env_map {
                        if let Some(v_str) = v.as_str() {
                            mcp_config = mcp_config.with_env(k.clone(), v_str.to_string());
                        }
                    }
                }
                mcp_config
            }
            crate::models::MCPServerConfig::Sse { server_url, .. } => {
                McpConfig::sse(server_name, server_url)
            }
            crate::models::MCPServerConfig::StreamableHttp { server_url, .. } => {
                McpConfig::http(server_name, server_url)
            }
        }
    }

    /// List tools provided by an MCP server
    ///
    /// Returns JSON Value array to avoid type conflicts from multiple compilations
    pub async fn list_mcp_server_tools(
        &self,
        server_id: &str,
    ) -> Result<Vec<serde_json::Value>, StateError> {
        use kelpie_tools::mcp::McpClient;
        use std::sync::Arc;

        // Get the MCP server
        let server = self
            .get_mcp_server(server_id)
            .await
            .ok_or_else(|| StateError::NotFound {
                resource: "MCP server",
                id: server_id.to_string(),
            })?;

        // Convert MCPServerConfig to McpConfig using helper
        let mcp_config = Self::mcp_server_config_to_mcp_config(&server.server_name, &server.config);

        // Create MCP client
        let client = Arc::new(McpClient::new(mcp_config));

        // Connect to the server
        client.connect().await.map_err(|e| StateError::Internal {
            message: format!("Failed to connect to MCP server: {}", e),
        })?;

        // Discover tools
        let tools = client
            .discover_tools()
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Failed to discover tools from MCP server: {}", e),
            })?;

        // Disconnect
        let _ = client.disconnect().await;

        // Convert McpToolDefinition to JSON Value to avoid type conflicts
        let tool_responses: Vec<serde_json::Value> = tools
            .into_iter()
            .map(|tool| {
                serde_json::json!({
                    "id": format!("mcp_{}_{}", server_id, tool.name),
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                    "source": serde_json::Value::Null,
                    "default_requires_approval": false,
                    "tool_type": "mcp",
                })
            })
            .collect();

        Ok(tool_responses)
    }

    /// Execute a tool on an MCP server
    pub async fn execute_mcp_server_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, StateError> {
        use kelpie_tools::mcp::McpClient;
        use std::sync::Arc;

        // Get the MCP server
        let server = self
            .get_mcp_server(server_id)
            .await
            .ok_or_else(|| StateError::NotFound {
                resource: "MCP server",
                id: server_id.to_string(),
            })?;

        // Convert MCPServerConfig to McpConfig using helper
        let mcp_config = Self::mcp_server_config_to_mcp_config(&server.server_name, &server.config);

        // Create MCP client
        let client = Arc::new(McpClient::new(mcp_config));

        // Connect to the server
        client.connect().await.map_err(|e| StateError::Internal {
            message: format!("Failed to connect to MCP server: {}", e),
        })?;

        // Execute tool
        let result = client
            .execute_tool(tool_name, arguments)
            .await
            .map_err(|e| StateError::Internal {
                message: format!("Failed to execute MCP tool: {}", e),
            })?;

        // Disconnect
        let _ = client.disconnect().await;

        Ok(result)
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::models::{AgentType, CreateAgentRequest, CreateBlockRequest};

    fn create_test_agent(name: &str) -> AgentState {
        AgentState::from_request(CreateAgentRequest {
            name: name.to_string(),
            agent_type: AgentType::default(),
            model: None,
            embedding: None,
            system: None,
            description: None,
            project_id: None,
            user_id: None,
            org_id: None,
            memory_blocks: vec![CreateBlockRequest {
                label: "persona".to_string(),
                value: "I am a test agent".to_string(),
                description: None,
                limit: None,
            }],
            block_ids: vec![],
            tool_ids: vec![],
            tags: vec![],
            metadata: serde_json::json!({}),
        })
    }

    #[test]
    fn test_create_and_get_agent() {
        let state = AppState::new(kelpie_core::TokioRuntime);
        let agent = create_test_agent("test-agent");
        let agent_id = agent.id.clone();

        let created = state.create_agent(agent).unwrap();
        assert_eq!(created.name, "test-agent");

        let retrieved = state.get_agent(&agent_id).unwrap().unwrap();
        assert_eq!(retrieved.id, agent_id);
        assert_eq!(retrieved.blocks.len(), 1);
    }

    #[test]
    fn test_list_agents_pagination() {
        let state = AppState::new(kelpie_core::TokioRuntime);

        for i in 0..5 {
            let agent = create_test_agent(&format!("agent-{}", i));
            state.create_agent(agent).unwrap();
        }

        // First page
        let (page1, cursor1) = state.list_agents(2, None).unwrap();
        assert_eq!(page1.len(), 2);
        assert!(cursor1.is_some());

        // Second page
        let (page2, cursor2) = state.list_agents(2, cursor1.as_deref()).unwrap();
        assert_eq!(page2.len(), 2);
        assert!(cursor2.is_some());

        // Third page (last)
        let (page3, cursor3) = state.list_agents(2, cursor2.as_deref()).unwrap();
        assert_eq!(page3.len(), 1);
        assert!(cursor3.is_none());
    }

    #[test]
    fn test_delete_agent() {
        let state = AppState::new(kelpie_core::TokioRuntime);
        let agent = create_test_agent("to-delete");
        let agent_id = agent.id.clone();

        state.create_agent(agent).unwrap();
        assert!(state.get_agent(&agent_id).unwrap().is_some());

        state.delete_agent(&agent_id).unwrap();
        assert!(state.get_agent(&agent_id).unwrap().is_none());
    }

    #[test]
    fn test_update_block() {
        let state = AppState::new(kelpie_core::TokioRuntime);
        let agent = create_test_agent("block-test");
        let agent_id = agent.id.clone();
        let block_id = agent.blocks[0].id.clone();

        state.create_agent(agent).unwrap();

        let updated = state
            .update_block(&agent_id, &block_id, |block| {
                block.value = "Updated value".to_string();
            })
            .unwrap();

        assert_eq!(updated.value, "Updated value");
    }

    #[test]
    fn test_messages() {
        let state = AppState::new(kelpie_core::TokioRuntime);
        let agent = create_test_agent("msg-test");
        let agent_id = agent.id.clone();

        state.create_agent(agent).unwrap();

        // Add messages
        for i in 0..5 {
            let msg = Message {
                id: format!("msg-{}", i),
                agent_id: agent_id.clone(),
                message_type: "user_message".to_string(),
                role: crate::models::MessageRole::User,
                content: format!("Message {}", i),
                tool_call_id: None,
                tool_calls: vec![],
                tool_call: None,
                tool_return: None,
                status: None,
                created_at: chrono::Utc::now(),
            };
            state.add_message(&agent_id, msg).unwrap();
        }

        // List last 3
        let messages = state.list_messages(&agent_id, 3, None).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "Message 2");
        assert_eq!(messages[2].content, "Message 4");
    }

    #[tokio::test]
    async fn test_async_methods_require_agent_service() {
        // Test that async methods return error when AgentService is not configured
        let state = AppState::new(kelpie_core::TokioRuntime);

        // get_agent_async should error without service
        let result = state.get_agent_async("any-id").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("AgentService not configured"));

        // create_agent_async should error without service
        let request = crate::models::CreateAgentRequest {
            name: "test".to_string(),
            agent_type: crate::models::AgentType::default(),
            model: None,
            embedding: None,
            system: None,
            description: None,
            memory_blocks: vec![],
            block_ids: vec![],
            tool_ids: vec![],
            tags: vec![],
            metadata: serde_json::json!({}),
            project_id: None,
            user_id: None,
            org_id: None,
        };
        let result = state.create_agent_async(request).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("AgentService not configured"));

        // delete_agent_async should error without service
        let result = state.delete_agent_async("any-id").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("AgentService not configured"));
    }
}
