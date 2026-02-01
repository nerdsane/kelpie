# VM Backends & Hypervisors

Kelpie uses a **multi-backend architecture** for VM management, allowing different hypervisors based on platform and use case.

## Backend Selection Strategy

| Backend | Platform | Use Case | Snapshot Support |
|---------|----------|----------|------------------|
| **MockVm** | All | Testing, DST, CI/CD | ✅ Simulated |
| **Apple Vz** | macOS | Production (Mac dev) | ✅ Native API (macOS 14+) |
| **Firecracker** | Linux | Production (cloud) | ✅ Production-proven |

## Why Multiple Backends?

1. **Platform-Native Performance**: Use native hypervisors for best performance
2. **Testing Everywhere**: MockVm works without system dependencies
3. **Production-Ready**: Apple Vz and Firecracker have mature snapshot APIs
4. **Cross-Platform Development**: Mac devs use Apple Vz, Linux devs use Firecracker

## Quick Testing Guide

```bash
# Default: MockVm (no system dependencies, works everywhere)
cargo test -p kelpie-vm

# Apple Vz backend (macOS only)
cargo test -p kelpie-vm --features vz

# Firecracker backend (Linux only)
cargo test -p kelpie-vm --features firecracker
```

## Platform-Specific Commands

```bash
# macOS Development
cargo test -p kelpie-vm --features vz
cargo run -p kelpie-server --features vz

# Linux Development
cargo test -p kelpie-vm --features firecracker
cargo run -p kelpie-server --features firecracker

# Testing (all platforms)
cargo test -p kelpie-vm  # Uses MockVm by default
DST_SEED=12345 cargo test -p kelpie-dst
```

## Architecture Compatibility

**Same-Architecture Teleport** (VM Snapshot):
- ✅ Mac ARM64 → AWS Graviton ARM64
- ✅ Linux x86_64 → Linux x86_64
- ✅ Full VM memory state preserved
- ✅ Fast restore (~125-500ms)

**Cross-Architecture Migration** (Checkpoint):
- ✅ Mac ARM64 → Linux x86_64 (agent state only)
- ✅ Linux x86_64 → Mac ARM64 (agent state only)
- ❌ VM memory cannot be transferred (CPU incompatibility)
- ⚠️ Slower (VM restarts fresh, agent state reloaded)

**Implementation Plan**: See `.progress/016_20260115_121324_teleport-dual-backend-implementation.md`
