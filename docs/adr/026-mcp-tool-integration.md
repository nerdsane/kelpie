# ADR-026: MCP Tool Integration

## Status

Accepted

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| MCP protocol support | 📋 Designed | - |
| Tool discovery | 📋 Designed | - |
| Execution sandbox | 📋 Designed | See ADR-027 |
| Result handling | 📋 Designed | - |

## Context

Kelpie agents need to interact with external tools and services. The Model Context Protocol (MCP) provides a standardized way for AI agents to:

1. **Discover Tools**: Find available tools and their capabilities
2. **Execute Tools**: Run tools with structured input/output
3. **Handle Results**: Process tool outputs and errors
4. **Maintain Context**: Share context between agent and tools

MCP is an emerging standard for AI-tool integration, providing better interoperability than custom protocols.

## Decision

Integrate MCP protocol with stdio transport for tool execution.

### Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    MCP Tool Integration                              │
│                                                                      │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐           │
│  │   Kelpie    │     │    MCP      │     │    Tool     │           │
│  │   Agent     │◀───▶│   Client    │◀───▶│   Server    │           │
│  └─────────────┘     └─────────────┘     └─────────────┘           │
│        │                   │                   │                    │
│        │                   │ stdio             │                    │
│        ▼                   ▼                   ▼                    │
│   ┌─────────┐        ┌─────────┐        ┌─────────┐               │
│   │ Request │───────▶│ JSON-RPC│───────▶│ Execute │               │
│   └─────────┘        └─────────┘        └─────────┘               │
│                                               │                    │
│                                               ▼                    │
│                                         ┌─────────┐               │
│                                         │ Sandbox │               │
│                                         │  (VM)   │               │
│                                         └─────────┘               │
│                                                                    │
└─────────────────────────────────────────────────────────────────────┘
```

### Key Design Points

1. **Transport: stdio**
   - Tools run as child processes with stdin/stdout communication
   - Simple, well-understood, works with existing tools
   - No network configuration needed

2. **Protocol: JSON-RPC over MCP**
   - Standard JSON-RPC 2.0 message format
   - MCP-defined methods: `initialize`, `tools/list`, `tools/call`
   - Bidirectional communication for streaming

3. **Tool Discovery**
   - On agent activation, discover available tools via `tools/list`
   - Cache tool schemas for fast access
   - Refresh periodically or on-demand

4. **Execution Model**
   - Each tool call creates a new sandbox context
   - Tools execute in isolated environment (see ADR-027)
   - Results returned as structured JSON

5. **Timeout and Error Handling**
   - Configurable timeout per tool (default: 30s)
   - Graceful handling of tool crashes
   - Structured error responses with error codes

### MCP Message Flow

```
Agent                    MCP Client                 Tool Server
  │                          │                           │
  │  invoke("shell", args)   │                           │
  │─────────────────────────▶│                           │
  │                          │  {"method": "tools/call"} │
  │                          │──────────────────────────▶│
  │                          │                           │
  │                          │  {"result": {...}}        │
  │                          │◀──────────────────────────│
  │  ToolResult              │                           │
  │◀─────────────────────────│                           │
  │                          │                           │
```

### Tool Schema Example

```json
{
  "name": "shell",
  "description": "Execute shell commands",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": {
        "type": "string",
        "description": "The shell command to execute"
      },
      "timeout_ms": {
        "type": "integer",
        "description": "Timeout in milliseconds",
        "default": 30000
      }
    },
    "required": ["command"]
  }
}
```

### Result Handling

Tool results are structured with content types:

```rust
enum ToolContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: Option<String> },
}

struct ToolResult {
    content: Vec<ToolContent>,
    is_error: bool,
}
```

### Security Considerations

1. **Tool Allowlist**: Only whitelisted tools can be executed
2. **Sandbox Isolation**: Tools run in sandboxed environment (ADR-027)
3. **Input Validation**: Tool inputs validated against schema
4. **Output Sanitization**: Tool outputs sanitized before use

## Consequences

### Positive

- **Standard Protocol**: MCP is gaining adoption, tools are reusable
- **Extensibility**: Easy to add new tools without code changes
- **Isolation**: stdio transport provides natural process isolation
- **Streaming**: Supports streaming responses for long-running tools

### Negative

- **Process Overhead**: Each tool call spawns a process (mitigated by pooling)
- **Serialization Cost**: JSON serialization for all communication
- **Protocol Complexity**: MCP adds abstraction layer vs. direct calls

### Neutral

- MCP is evolving; may need to track protocol changes
- stdio is simple but limits to local tools (can extend to HTTP later)

## Alternatives Considered

### Custom Protocol

- Design Kelpie-specific tool protocol
- Optimize for our use cases

**Rejected because**: Reinventing the wheel. MCP provides standardization and ecosystem benefits.

### OpenAI Function Calling Format

- Use OpenAI's function calling schema
- Compatible with many LLM providers

**Rejected because**: Less flexible than MCP. No bidirectional communication. Vendor-specific origins.

### Direct SDK Integration

- Call tool SDKs directly from Rust
- No process overhead

**Rejected because**: Tight coupling. Each tool needs Rust bindings. Harder to extend.

### HTTP/WebSocket Transport

- Tools as HTTP services
- Network-native communication

**Rejected because**: More complex setup. Security concerns with network exposure. stdio is simpler for local tools.

## References

- [Model Context Protocol](https://github.com/anthropics/anthropic-cookbook/tree/main/mcp) - Protocol specification
- [ADR-027: Sandbox Execution Design](./027-sandbox-execution-design.md) - Sandbox integration
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification) - Base protocol
