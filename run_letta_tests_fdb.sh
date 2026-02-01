#!/bin/bash
# Run Letta SDK tests against Kelpie server with FoundationDB backend
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== Running Letta SDK Tests Against Kelpie+FDB ===${NC}"

# Configuration
FDB_CLUSTER_FILE="${FDB_CLUSTER_FILE:-/usr/local/etc/foundationdb/fdb.cluster}"
LETTA_REPO_DIR="${LETTA_REPO_DIR:-./letta-repo}"
KELPIE_PORT="${KELPIE_PORT:-8283}"
ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY:-sk-dummy-key}"

# Get API key from .env if exists
if [ -f .env ]; then
    ANTHROPIC_API_KEY=$(grep ANTHROPIC_API_KEY .env | cut -d= -f2 || echo "sk-dummy-key")
fi

echo -e "${YELLOW}Config:${NC}"
echo "  FDB Cluster: $FDB_CLUSTER_FILE"
echo "  Kelpie Port: $KELPIE_PORT"
echo "  Letta Repo:  $LETTA_REPO_DIR"
echo ""

# Step 1: Build Kelpie server
echo -e "${GREEN}[1/5] Building Kelpie server...${NC}"
cargo build --release -p kelpie-server

# Step 2: Start Kelpie with FDB backend
echo -e "${GREEN}[2/5] Starting Kelpie server with FDB backend...${NC}"
# Set DYLD_LIBRARY_PATH for macOS to find libfdb_c.dylib
export DYLD_LIBRARY_PATH=/usr/local/lib:${DYLD_LIBRARY_PATH:-}
# Pass ANTHROPIC_API_KEY to server subprocess explicitly
ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY" \
RUST_LOG=info ./target/release/kelpie-server \
    --fdb-cluster-file "$FDB_CLUSTER_FILE" \
    --bind "0.0.0.0:$KELPIE_PORT" &
SERVER_PID=$!

echo "  Server PID: $SERVER_PID"

# Wait for server to be ready (macOS-compatible, no timeout command needed)
echo -e "${YELLOW}Waiting for server health check...${NC}"
WAIT_COUNT=0
MAX_WAIT=30
until curl -s http://localhost:$KELPIE_PORT/health > /dev/null 2>&1; do
    sleep 1
    WAIT_COUNT=$((WAIT_COUNT + 1))
    if [ $WAIT_COUNT -ge $MAX_WAIT ]; then
        echo -e "${RED}ERROR: Server failed to start after ${MAX_WAIT}s${NC}"
        echo -e "${YELLOW}Checking server logs...${NC}"
        jobs -l
        kill $SERVER_PID 2>/dev/null || true
        exit 1
    fi
    echo -n "."
done
echo ""
echo -e "${GREEN}  ✓ Server is ready${NC}"

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    if [ -n "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
        echo "  Killed server (PID: $SERVER_PID)"
    fi
}
trap cleanup EXIT

# Step 3: Clone/update Letta repo
if [ ! -d "$LETTA_REPO_DIR" ]; then
    echo -e "${GREEN}[3/5] Cloning Letta repository...${NC}"
    git clone --depth 1 https://github.com/letta-ai/letta.git "$LETTA_REPO_DIR"
else
    echo -e "${GREEN}[3/5] Using existing Letta repository${NC}"
    echo -e "${YELLOW}  (To update: rm -rf $LETTA_REPO_DIR and re-run)${NC}"
fi

# Step 4: Install Letta SDK
echo -e "${GREEN}[4/5] Installing Letta SDK dependencies...${NC}"
cd "$LETTA_REPO_DIR"

# Create/activate virtual environment (use Python 3.13 for Letta compatibility)
if [ ! -d .venv ]; then
    python3.13 -m venv .venv
fi
source .venv/bin/activate

# Install Letta in editable mode with dev dependencies
pip install -q --upgrade pip
pip install -q -e ".[dev]"
pip install -q asyncpg  # Extra dependency sometimes needed

# Step 5: Run tests
echo -e "${GREEN}[5/5] Running Letta SDK tests against Kelpie+FDB...${NC}"
echo ""

export LETTA_SERVER_URL="http://localhost:$KELPIE_PORT"
export ANTHROPIC_API_KEY="$ANTHROPIC_API_KEY"

# Run core tests (these MUST pass for compatibility)
echo -e "${YELLOW}Running core compatibility tests...${NC}"
pytest tests/sdk/agents_test.py \
       tests/sdk/blocks_test.py \
       tests/sdk/tools_test.py \
       tests/sdk/mcp_servers_test.py \
       -v --tb=short 2>&1 | tee ../letta-fdb-test-results.txt

# Check results
if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo ""
    echo -e "${GREEN}=== ✓ ALL TESTS PASSED ===${NC}"
    echo -e "${GREEN}Letta SDK is compatible with Kelpie+FDB backend${NC}"
else
    echo ""
    echo -e "${RED}=== ✗ SOME TESTS FAILED ===${NC}"
    echo -e "${YELLOW}Check letta-fdb-test-results.txt for details${NC}"
    exit 1
fi
