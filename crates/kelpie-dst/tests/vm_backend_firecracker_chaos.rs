#[cfg(all(feature = "firecracker", target_os = "linux"))]
mod tests {
    use kelpie_vm::{FirecrackerConfig, VmBackendFactory};
    use kelpie_vm::{VmConfig, VmError, VmFactory};

    #[madsim::test]
    async fn test_firecracker_factory_create_missing_kernel() {
        let factory = VmBackendFactory::firecracker(FirecrackerConfig::default());
        let config = VmConfig::builder()
            .root_disk("/tmp/kelpie-missing-rootfs")
            .kernel_image("/tmp/kelpie-missing-kernel")
            .build()
            .expect("config should build");

        let result = factory.create(config).await;
        match result {
            Err(VmError::ConfigInvalid { .. }) => {}
            Ok(_) => panic!("expected ConfigInvalid error, but VM creation succeeded"),
            Err(e) => panic!("expected ConfigInvalid, got different error: {}", e),
        }
    }
}
