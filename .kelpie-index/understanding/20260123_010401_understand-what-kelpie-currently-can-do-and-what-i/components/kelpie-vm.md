# kelpie-vm

**Examined:** 2026-01-23T01:02:14.157164+00:00

## Summary

VM abstraction layer with MockVm, Apple Vz, and Firecracker backends for sandbox execution

## Connections

- kelpie-core
- kelpie-dst
- kelpie-sandbox

## Details

**WORKING (36 tests pass):**
- VmInstance trait - lifecycle (start, stop, pause, resume), exec, snapshot/restore
- MockVm - test implementation with simulated commands
- VmSnapshot with CRC32 checksums and architecture compatibility
- VirtioFS mount configuration
- VmConfig builder pattern with resource limits

**Backends:**
- MockVm (working) - for testing without hypervisor
- Apple Vz (macOS, feature-gated) - Virtualization.framework via C FFI bridge
- Firecracker (Linux, feature-gated) - wraps kelpie-sandbox::FirecrackerSandbox

**Resource limits defined:**
- vCPU: 32 max
- RAM: 128-16384 MiB
- Snapshot: 1 GiB max
- Mounts: 16 max

## Issues

### [LOW] MockVm command execution is hardcoded (only ~6 commands)

**Evidence:** mock.rs - shell simulation extremely basic

### [LOW] Snapshot cleanup ignores errors silently in Firecracker backend

**Evidence:** firecracker.rs - cleanup error suppression
