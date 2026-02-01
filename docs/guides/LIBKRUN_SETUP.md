# libkrun Setup Guide

## Overview

libkrun provides hardware VM isolation using Apple's Hypervisor.framework on macOS ARM64 and KVM on Linux. It bundles its own optimized kernel via libkrunfw, so no kernel management is required.

## Installation

### macOS (Homebrew)

```bash
# Add the krun tap
brew tap slp/krun

# Install libkrun and libkrunfw
brew install libkrun libkrunfw

# Copy to system library path (requires sudo)
sudo cp /opt/homebrew/Cellar/libkrun/*/lib/libkrun*.dylib /usr/local/lib/
sudo cp /opt/homebrew/Cellar/libkrunfw/*/lib/libkrunfw*.dylib /usr/local/lib/
```

### Linux

Follow the libkrun installation guide at https://github.com/containers/libkrun

## Building the Rootfs

libkrun uses a directory-based rootfs (not an ext4 image). Build it using Docker:

```bash
# Build guest agent and extract rootfs
cd images/guest-agent
docker build -t kelpie-guest-build .

# Export to cache directory
ROOTFS_DIR="$HOME/Library/Caches/kelpie/libkrun-rootfs"  # macOS
# ROOTFS_DIR="$HOME/.cache/kelpie/libkrun-rootfs"        # Linux
rm -rf "$ROOTFS_DIR"
mkdir -p "$ROOTFS_DIR"
docker create --name kelpie-tmp kelpie-guest-build
docker export kelpie-tmp | tar -xf - -C "$ROOTFS_DIR"
docker rm kelpie-tmp
```

## Code Signing (macOS)

Test binaries need the Hypervisor framework entitlement:

```bash
# Create entitlements file
cat > entitlements.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
EOF

# Sign binary
codesign --force --sign - --entitlements entitlements.plist /path/to/binary
```

## Running Tests

```bash
# Build tests
cargo test --package kelpie-server --features libkrun --test sandbox_provider_integration --no-run

# Sign the test binary (macOS)
codesign --force --sign - --entitlements entitlements.plist target/debug/deps/sandbox_provider_integration-*

# Run with library path
DYLD_LIBRARY_PATH=/usr/local/lib cargo test --package kelpie-server --features libkrun
```

## Known Issues

### vsock Communication

The vsock communication between host and guest requires careful setup. The current implementation uses:
- Host: Unix socket created by libkrun with `listen=true`
- Guest: Connects to vsock port 9001 (CID 2 = host)

If vsock connections fail, check:
1. Socket directory exists: `/tmp/kelpie-libkrun/`
2. Guest agent is running: logs should show "Connecting to host via vsock"
3. Unix socket is created by libkrun

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Host (macOS)                        │
│  ┌─────────────────┐     ┌──────────────────────────┐   │
│  │  kelpie-server  │────▶│  Unix Socket             │   │
│  │  (test runner)  │     │  /tmp/kelpie-libkrun/*.sock  │
│  └─────────────────┘     └──────────────────────────┘   │
│                                     │                    │
│                           ┌─────────▼──────────┐        │
│                           │      libkrun       │        │
│                           │  (Hypervisor.framework)     │
│                           └─────────┬──────────┘        │
│                                     │ vsock             │
└─────────────────────────────────────┼───────────────────┘
                                      │
┌─────────────────────────────────────┼───────────────────┐
│                      Guest (Linux VM)                    │
│                           ┌─────────▼──────────┐        │
│                           │   kelpie-guest     │        │
│                           │   (vsock client)   │        │
│                           └────────────────────┘        │
│                                                          │
│  Rootfs: ~/Library/Caches/kelpie/libkrun-rootfs/        │
└─────────────────────────────────────────────────────────┘
```
