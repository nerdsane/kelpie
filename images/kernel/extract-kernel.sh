#!/usr/bin/env bash
#
# Extract kernel and initrd from Alpine Linux Docker image
#
# TigerStyle: Explicit error handling, clear output, reproducible results

set -euo pipefail

# Constants
readonly ALPINE_VERSION="${ALPINE_VERSION:-3.19}"
readonly ARCH="${ARCH:-$(uname -m)}"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly OUTPUT_DIR="${OUTPUT_DIR:-$SCRIPT_DIR}"

# Colors for output
readonly RED='\033[0;31m'
readonly GREEN='\033[0;32m'
readonly YELLOW='\033[1;33m'
readonly NC='\033[0m' # No Color

# Map architecture names
map_arch() {
    case "$1" in
        aarch64) echo "aarch64" ;;
        arm64) echo "aarch64" ;;
        x86_64) echo "x86_64" ;;
        amd64) echo "x86_64" ;;
        *) echo "$1" ;;
    esac
}

ALPINE_ARCH=$(map_arch "$ARCH")

echo -e "${GREEN}Extracting Alpine Linux Kernel${NC}"
echo "Alpine version: $ALPINE_VERSION"
echo "Architecture:   $ALPINE_ARCH"
echo "Output dir:     $OUTPUT_DIR"
echo ""

# Create and start container with linux-virt package
echo -e "${YELLOW}Creating container and installing linux-virt package...${NC}"

CONTAINER_ID=$(docker run -d "alpine:$ALPINE_VERSION" sleep 300)

cleanup() {
    if [ -n "${CONTAINER_ID:-}" ]; then
        echo -e "${YELLOW}Cleaning up container...${NC}"
        docker rm -f "$CONTAINER_ID" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

# Install linux-virt package (VM-optimized kernel)
docker exec "$CONTAINER_ID" apk add --no-cache linux-virt || {
    echo -e "${RED}Failed to install linux-virt package${NC}"
    exit 1
}

# Find kernel and initrd files
echo -e "${YELLOW}Locating kernel files...${NC}"

KERNEL_FILE=$(docker exec "$CONTAINER_ID" sh -c 'ls /boot/vmlinuz-virt 2>/dev/null || ls /boot/vmlinuz-* | head -1' || echo "")
INITRD_FILE=$(docker exec "$CONTAINER_ID" sh -c 'ls /boot/initramfs-virt 2>/dev/null || ls /boot/initramfs-* | grep -v rescue | head -1' || echo "")

if [ -z "$KERNEL_FILE" ]; then
    echo -e "${RED}Error: Kernel file not found${NC}"
    exit 1
fi

if [ -z "$INITRD_FILE" ]; then
    echo -e "${YELLOW}Warning: Initrd file not found (optional)${NC}"
fi

echo "Kernel: $KERNEL_FILE"
if [ -n "$INITRD_FILE" ]; then
    echo "Initrd: $INITRD_FILE"
fi

# Extract kernel - need raw Image format for VZLinuxBootLoader, not EFI stub
echo -e "${YELLOW}Extracting kernel...${NC}"
OUTPUT_KERNEL="$OUTPUT_DIR/vmlinuz-$ALPINE_ARCH"

# First copy the vmlinuz
docker cp "$CONTAINER_ID:$KERNEL_FILE" "$OUTPUT_KERNEL"

if [ ! -f "$OUTPUT_KERNEL" ]; then
    echo -e "${RED}✗ Failed to extract kernel${NC}"
    exit 1
fi

# Check if it's EFI stub format (PE32+) - if so, extract raw Image
FILE_TYPE=$(file "$OUTPUT_KERNEL")
if echo "$FILE_TYPE" | grep -q "PE32+"; then
    echo -e "${YELLOW}Kernel is EFI stub format, extracting raw Image...${NC}"

    # Find gzip offset and extract
    GZIP_OFFSET=$(od -A d -t x1 "$OUTPUT_KERNEL" | grep "1f 8b 08" | head -1 | awk '{print $1}')

    if [ -n "$GZIP_OFFSET" ]; then
        dd if="$OUTPUT_KERNEL" of="$OUTPUT_KERNEL.gz" bs=1 skip="$GZIP_OFFSET" 2>/dev/null
        gunzip -c "$OUTPUT_KERNEL.gz" > "$OUTPUT_KERNEL.tmp" 2>/dev/null
        mv "$OUTPUT_KERNEL.tmp" "$OUTPUT_KERNEL"
        rm -f "$OUTPUT_KERNEL.gz"

        NEW_TYPE=$(file "$OUTPUT_KERNEL")
        echo "  Extracted: $NEW_TYPE"
    else
        echo -e "${YELLOW}Warning: Could not find gzip offset, keeping original${NC}"
    fi
fi

KERNEL_SIZE=$(du -h "$OUTPUT_KERNEL" | cut -f1)
echo -e "${GREEN}✓ Kernel extracted: $OUTPUT_KERNEL ($KERNEL_SIZE)${NC}"

# Extract initrd if present
if [ -n "$INITRD_FILE" ]; then
    echo -e "${YELLOW}Extracting initrd...${NC}"
    OUTPUT_INITRD="$OUTPUT_DIR/initramfs-$ALPINE_ARCH"
    docker cp "$CONTAINER_ID:$INITRD_FILE" "$OUTPUT_INITRD"

    if [ -f "$OUTPUT_INITRD" ]; then
        INITRD_SIZE=$(du -h "$OUTPUT_INITRD" | cut -f1)
        echo -e "${GREEN}✓ Initrd extracted: $OUTPUT_INITRD ($INITRD_SIZE)${NC}"
    else
        echo -e "${YELLOW}⚠ Initrd extraction failed (optional)${NC}"
    fi
fi

# Get kernel version
echo -e "${YELLOW}Detecting kernel version...${NC}"
KERNEL_VERSION=$(docker exec "$CONTAINER_ID" sh -c 'apk info linux-virt | grep "linux-virt-" | head -1' | sed 's/linux-virt-//' | awk '{print $1}' || echo "unknown")
echo "Kernel version: $KERNEL_VERSION"

# Create metadata file
METADATA_FILE="$OUTPUT_DIR/kernel-metadata.json"
cat > "$METADATA_FILE" <<EOF
{
  "alpine_version": "$ALPINE_VERSION",
  "architecture": "$ALPINE_ARCH",
  "kernel_version": "$KERNEL_VERSION",
  "kernel_file": "$(basename "$OUTPUT_KERNEL")",
  "initrd_file": "$(basename "${OUTPUT_INITRD:-none}")",
  "extracted_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

echo ""
echo -e "${GREEN}✓ Extraction complete!${NC}"
echo ""
echo "Files:"
echo "  Kernel:   $OUTPUT_KERNEL"
if [ -n "${OUTPUT_INITRD:-}" ] && [ -f "$OUTPUT_INITRD" ]; then
    echo "  Initrd:   $OUTPUT_INITRD"
fi
echo "  Metadata: $METADATA_FILE"
echo ""
echo "To use with libkrun:"
echo "  libkrun_set_kernel_path(\"$OUTPUT_KERNEL\")"
if [ -n "${OUTPUT_INITRD:-}" ] && [ -f "$OUTPUT_INITRD" ]; then
    echo "  libkrun_set_initrd_path(\"$OUTPUT_INITRD\")"
fi
