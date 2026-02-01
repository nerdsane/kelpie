# Base Images

Kelpie agents run in lightweight Alpine Linux microVMs for isolation and teleportation. The base image system (Phases 5.1-5.6) provides:

## Quick Reference

```bash
# Build images locally
cd images && ./build.sh --arch arm64 --version 1.0.0

# Extract kernel/initramfs
cd images/kernel && ./extract-kernel.sh

# Run tests
cargo test -p kelpie-server --test version_validation_test
```

## Key Features

1. **Alpine 3.19 Base** (~28.8MB)
   - Essential packages: busybox, bash, coreutils, util-linux
   - Multi-arch: ARM64 + x86_64
   - VM-optimized kernel (linux-virt 6.6.x)

2. **Guest Agent** (Rust)
   - Unix socket communication (virtio-vsock in production)
   - Command execution with stdin/stdout/stderr
   - File operations (read, write, list)
   - Health monitoring (ping/pong)

3. **Custom Init System**
   - Mounts essential filesystems (proc, sys, dev, tmp, run)
   - Starts guest agent automatically
   - Graceful shutdown handling
   - Boot time: <1s

4. **Version Compatibility**
   - Format: `MAJOR.MINOR.PATCH[-prerelease]-DATE-GITSHA`
   - MAJOR.MINOR must match for teleport compatibility
   - PATCH differences allowed (with warning)
   - Prerelease metadata ignored

5. **CI/CD Pipeline**
   - GitHub Actions with native ARM64 + x86_64 runners
   - Automated builds on push/release
   - Upload to GitHub Releases + Container Registry
   - Multi-arch Docker manifests

## Documentation

See `images/README.md` for:
- Build instructions
- Image structure
- Guest agent protocol
- Troubleshooting
- Development workflow

## Status

- ✅ Phase 5.1: Build System (complete)
- ✅ Phase 5.2: Guest Agent (complete, 4 tests)
- ✅ Phase 5.3: Init System (complete)
- ✅ Phase 5.4: Kernel Extraction (complete)
- ✅ Phase 5.5: Distribution (complete, GitHub Actions)
- ✅ Phase 5.6: Version Validation (complete, 5 tests)
- ✅ Phase 5.7: libkrun Integration (complete, testing/reference only)
- ✅ Phase 5.9: VM Backends (complete, Apple Vz + Firecracker with DST coverage)
