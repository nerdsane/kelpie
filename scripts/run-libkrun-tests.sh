#!/bin/bash
#
# Run libkrun integration tests with proper code signing on macOS
#
# libkrun requires the com.apple.security.hypervisor entitlement to create VMs.
# This script builds the test binary, signs it with the required entitlement,
# and runs it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Create entitlements file
ENTITLEMENTS_FILE="$SCRIPT_DIR/hypervisor.entitlements.plist"
cat > "$ENTITLEMENTS_FILE" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
EOF

echo "=== Building test binary ==="
cd "$PROJECT_ROOT"
cargo build --package kelpie-server --features libkrun --tests 2>&1

# Find the test binary
TEST_BIN=$(find target/debug/deps -name 'sandbox_provider_integration-*' -type f ! -name '*.d' | head -1)

if [ -z "$TEST_BIN" ]; then
    echo "Error: Could not find test binary"
    exit 1
fi

echo "=== Signing test binary with Hypervisor entitlement ==="
echo "Binary: $TEST_BIN"
codesign --force --sign - --entitlements "$ENTITLEMENTS_FILE" "$TEST_BIN" 2>&1

echo "=== Running tests ==="
# Set DYLD_LIBRARY_PATH for libkrunfw (won't work in SIP-restricted context)
# The library must be in /usr/local/lib or another trusted location
DYLD_LIBRARY_PATH=/usr/local/lib "$TEST_BIN" --test-threads=1 "$@"
