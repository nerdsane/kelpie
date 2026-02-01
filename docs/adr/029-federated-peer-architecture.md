# ADR-029: Federated Peer Architecture

## Status

Proposed

## Date

2026-01-29

## Context

Kelpie and RikaiOS aim to support a personal AI assistant that:

1. **Multi-agent per user**: Each user can have multiple agents (personal, work, research)
2. **Agent-to-agent within user**: Agents collaborate locally via `call_agent` (ADR-028)
3. **Agent-to-agent across users**: User A's agent collaborates with User B's agent
4. **User owns their data**: "You own your context" - no centralized data store
5. **Decentralized operation**: No single point of failure

The existing `call_agent` mechanism (ADR-028) works within a single Kelpie instance. Federation extends this to work across independent Kelpie nodes operated by different users.

### Security Lessons from Moltbot/Clawdbot Incidents

Recent security incidents with similar AI assistant platforms inform our design:

| Incident | What Happened | Our Mitigation |
|----------|---------------|----------------|
| 1000+ exposed instances | No auth by default | Auth required, localhost-only by default |
| Prompt injection via email | Malicious content → agent action | Input sanitization, human approval for sensitive ops |
| Command injection (CVE-2025-6514) | CVSS 9.6 | VM sandbox isolation |
| Supply chain attack | Poisoned skills | No external skill loading without approval |

### Architectural Options Considered

We evaluated four architectural approaches:

| Option | Description | Trade-offs |
|--------|-------------|------------|
| **1. Central Service** | Kelpie as shared server, RikaiOS as thin client | ❌ Single point of failure, ❌ Centralized data |
| **2. Library Only** | Kelpie embedded in RikaiOS, no federation | ✅ Simple, ❌ No cross-user collaboration |
| **3. Fused + Manual Federation** | Library approach + custom federation per app | ⚠️ Inconsistent implementations |
| **4. Federated Peers** | Library approach + federation built into Kelpie | ✅ Decentralized, ✅ Consistent protocol |

## Decision

We adopt **Federated Peer Architecture** where:

1. **Each user runs their own RikaiOS+Kelpie node**
2. **Kelpie provides federation protocol** for cross-node agent communication
3. **RikaiOS remains the application layer** (Telegram, UX, proposals)
4. **Kelpie remains the infrastructure layer** (agents, tools, storage, federation)

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                     FEDERATED PEER ARCHITECTURE                              │
│                                                                              │
│  User A's Node                              User B's Node                    │
│  ┌───────────────────────────┐             ┌───────────────────────────┐    │
│  │ RikaiOS (application)     │             │ RikaiOS (application)     │    │
│  │ • Telegram interface      │             │ • Telegram interface      │    │
│  │ • User authentication     │             │ • User authentication     │    │
│  │ • Proposal UX             │             │ • Proposal UX             │    │
│  └─────────────┬─────────────┘             └─────────────┬─────────────┘    │
│                │ uses                                    │ uses             │
│                ▼                                         ▼                  │
│  ┌───────────────────────────┐             ┌───────────────────────────┐    │
│  │ Kelpie (infrastructure)   │             │ Kelpie (infrastructure)   │    │
│  │ • Agent runtime           │             │ • Agent runtime           │    │
│  │ • Tool registry           │             │ • Tool registry           │    │
│  │ • Storage (FDB)           │             │ • Storage (FDB)           │    │
│  │ • Federation layer ───────┼─────────────┼─► Federation layer        │    │
│  │   (Kelpie-to-Kelpie)      │   QUIC/TLS  │   (Kelpie-to-Kelpie)      │    │
│  └───────────────────────────┘             └───────────────────────────┘    │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Responsibility Separation

| Layer | Component | Responsibilities |
|-------|-----------|------------------|
| **Application** | RikaiOS | User interface (Telegram, CLI), user auth, proposal approval UX, API key management |
| **Infrastructure** | Kelpie | Agent lifecycle, memory management, tool registry, MCP client, VM sandboxing, **federation protocol** |

### Federation Layer Design

