//! Tests for base image version validation in TeleportService
//!
//! TigerStyle: Explicit test cases, clear assertions

use bytes::Bytes;
use kelpie_core::{Error, Result};
use kelpie_server::service::TeleportService;
use kelpie_server::storage::{KvAdapter, LocalTeleportStorage, SnapshotKind, TeleportStorage};
use kelpie_storage::MemoryKV;
use kelpie_vm::{MockVmFactory, VmConfig, VmInstance};
use std::sync::Arc;

/// Helper to create a mock agent storage
#[allow(dead_code)]
fn mock_agent_storage() -> Arc<KvAdapter> {
    Arc::new(KvAdapter::new(Arc::new(MemoryKV::new())))
}

fn test_config() -> VmConfig {
    VmConfig::builder()
        .vcpu_count(2)
        .memory_mib(512)
        .root_disk("/tmp/rootfs.ext4")
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_version_validation_same_major_minor() -> Result<()> {
    // Setup: Service with version 1.0.0
    let storage = Arc::new(LocalTeleportStorage::new());
    let factory = Arc::new(MockVmFactory::new());
    let service =
        TeleportService::new(storage.clone(), factory.clone()).with_base_image_version("1.0.0");

    // Create VM and teleport out
    let mut vm = factory
        .create_vm(test_config())
        .map_err(|e| Error::Internal {
            message: format!("Failed to create VM: {}", e),
        })?;
    vm.start().await.map_err(|e| Error::Internal {
        message: format!("Failed to start VM: {}", e),
    })?;
    let package_id = service
        .teleport_out(
            "agent-1",
            &mut vm,
            Bytes::from("state"),
            SnapshotKind::Teleport,
        )
        .await?;

    // Teleport in should succeed (same MAJOR.MINOR)
    let result = service.teleport_in(&package_id, test_config()).await;
    assert!(
        result.is_ok(),
        "Should succeed with same MAJOR.MINOR version"
    );

    Ok(())
}

#[tokio::test]
async fn test_version_validation_patch_difference_allowed() -> Result<()> {
    // Setup: Service with version 1.0.0 creates package
    let storage = Arc::new(LocalTeleportStorage::new());
    let factory = Arc::new(MockVmFactory::new());
    let service_v1 =
        TeleportService::new(storage.clone(), factory.clone()).with_base_image_version("1.0.0");

    // Create VM and teleport out with v1.0.0
    let mut vm = factory
        .create_vm(test_config())
        .map_err(|e| Error::Internal {
            message: format!("Failed to create VM: {}", e),
        })?;
    vm.start().await.map_err(|e| Error::Internal {
        message: format!("Failed to start VM: {}", e),
    })?;
    let package_id = service_v1
        .teleport_out(
            "agent-2",
            &mut vm,
            Bytes::from("state"),
            SnapshotKind::Teleport,
        )
        .await?;

    // Service with version 1.0.1 tries to restore
    let service_v2 =
        TeleportService::new(storage.clone(), factory.clone()).with_base_image_version("1.0.1");

    // Teleport in should succeed (PATCH difference allowed)
    let result = service_v2.teleport_in(&package_id, test_config()).await;
    assert!(
        result.is_ok(),
        "Should succeed with different PATCH version"
    );

    Ok(())
}

#[tokio::test]
async fn test_version_validation_major_mismatch_rejected() -> Result<()> {
    // Setup: Service with version 1.0.0 creates package
    let storage_v1 = Arc::new(LocalTeleportStorage::new().with_expected_image_version("1.0.0"));
    let factory = Arc::new(MockVmFactory::new());
    let service_v1 =
        TeleportService::new(storage_v1.clone(), factory.clone()).with_base_image_version("1.0.0");

    // Create VM and teleport out
    let mut vm = factory
        .create_vm(test_config())
        .map_err(|e| Error::Internal {
            message: format!("Failed to create VM: {}", e),
        })?;
    vm.start().await.map_err(|e| Error::Internal {
        message: format!("Failed to start VM: {}", e),
    })?;
    let package_id = service_v1
        .teleport_out(
            "agent-3",
            &mut vm,
            Bytes::from("state"),
            SnapshotKind::Teleport,
        )
        .await?;

    // Service with version 2.0.0 tries to restore (needs storage with 2.0.0 expected version)
    let storage_v2 = Arc::new(LocalTeleportStorage::new().with_expected_image_version("2.0.0"));
    // Copy the package to the new storage
    let package = storage_v1.download(&package_id).await.unwrap();
    storage_v2.upload(package).await.unwrap();

    let service_v2 =
        TeleportService::new(storage_v2, factory.clone()).with_base_image_version("2.0.0");

    // Teleport in should FAIL (MAJOR mismatch)
    let result = service_v2.teleport_in(&package_id, test_config()).await;
    assert!(result.is_err(), "Should fail with different MAJOR version");

    let err_msg = match result {
        Ok(_) => panic!("Expected error, got success"),
        Err(e) => e.to_string(),
    };
    assert!(
        err_msg.contains("version mismatch"),
        "Error should mention version mismatch"
    );
    assert!(err_msg.contains("1.0"), "Error should show package version");
    assert!(err_msg.contains("2.0"), "Error should show host version");

    Ok(())
}

#[tokio::test]
async fn test_version_validation_minor_mismatch_rejected() -> Result<()> {
    // Setup: Service with version 1.1.0 creates package
    let storage_v1 = Arc::new(LocalTeleportStorage::new().with_expected_image_version("1.1.0"));
    let factory = Arc::new(MockVmFactory::new());
    let service_v1 =
        TeleportService::new(storage_v1.clone(), factory.clone()).with_base_image_version("1.1.0");

    // Create VM and teleport out
    let mut vm = factory
        .create_vm(test_config())
        .map_err(|e| Error::Internal {
            message: format!("Failed to create VM: {}", e),
        })?;
    vm.start().await.map_err(|e| Error::Internal {
        message: format!("Failed to start VM: {}", e),
    })?;
    let package_id = service_v1
        .teleport_out(
            "agent-4",
            &mut vm,
            Bytes::from("state"),
            SnapshotKind::Teleport,
        )
        .await?;

    // Service with version 1.2.0 tries to restore (needs storage with 1.2.0 expected version)
    let storage_v2 = Arc::new(LocalTeleportStorage::new().with_expected_image_version("1.2.0"));
    // Copy the package to the new storage
    let package = storage_v1.download(&package_id).await.unwrap();
    storage_v2.upload(package).await.unwrap();

    let service_v2 =
        TeleportService::new(storage_v2, factory.clone()).with_base_image_version("1.2.0");

    // Teleport in should FAIL (MINOR mismatch)
    let result = service_v2.teleport_in(&package_id, test_config()).await;
    assert!(result.is_err(), "Should fail with different MINOR version");

    let err_msg = match result {
        Ok(_) => panic!("Expected error, got success"),
        Err(e) => e.to_string(),
    };
    assert!(err_msg.contains("1.1"), "Error should show package version");
    assert!(err_msg.contains("1.2"), "Error should show host version");

    Ok(())
}

#[tokio::test]
async fn test_version_validation_with_prerelease() -> Result<()> {
    // Setup: Service with version 1.0.0-dev-20260115-abc1234
    let storage = Arc::new(LocalTeleportStorage::new());
    let factory = Arc::new(MockVmFactory::new());
    let service = TeleportService::new(storage.clone(), factory.clone())
        .with_base_image_version("1.0.0-dev-20260115-abc1234");

    // Create VM and teleport out
    let mut vm = factory
        .create_vm(test_config())
        .map_err(|e| Error::Internal {
            message: format!("Failed to create VM: {}", e),
        })?;
    vm.start().await.map_err(|e| Error::Internal {
        message: format!("Failed to start VM: {}", e),
    })?;
    let package_id = service
        .teleport_out(
            "agent-5",
            &mut vm,
            Bytes::from("state"),
            SnapshotKind::Teleport,
        )
        .await?;

    // Service with different prerelease but same MAJOR.MINOR.PATCH tries to restore
    let service_v2 = TeleportService::new(storage.clone(), factory.clone())
        .with_base_image_version("1.0.0-prod-20260116-def5678");

    // Teleport in should succeed (prerelease metadata ignored for compatibility)
    let result = service_v2.teleport_in(&package_id, test_config()).await;
    assert!(
        result.is_ok(),
        "Should succeed - prerelease metadata doesn't affect compatibility"
    );

    Ok(())
}
