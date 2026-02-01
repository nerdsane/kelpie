//! Teleport service for agent migration between hosts
//!
//! TigerStyle: Service wraps storage, provides clean API, handles errors.
//!
//! This service provides high-level teleportation operations:
//! - teleport_out: Snapshot agent state and upload to storage
//! - teleport_in: Download from storage and restore agent state
//!
//! DST Support: Works with SimTeleportStorage for fault injection testing.

use crate::actor::{AgentActorState, RegisterRequest};
use crate::storage::{AgentMetadata, Architecture, SnapshotKind, TeleportPackage, TeleportStorage};
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_core::io::{TimeProvider, WallClockTime};
use kelpie_core::{Error, Result};
use kelpie_runtime::DispatcherHandle;
use kelpie_vm::{VmConfig, VmFactory, VmInstance, VmSnapshot, VmSnapshotMetadata};
use std::sync::Arc;

/// Teleport service for agent migration
///
/// TigerStyle: Clean abstraction, explicit error handling, testable.
#[derive(Clone)]
pub struct TeleportService<S, F>
where
    S: TeleportStorage,
    F: VmFactory,
{
    /// Teleport storage backend
    storage: Arc<S>,
    /// VM factory for creating VM instances
    vm_factory: Arc<F>,
    /// Optional dispatcher for RegistryActor registration
    /// If None, registration is skipped (backward compatible)
    dispatcher: Option<DispatcherHandle<kelpie_core::TokioRuntime>>,
    /// Expected base image version
    base_image_version: String,
}

