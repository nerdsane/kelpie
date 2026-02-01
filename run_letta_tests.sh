#!/bin/bash
set -e

cd ~/letta-sdk-test
source .venv/bin/activate

# Install missing module
pip install -q asyncpg

# Get API key from .env
ANTHROPIC_API_KEY=$(grep ANTHROPIC_API_KEY ~/Development/kelpie/.env | cut -d= -f2)

# Run tests
LETTA_SERVER_URL=http://localhost:8283 \
ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
pytest tests/sdk/agents_test.py -v --tb=short 2>&1

echo "Tests complete"
