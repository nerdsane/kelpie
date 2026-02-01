//! VM backend selection and factories
//!
//! TigerStyle: Explicit backend enums with feature gating.

use async_trait::async_trait;

use crate::error::{VmError, VmResult};
use crate::traits::{VmFactory, VmInstance, VmState};
use crate::{MockVm, MockVmFactory, VmConfig, VmSnapshot};

#[cfg(feature = "firecracker")]
pub use crate::backends::firecracker::FirecrackerConfig;

#[cfg(feature = "firecracker")]
use crate::backends::firecracker::{FirecrackerVm, FirecrackerVmFactory};

/// VM backend variants
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Different backends have different sizes
pub enum VmBackend {
    /// Mock VM backend (testing)
    Mock(MockVm),

    /// Firecracker backend (Linux)
    #[cfg(feature = "firecracker")]
    Firecracker(FirecrackerVm),
}

#[async_trait]
impl VmInstance for VmBackend {
    fn id(&self) -> &str {
        match self {
            VmBackend::Mock(vm) => vm.id(),
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.id(),
        }
    }

    fn state(&self) -> VmState {
        match self {
            VmBackend::Mock(vm) => vm.state(),
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.state(),
        }
    }

    fn config(&self) -> &VmConfig {
        match self {
            VmBackend::Mock(vm) => vm.config(),
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.config(),
        }
    }

    async fn start(&mut self) -> VmResult<()> {
        match self {
            VmBackend::Mock(vm) => vm.start().await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.start().await,
        }
    }

    async fn stop(&mut self) -> VmResult<()> {
        match self {
            VmBackend::Mock(vm) => vm.stop().await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.stop().await,
        }
    }

    async fn pause(&mut self) -> VmResult<()> {
        match self {
            VmBackend::Mock(vm) => vm.pause().await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.pause().await,
        }
    }

    async fn resume(&mut self) -> VmResult<()> {
        match self {
            VmBackend::Mock(vm) => vm.resume().await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.resume().await,
        }
    }

    async fn exec(&self, cmd: &str, args: &[&str]) -> VmResult<crate::traits::ExecOutput> {
        match self {
            VmBackend::Mock(vm) => vm.exec(cmd, args).await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.exec(cmd, args).await,
        }
    }

    async fn exec_with_options(
        &self,
        cmd: &str,
        args: &[&str],
        options: crate::traits::ExecOptions,
    ) -> VmResult<crate::traits::ExecOutput> {
        match self {
            VmBackend::Mock(vm) => vm.exec_with_options(cmd, args, options).await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.exec_with_options(cmd, args, options).await,
        }
    }

    async fn snapshot(&self) -> VmResult<VmSnapshot> {
        match self {
            VmBackend::Mock(vm) => vm.snapshot().await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.snapshot().await,
        }
    }

    async fn restore(&mut self, snapshot: &VmSnapshot) -> VmResult<()> {
        match self {
            VmBackend::Mock(vm) => vm.restore(snapshot).await,
            #[cfg(feature = "firecracker")]
            VmBackend::Firecracker(vm) => vm.restore(snapshot).await,
        }
    }
}

/// Backend selection for VmBackendFactory
#[derive(Debug, Clone, Copy)]
pub enum VmBackendKind {
    /// Use MockVm backend (default)
    Mock,

    /// Use Firecracker backend (feature-gated)
    #[cfg(feature = "firecracker")]
    Firecracker,
}

/// Factory for creating VmBackend instances
#[derive(Debug, Clone)]
pub struct VmBackendFactory {
    kind: VmBackendKind,
    mock_factory: MockVmFactory,
    #[cfg(feature = "firecracker")]
    firecracker_factory: Option<FirecrackerVmFactory>,
}

impl VmBackendFactory {
    /// Create a factory with MockVm backend
    pub fn mock() -> Self {
        Self {
            kind: VmBackendKind::Mock,
            mock_factory: MockVmFactory::new(),
            #[cfg(feature = "firecracker")]
            firecracker_factory: None,
        }
    }

    /// Create a factory with Firecracker backend
    #[cfg(feature = "firecracker")]
    pub fn firecracker(config: FirecrackerConfig) -> Self {
        Self {
            kind: VmBackendKind::Firecracker,
            mock_factory: MockVmFactory::new(),
            firecracker_factory: Some(FirecrackerVmFactory::new(config)),
        }
    }

    /// Create a factory that chooses the native backend for the host
    #[allow(unreachable_code)] // False positive with conditional compilation
    pub fn for_host() -> Self {
        #[cfg(all(feature = "firecracker", target_os = "linux"))]
        {
            return Self::firecracker(FirecrackerConfig::default());
        }

        Self::mock()
    }
}

#[async_trait]
impl VmFactory for VmBackendFactory {
    async fn create(&self, config: VmConfig) -> VmResult<Box<dyn VmInstance>> {
        match self.kind {
            VmBackendKind::Mock => {
                let vm =
                    self.mock_factory
                        .create_vm(config)
                        .map_err(|e| VmError::CreateFailed {
                            reason: e.to_string(),
                        })?;
                Ok(Box::new(VmBackend::Mock(vm)))
            }
            #[cfg(feature = "firecracker")]
            VmBackendKind::Firecracker => {
                let factory =
                    self.firecracker_factory
                        .as_ref()
                        .ok_or_else(|| VmError::CreateFailed {
                            reason: "Firecracker factory not configured".to_string(),
                        })?;
                let vm = factory.create_vm(config).await?;
                Ok(Box::new(VmBackend::Firecracker(vm)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "firecracker", target_os = "linux"))]
    #[test]
    fn test_for_host_firecracker() {
        let factory = VmBackendFactory::for_host();
        assert!(matches!(factory.kind, VmBackendKind::Firecracker));
    }

    #[cfg(not(all(feature = "firecracker", target_os = "linux")))]
    #[test]
    fn test_for_host_mock_fallback() {
        let factory = VmBackendFactory::for_host();
        assert!(matches!(factory.kind, VmBackendKind::Mock));
    }
}