#### 1. Extended Agent Addressing

Extend agent IDs to support remote nodes:

```
Local:  agent-abc123
Remote: user-bob@node-xyz.example.com/agent-def456
        └──────────┬──────────────────┘ └────┬────┘
              Node identifier           Agent ID
```

#### 2. Extended `call_agent` for Federation

```rust
// call_agent with federation support
pub async fn call_agent(
    ctx: &ToolExecutionContext,
    target: &str,           // "agent-id" or "node/agent-id"
    message: &str,
    timeout_ms: Option<u64>,
) -> ToolResult<String> {
    if is_remote_agent(target) {
        // Route through federation layer
        let (node, agent_id) = parse_remote_target(target)?;
        ctx.federation
            .call_remote_agent(node, agent_id, message, timeout_ms)
            .await
    } else {
        // Local call (existing ADR-028 behavior)
        ctx.dispatcher
            .invoke(target, "handle_message", message)
            .await
    }
}
```

#### 3. Federation Protocol Messages

```rust
/// Federation protocol message types
#[derive(Serialize, Deserialize)]
pub enum FederationMessage {
    /// Request to call an agent on this node
    AgentCallRequest {
        request_id: String,
        source_node: NodeId,
        source_agent: String,
        target_agent: String,
        message: String,
        call_chain: Vec<(NodeId, String)>,  // For cycle detection
        timeout_ms: u64,
    },

    /// Response to an agent call
    AgentCallResponse {
        request_id: String,
        result: Result<String, FederationError>,
    },

    /// Discovery: list available agents
    ListAgentsRequest {
        request_id: String,
        filter: Option<AgentFilter>,
    },

    /// Discovery response
    ListAgentsResponse {
        request_id: String,
        agents: Vec<PublicAgentInfo>,
    },

    /// Health check / keepalive
    Ping { timestamp_ms: u64 },
    Pong { timestamp_ms: u64 },
}
```

#### 4. Security Model

```rust
/// Federation security configuration
pub struct FederationSecurityConfig {
    /// Nodes allowed to connect (allowlist)
    pub allowed_nodes: Vec<NodeId>,

    /// Agents that can be called remotely (default: none)
    pub public_agents: Vec<String>,

    /// Require mutual TLS
    pub require_mtls: bool,

    /// Maximum calls per minute from remote nodes
    pub rate_limit_per_node: u32,
}

impl Default for FederationSecurityConfig {
    fn default() -> Self {
        Self {
            allowed_nodes: vec![],      // No one by default
            public_agents: vec![],      // No agents exposed by default
            require_mtls: true,
            rate_limit_per_node: 60,
        }
    }
}
```

#### 5. Cross-Node Cycle Detection

Extend ADR-028's cycle detection to work across nodes:

```rust
// Call chain includes node + agent
type CallChain = Vec<(NodeId, AgentId)>;

fn detect_cycle(chain: &CallChain, target_node: &NodeId, target_agent: &AgentId) -> bool {
    chain.iter().any(|(node, agent)| {
        node == target_node && agent == target_agent
    })
}

// TigerStyle constants
const FEDERATION_CALL_DEPTH_MAX: u32 = 3;  // Lower than local (5)
const FEDERATION_CALL_TIMEOUT_MS_DEFAULT: u64 = 60_000;
const FEDERATION_CALL_TIMEOUT_MS_MAX: u64 = 300_000;
```

#### 6. Transport Layer

```rust
/// Federation transport options
pub enum FederationTransport {
    /// QUIC with TLS 1.3 (recommended)
    Quic {
        bind_addr: SocketAddr,
        cert_path: PathBuf,
        key_path: PathBuf,
    },

    /// Via Tailscale network (simpler setup)
    Tailscale {
        tailnet: String,
    },

    /// For testing
    InMemory,
}
```

### Discovery Mechanisms

#### Option A: Manual Configuration

```toml
# ~/.rikai/federation.toml
[[peers]]
name = "bob"
node_id = "node-xyz"
address = "bob.tailnet.ts.net:8285"
public_key = "..."

[[peers]]
name = "alice"
node_id = "node-abc"
address = "alice.example.com:8285"
public_key = "..."
```

