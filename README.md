# Kelpie

[![CI](https://github.com/rita-aga/kelpie/actions/workflows/ci.yml/badge.svg)](https://github.com/rita-aga/kelpie/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

An open-source agent runtime with stateful memory, sandboxed execution, and MCP tool integration.

## Overview

Kelpie provides infrastructure for building AI agents that:

- **Persist state** across invocations with a three-tier memory hierarchy
- **Execute tools safely** via MCP protocol with process or VM isolation
- **Scale horizontally** with virtual actor model and cluster coordination
- **Test deterministically** with simulation testing and fault injection

## Quick Start

```bash
# Start the server
ANTHROPIC_API_KEY=sk-... cargo run -p kelpie-server

# Create an agent
curl -X POST http://localhost:8283/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-agent",
    "memory_blocks": [
      {"label": "persona", "value": "You are a helpful assistant."}
    ]
  }'

# Send a message
curl -X POST http://localhost:8283/v1/agents/{id}/messages \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": "Hello!"}'
```

## Features

### Memory Hierarchy

| Tier | Purpose | Size | Persistence |
|------|---------|------|-------------|
| **Core** | Always in LLM context | ~32KB | Per-agent |
| **Working** | Session KV store | Unbounded | Per-agent |
| **Archival** | Semantic search | Unbounded | Per-agent |

### Sandbox Isolation

| Level | Implementation | Boot Time |
|-------|----------------|-----------|
| Process | OS process isolation | <10ms |
| VM (macOS) | Apple Virtualization.framework | ~200-500ms |
| VM (Linux) | Firecracker microVM | ~125ms |

### Tool Integration (MCP)

```rust
// Connect to any MCP server
let client = McpClient::new(McpTransportType::Stdio {
    command: "npx".to_string(),
    args: vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
});
client.connect().await?;

// Discover and execute tools
let tools = client.discover_tools().await?;
let result = client.execute_tool("read_file", json!({"path": "/tmp/test.txt"})).await?;
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       Kelpie Server                          │
│  ┌────────────┐  ┌────────────┐  ┌────────────────────────┐ │
│  │  REST API  │  │ LLM Client │  │ Tool Registry          │ │
│  │   (Axum)   │  │(Claude/GPT)│  │ (Rust + MCP tools)     │ │
│  └─────┬──────┘  └─────┬──────┘  └───────────┬────────────┘ │
│        └───────────────┼─────────────────────┘              │
│                  ┌─────┴─────┐                              │
│                  │Agent Loop │ ← LLM ↔ Tools ↔ Memory      │
│                  └─────┬─────┘                              │
│  ┌─────────────────────┼─────────────────────────────────┐  │
│  │               Memory System                           │  │
│  │  [Core 32KB]    [Working KV]    [Archival Vectors]   │  │
│  └─────────────────────┼─────────────────────────────────┘  │
│                  ┌─────┴─────┐                              │
│                  │  Storage  │  (In-Memory / FDB)          │
│                  └───────────┘                              │
└─────────────────────────────────────────────────────────────┘
```

## Agent Capabilities

The agent framework is **~80% complete**. Core functionality works today:

| Capability | Status | Notes |
|------------|--------|-------|
| Agent loop with LLM | ✅ Working | Claude/GPT support |
| Tool execution | ✅ Working | Up to 5 chained calls |
| Memory blocks in context | ✅ Working | Core memory → system prompt |
| SSE streaming | ✅ Working | Real-time responses |
| Memory editing tools | ✅ Working | `core_memory_append`, `archival_search`, etc. |
| MCP tool integration | ✅ Working | Client ready, static config |
| Agent types (MemGPT/ReAct) | 🔜 Planned | Single behavior currently |
| Heartbeat/pause | 🔜 Planned | Fixed 5-iteration limit |

> **Current behavior:** All agents use a MemGPT-style loop with memory tools. Agent type selection (`memgpt_agent`, `react_agent`) is planned.

## Crates

| Crate | Description | Status |
|-------|-------------|--------|
| `kelpie-core` | Types, errors, constants | Complete |
| `kelpie-runtime` | Actor dispatcher | Complete |
| `kelpie-memory` | Memory hierarchy | Complete |
| `kelpie-storage` | KV storage (FDB backend) | Complete |
| `kelpie-vm` | VM backends (Vz/Firecracker) | Complete |
| `kelpie-sandbox` | Process isolation wrapper | Complete |
| `kelpie-tools` | MCP client, tool registry | Complete |
| `kelpie-cluster` | Node coordination | Complete |
| `kelpie-server` | REST API + agent loop | Complete |
| `kelpie-dst` | Simulation testing | Complete |
| `kelpie-agent` | Agent type abstractions | Stub (Phase 5) |
| `kelpie-wasm` | WASM actors | Planned |

## API Compatibility

Kelpie implements Letta-compatible REST endpoints with SSE streaming support:

```
GET  /health
GET  /v1/agents
POST /v1/agents
GET  /v1/agents/{id}
PATCH /v1/agents/{id}
DELETE /v1/agents/{id}
GET  /v1/agents/{id}/blocks
PATCH /v1/agents/{id}/blocks/{bid}
GET  /v1/agents/{id}/messages
POST /v1/agents/{id}/messages              # JSON or SSE (stream_steps=true)
POST /v1/agents/{id}/messages/stream       # SSE streaming
GET  /v1/blocks                            # Standalone blocks
POST /v1/blocks
GET  /v1/agents/{id}/core-memory/blocks/{label}  # Access by label
GET  /v1/tools                             # List tools
GET  /v1/mcp-servers                       # List MCP servers
POST /v1/mcp-servers                       # Connect new MCP server
```

### letta-code CLI

Use Kelpie as a drop-in replacement for Letta server with [letta-code](https://github.com/letta-ai/letta-code):

```bash
# Start Kelpie server
ANTHROPIC_API_KEY=sk-... cargo run -p kelpie-server

# Run letta-code pointing to Kelpie
LETTA_BASE_URL=http://localhost:8283 letta -p "help me write a function"
```

### Python SDK

Use with existing Letta clients:

```python
from letta_client import Letta

client = Letta(base_url="http://localhost:8283")
agent = client.agents.create(name="my-agent")
```

## Observability

Kelpie includes built-in metrics and distributed tracing for production monitoring.

### Metrics (Prometheus)

```bash
# Scrape metrics endpoint
curl http://localhost:8283/metrics

# Example metrics:
# - kelpie_agents_active_count
# - kelpie_invocations_total{operation, status}
# - kelpie_invocation_duration_seconds{operation}
# - kelpie_memory_usage_bytes{tier="core|working|archival"}
```

Add to `prometheus.yml`:
```yaml
scrape_configs:
  - job_name: 'kelpie'
    static_configs:
      - targets: ['localhost:8283']
    metrics_path: '/metrics'
```

### Distributed Tracing (OpenTelemetry)

```bash
# Run server with tracing
RUST_LOG=info cargo run -p kelpie-server

# Export to Jaeger/Zipkin/Tempo
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 \
RUST_LOG=info \
cargo run -p kelpie-server --features otel

# Start Jaeger all-in-one
docker run -d -p 4317:4317 -p 16686:16686 jaegertracing/all-in-one:latest
# View traces at http://localhost:16686
```

See [docs/observability/METRICS.md](docs/observability/METRICS.md) and [docs/observability/TRACING.md](docs/observability/TRACING.md) for complete reference.

## Testing

Kelpie uses Deterministic Simulation Testing (DST) for reliability:

```bash
# Run all tests
cargo test

# Run with specific seed for reproduction
DST_SEED=12345 cargo test -p kelpie-dst

# Run with fault injection
cargo test -p kelpie-dst -- --ignored
```

### Fault Types

- **Storage**: Write fail, read fail, corruption, latency
- **Network**: Partition, delay, packet loss, reordering
- **Crash**: Before write, after write, during transaction
- **Time**: Clock skew, clock jump

## Configuration

Environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `ANTHROPIC_API_KEY` | Anthropic API key | Required for Claude |
| `OPENAI_API_KEY` | OpenAI API key | Required for GPT |
| `KELPIE_HOST` | Server bind address | `0.0.0.0` |
| `KELPIE_PORT` | Server port | `8283` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OpenTelemetry endpoint | None |
| `RUST_LOG` | Log level | `info` |

## Storage

Kelpie supports two storage backends:

1. **In-Memory (Default)**: Fast, non-persistent. Data is lost on restart.
2. **FoundationDB**: Persistent, linearizable, distributed (enabled by default).

To use FoundationDB:

```bash
# Run with FDB cluster file
cargo run -p kelpie-server -- --fdb-cluster-file /path/to/fdb.cluster

# Or just use cargo build/test (FDB is now compiled by default)
cargo build
cargo test
```

## Roadmap

See [VISION.md](./VISION.md) for detailed roadmap.

**Next priorities:**
1. **Wire FDB to server** - Persistence (backend complete, integration pending)
2. **Agent types** - MemGPT, ReAct, LettaV1 behaviors
3. **Heartbeat mechanism** - Agent-controlled pause/resume
4. **Authentication** - API keys, rate limiting

## Engineering Principles

Kelpie follows **TigerStyle** (Safety > Performance > DX):

- Explicit constants with units: `TIMEOUT_MS_MAX`, `MEMORY_BYTES_LIMIT`
- Big-endian naming: `actor_state_size_bytes_max`
- 2+ assertions per function
- No silent truncation
- DST coverage for critical paths

See [CLAUDE.md](./CLAUDE.md) for development guidelines.

## Inspiration

- [NOLA](https://github.com/richardartoul/nola) - Go virtual actors
- [Letta](https://github.com/letta-ai/letta) - Stateful AI agents
- [FoundationDB](https://www.foundationdb.org/) - Simulation testing
- [TigerBeetle](https://github.com/tigerbeetle/tigerbeetle) - TigerStyle

## License

Apache-2.0