impl<S, F> TeleportService<S, F>
where
    S: TeleportStorage,
    F: VmFactory,
{
    /// Create a new TeleportService without dispatcher (backward compatible)
    pub fn new(storage: Arc<S>, vm_factory: Arc<F>) -> Self {
        Self {
            storage,
            vm_factory,
            dispatcher: None,
            base_image_version: "1.0.0".to_string(),
        }
    }

    /// Create TeleportService with dispatcher for RegistryActor registration
    pub fn with_dispatcher(
        mut self,
        dispatcher: DispatcherHandle<kelpie_core::TokioRuntime>,
    ) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Set the expected base image version
    pub fn with_base_image_version(mut self, version: impl Into<String>) -> Self {
        self.base_image_version = version.into();
        self
    }

    /// Teleport agent OUT (snapshot + upload to storage)
    ///
    /// # Arguments
    /// * `agent_id` - Agent ID to teleport
    /// * `vm` - Running VM to snapshot
    /// * `agent_state` - Serialized agent state (memory blocks, conversation, etc.)
    /// * `kind` - Type of snapshot (Suspend, Teleport, or Checkpoint)
    ///
    /// # Returns
    /// The teleport package ID on success
    ///
    /// # Errors
    /// - SnapshotCreateFail: Failed to create snapshot
    /// - UploadFailed: Failed to upload package to storage
    ///
    /// # TigerStyle
    /// - Returns package ID for tracking
    /// - VM state is preserved on failure (no partial mutations)
    pub async fn teleport_out(
        &self,
        agent_id: &str,
        vm: &mut dyn VmInstance,
        agent_state: Bytes,
        kind: SnapshotKind,
    ) -> Result<String> {
        // Preconditions
        assert!(!agent_id.is_empty(), "agent_id must not be empty");

        tracing::info!(agent_id = %agent_id, kind = ?kind, "Teleporting agent OUT");

        // Step 1: Create snapshot of VM
        let snapshot = if kind == SnapshotKind::Checkpoint {
            None
        } else {
            Some(vm.snapshot().await.map_err(|e| Error::Internal {
                message: format!("Failed to create snapshot: {}", e),
            })?)
        };

        // Step 2: Build teleport package
        let package_id = format!("teleport-{}-{}", agent_id, uuid::Uuid::new_v4());
        let now_ms = WallClockTime::new().now_ms();

        let mut package =
            TeleportPackage::new(package_id.clone(), agent_id, self.storage.host_arch(), kind)
                .with_agent_state(agent_state)
                .with_created_at(now_ms)
                .with_base_image_version(&self.base_image_version);

        // Add VM state for full teleport
        if let Some(snapshot) = snapshot {
            let metadata_bytes =
                serde_json::to_vec(&snapshot.metadata).map_err(|e| Error::Internal {
                    message: format!("Failed to serialize snapshot metadata: {}", e),
                })?;
            let vm_snapshot = kelpie_core::teleport::VmSnapshotBlob::encode(
                Bytes::from(metadata_bytes),
                snapshot.data.clone(),
                Bytes::new(),
            );
            package = package.with_vm_snapshot(vm_snapshot);
        }

        // Step 3: Upload to storage
        let uploaded_id = self
            .storage
            .upload(package)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to upload teleport package: {}", e),
            })?;

        tracing::info!(
            agent_id = %agent_id,
            package_id = %uploaded_id,
            "Agent teleported OUT successfully"
        );

        Ok(uploaded_id)
    }

    /// Teleport agent IN (download + restore from storage)
    ///
    /// # Arguments
    /// * `package_id` - Teleport package ID to restore from
    /// * `vm_config` - Configuration for creating new VM
    ///
    /// # Returns
    /// Tuple of (created VM, agent state bytes)
    ///
    /// # Errors
    /// - NotFound: Package not found in storage
    /// - ArchMismatch: Package architecture doesn't match host
    /// - ImageMismatch: Base image version mismatch (validated by storage)
    /// - DownloadFailed: Failed to download package
    /// - VM creation/restore failed
    ///
    /// # TigerStyle
    /// - Creates new VM (caller owns it)
    /// - Returns agent state for caller to deserialize
    /// - Clean failure (no partial state on error)
    /// - Version validation happens in storage layer (MAJOR.MINOR must match)
    /// - VM is started before returning
    pub async fn teleport_in(
        &self,
        package_id: &str,
        vm_config: VmConfig,
    ) -> Result<(Box<dyn VmInstance>, Bytes)> {
        // Preconditions
        assert!(!package_id.is_empty(), "package_id must not be empty");

        tracing::info!(package_id = %package_id, "Teleporting agent IN");

        // Step 1: Download and validate package
        let host_arch = self.storage.host_arch();
        let package = self
            .storage
            .download_for_restore(package_id, host_arch)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to download teleport package: {}", e),
            })?;

        tracing::debug!(
            package_id = %package_id,
            agent_id = %package.agent_id,
            kind = ?package.kind,
            "Package downloaded successfully (version validated by storage)"
        );

        // Step 2: Create new VM
        let mut vm = self
            .vm_factory
            .create(vm_config)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to create VM: {}", e),
            })?;

        // Step 4: Restore from snapshot (for full teleport/suspend)
        if matches!(package.kind, SnapshotKind::Suspend | SnapshotKind::Teleport) {
            if let Some(ref vm_snapshot) = package.vm_snapshot {
                let decoded =
                    kelpie_core::teleport::VmSnapshotBlob::decode(vm_snapshot).map_err(|e| {
                        Error::Internal {
                            message: format!("Failed to decode snapshot blob: {}", e),
                        }
                    })?;
                let metadata: VmSnapshotMetadata = serde_json::from_slice(&decoded.metadata_bytes)
                    .map_err(|e| Error::Internal {
                        message: format!("Failed to deserialize snapshot metadata: {}", e),
                    })?;
                let snapshot = VmSnapshot::new(metadata, decoded.snapshot_bytes).map_err(|e| {
                    Error::Internal {
                        message: format!("Failed to rebuild snapshot: {}", e),
                    }
                })?;

                vm.restore(&snapshot).await.map_err(|e| Error::Internal {
                    message: format!("Failed to restore VM: {}", e),
                })?;
            }
        }

        // Step 5: Start VM after restore/creation
        vm.start().await.map_err(|e| Error::Internal {
            message: format!("Failed to start VM: {}", e),
        })?;

        // Extract agent state
        let agent_state = package.agent_state.unwrap_or_default();

        // Step 6: Register agent in global registry (Option 1: RegistryActor)
        // Deserialize agent state to extract metadata and send message to RegistryActor
        if !agent_state.is_empty() {
            if let Some(ref dispatcher) = self.dispatcher {
                match serde_json::from_slice::<AgentActorState>(&agent_state) {
                    Ok(actor_state) => {
                        if let Some(agent) = actor_state.agent {
                            // Convert AgentState to AgentMetadata
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

                            // Send register message to RegistryActor
                            let registry_id = ActorId::new("system", "agent_registry")?;
                            let request = RegisterRequest { metadata };
                            let payload =
                                serde_json::to_vec(&request).map_err(|e| Error::Internal {
                                    message: format!("Failed to serialize RegisterRequest: {}", e),
                                })?;

                            match dispatcher
                                .invoke(registry_id, "register".to_string(), Bytes::from(payload))
                                .await
                            {
                                Ok(_) => {
                                    tracing::info!(
                                        agent_id = %agent.id,
                                        "Agent registered via RegistryActor after teleport"
                                    );
                                }
                                Err(e) => {
                                    // Non-fatal: registration failure doesn't prevent teleport
                                    tracing::warn!(
                                        agent_id = %agent.id,
                                        error = %e,
                                        "Failed to register with RegistryActor (non-fatal)"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            package_id = %package_id,
                            error = %e,
                            "Failed to deserialize agent state for registration"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    package_id = %package_id,
                    "No dispatcher configured, skipping RegistryActor registration"
                );
            }
        }

        tracing::info!(
            package_id = %package_id,
            agent_id = %package.agent_id,
            "Agent teleported IN successfully"
        );

        Ok((vm, agent_state))
    }

    /// List available teleport packages
    pub async fn list_packages(&self) -> Vec<String> {
        self.storage.list().await
    }

    /// Delete a teleport package
    pub async fn delete_package(&self, package_id: &str) -> Result<()> {
        self.storage
            .delete(package_id)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to delete teleport package: {}", e),
            })
    }

    /// Get package details without downloading full state
    pub async fn get_package(&self, package_id: &str) -> Result<TeleportPackage> {
        self.storage
            .download(package_id)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to get teleport package: {}", e),
            })
    }

    /// Get the host architecture
    pub fn host_arch(&self) -> Architecture {
        self.storage.host_arch()
    }
}