#### Option B: Discovery Service (Optional)

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  User A's Node  │     │   Discovery     │     │  User B's Node  │
│                 │────▶│   Service       │◀────│                 │
│                 │     │  (optional)     │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
                              │
                              ▼
                        Returns peer list
                        (address, public key)
```

#### Option C: DHT-Based Discovery (Future)

Use libp2p Kademlia DHT for fully decentralized discovery.

### Implementation Phases

| Phase | Scope | Components |
|-------|-------|------------|
| **Phase 1** | Manual federation | QUIC transport, manual peer config, `call_agent` extension |
| **Phase 2** | Security hardening | mTLS, rate limiting, audit logging |
| **Phase 3** | Discovery service | Optional central discovery |
| **Phase 4** | Full decentralization | DHT-based discovery, no central services |

## Consequences

### Positive

- **User Ownership**: Each user runs their own node, owns their data
- **No Single Point of Failure**: Decentralized, no central server
- **Privacy**: Agent-to-agent calls are direct (no intermediary)
- **Builds on Existing Work**: Extends ADR-028's `call_agent`, reuses dispatcher
- **Incremental Adoption**: Users can start local-only, add federation later
- **RikaiOS Vision Alignment**: "You own your context"

### Negative

- **Network Complexity**: NAT traversal, firewalls, dynamic IPs
- **Key Management**: Users must exchange public keys (or trust discovery service)
- **Latency**: Remote calls add network round-trips
- **Availability**: Remote agents may be offline
- **Implementation Effort**: Significant new code (~2000 LOC estimated)

### Neutral

- **Tailscale Simplifies Networking**: Recommended for most users
- **Manual Config for MVP**: Discovery service is optional
- **Security-First Default**: No federation until explicitly configured

## Alternatives Considered

### Alternative 1: Central Relay Service

All cross-user communication routes through a central server:

```
User A ──▶ Central Relay ──▶ User B
```

**Rejected because**:
- Single point of failure
- Privacy concerns (relay sees all traffic)
- Operational burden (someone must run the relay)
- Contradicts "you own your context" principle

### Alternative 2: Kelpie as Multi-Tenant Service

Run Kelpie as a shared service, multiple users connect:

```
User A ──┐
         ├──▶ Shared Kelpie Server
User B ──┘
```

**Rejected because**:
- Centralized data storage
- Trust issues (who operates the server?)
- Doesn't scale to "everyone runs their own"
- More like traditional SaaS than personal computing

### Alternative 3: Federation at RikaiOS Layer

Keep Kelpie local-only, build federation into RikaiOS:

```
RikaiOS A ◀──federation──▶ RikaiOS B
    │                          │
    ▼                          ▼
Kelpie A (local)          Kelpie B (local)
```

**Rejected because**:
- Agent-to-agent is Kelpie's domain, not RikaiOS
- Would duplicate `call_agent` logic
- Other apps using Kelpie wouldn't get federation
- Kelpie already has actor model suited for this

### Alternative 4: Matrix Protocol

Use Matrix for federation (already federated, E2E encrypted):

**Rejected because**:
- Heavy dependency (Matrix homeserver)
- Designed for chat, not RPC
- Overkill for agent-to-agent calls
- Would add ~50MB to binary size

## References

- [ADR-028: Multi-Agent Communication](./028-multi-agent-communication.md) - Local `call_agent`
- [ADR-001: Virtual Actor Model](./001-virtual-actor-model.md) - Actor foundations
- [ADR-027: Sandbox Execution Design](./027-sandbox-execution-design.md) - Security model
- [libp2p](https://libp2p.io/) - P2P networking library
- [QUIC Protocol](https://quicwg.org/) - Transport layer
- [Tailscale](https://tailscale.com/) - Simplified networking option
- [Moltbot Security Analysis](https://www.bitdefender.com/en-us/blog/hotforsecurity/moltbot-security-alert) - Security lessons