/// Request for teleport_out operation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeleportOutRequest {
    /// Agent ID to teleport
    pub agent_id: String,
    /// Snapshot kind (Suspend, Teleport, Checkpoint)
    pub kind: SnapshotKind,
}

/// Response from teleport_out operation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeleportOutResponse {
    /// Teleport package ID
    pub package_id: String,
    /// Source architecture
    pub source_arch: Architecture,
    /// Package size in bytes
    pub size_bytes: u64,
}

/// Request for teleport_in operation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeleportInRequest {
    /// Teleport package ID to restore from
    pub package_id: String,
}

/// Response from teleport_in operation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeleportInResponse {
    /// Restored agent ID
    pub agent_id: String,
    /// Snapshot kind that was restored
    pub kind: SnapshotKind,
}

/// Package info for listing
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TeleportPackageInfo {
    /// Package ID
    pub id: String,
    /// Agent ID
    pub agent_id: String,
    /// Source architecture
    pub source_arch: Architecture,
    /// Snapshot kind
    pub kind: SnapshotKind,
    /// Creation timestamp (ms)
    pub created_at_ms: u64,
    /// Package size in bytes
    pub size_bytes: u64,
}

impl From<TeleportPackage> for TeleportPackageInfo {
    fn from(p: TeleportPackage) -> Self {
        TeleportPackageInfo {
            id: p.id,
            agent_id: p.agent_id,
            source_arch: p.source_arch,
            kind: p.kind,
            created_at_ms: p.created_at_ms,
            size_bytes: p.size_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalTeleportStorage;
    use kelpie_vm::{MockVmFactory, VmConfig, VmState};

    fn test_config() -> VmConfig {
        VmConfig::builder()
            .vcpu_count(2)
            .memory_mib(512)
            .root_disk("/tmp/rootfs.ext4")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_teleport_service_roundtrip() {
        let storage = Arc::new(LocalTeleportStorage::new());
        let factory = Arc::new(MockVmFactory::new());
        let service = TeleportService::new(storage, factory.clone());

        let mut vm = factory.create_vm(test_config()).unwrap();
        vm.start().await.unwrap();

        // Teleport out
        let agent_state = Bytes::from("test agent state");
        let package_id = service
            .teleport_out(
                "agent-1",
                &mut vm,
                agent_state.clone(),
                SnapshotKind::Teleport,
            )
            .await
            .unwrap();

        assert!(package_id.contains("agent-1"));

        // Verify package exists
        let packages = service.list_packages().await;
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0], package_id);

        // Teleport in
        let (new_vm, restored_state) = service
            .teleport_in(&package_id, test_config())
            .await
            .unwrap();
        assert_eq!(restored_state, agent_state);
        assert_eq!(new_vm.state(), VmState::Running);

        // Cleanup
        service.delete_package(&package_id).await.unwrap();
        let packages = service.list_packages().await;
        assert!(packages.is_empty());
    }

    #[tokio::test]
    async fn test_teleport_service_checkpoint() {
        let storage = Arc::new(LocalTeleportStorage::new());
        let factory = Arc::new(MockVmFactory::new());
        let service = TeleportService::new(storage, factory.clone());

        let mut vm = factory.create_vm(test_config()).unwrap();
        vm.start().await.unwrap();

        // Teleport out as checkpoint
        let agent_state = Bytes::from("checkpoint state");
        let package_id = service
            .teleport_out(
                "agent-2",
                &mut vm,
                agent_state.clone(),
                SnapshotKind::Checkpoint,
            )
            .await
            .unwrap();

        // Get package details
        let package = service.get_package(&package_id).await.unwrap();
        assert_eq!(package.kind, SnapshotKind::Checkpoint);
        assert!(package.is_checkpoint());
        assert!(!package.is_full_teleport()); // Checkpoint has no VM state

        // Teleport in
        let (new_vm, restored_state) = service
            .teleport_in(&package_id, test_config())
            .await
            .unwrap();
        assert_eq!(restored_state, agent_state);
        assert_eq!(new_vm.state(), VmState::Running);
    }
}
