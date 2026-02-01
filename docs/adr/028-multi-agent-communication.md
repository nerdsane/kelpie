# ADR-028: Multi-Agent Communication

## Status

Accepted

## Date

2026-01-28

## Context

Kelpie needs to support agent-to-agent communication for multi-agent workflows. Use cases include:

1. **Delegation**: A coordinator agent delegates subtasks to specialist agents
2. **Orchestration**: A supervisor agent manages a team of worker agents
3. **Research**: An agent queries a knowledge-specialist agent for information
4. **Collaboration**: Multiple agents work together on a complex task

The challenge is implementing this communication while maintaining:
- **Single Activation Guarantee (SAG)**: Each agent can only be active on one node
- **Deadlock Prevention**: Circular calls (A→B→A) must not cause hangs
- **Bounded Resources**: Call depth and pending calls must be limited
- **Fault Tolerance**: Network failures and timeouts must be handled gracefully
- **DST Testability**: All behavior must be deterministically simulatable

## Decision

We implement agent-to-agent communication through a **tool-based approach** with the following design:

### 1. `call_agent` Tool

Add a new built-in tool that LLMs can use to invoke other agents:

```json
{
  "name": "call_agent",
  "description": "Call another agent and wait for their response",
  "parameters": {
    "agent_id": {
      "type": "string",
      "description": "The ID of the agent to call"
    },
    "message": {
      "type": "string",
      "description": "The message to send to the agent"
    },
    "timeout_ms": {
      "type": "integer",
      "description": "Optional timeout in milliseconds (default: 30000, max: 300000)"
    }
  }
}
```

### 2. Safety Mechanisms

#### 2.1 Cycle Detection

Every call carries a `call_chain` (sequence of caller agent IDs). Before processing a call, we check if the target is already in the chain:

```rust
// TigerStyle constants
const AGENT_CALL_DEPTH_MAX: u32 = 5;

// In call_agent tool
if ctx.call_chain.contains(&target_id) {
    return Err(Error::CycleDetected {
        caller: ctx.agent_id.clone(),
        target: target_id.clone(),
        chain: ctx.call_chain.clone(),
    });
}
```

#### 2.2 Depth Limiting

Call depth is tracked and bounded:

```rust
if ctx.call_depth >= AGENT_CALL_DEPTH_MAX {
    return Err(Error::CallDepthExceeded {
        depth: ctx.call_depth,
        max_depth: AGENT_CALL_DEPTH_MAX,
    });
}
```

#### 2.3 Timeout Handling

All cross-agent calls have a timeout (configurable, bounded):

```rust
// TigerStyle constants
const AGENT_CALL_TIMEOUT_MS_DEFAULT: u64 = 30_000;
const AGENT_CALL_TIMEOUT_MS_MAX: u64 = 300_000;

let timeout = input.timeout_ms
    .unwrap_or(AGENT_CALL_TIMEOUT_MS_DEFAULT)
    .min(AGENT_CALL_TIMEOUT_MS_MAX);

time_provider.timeout(
    Duration::from_millis(timeout),
    dispatcher.invoke(actor_id, "handle_message_full", payload)
).await
```

### 3. Extended ToolExecutionContext

The `ToolExecutionContext` is extended to support agent calls:

```rust
pub struct ToolExecutionContext {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    // New fields for multi-agent support
    pub dispatcher: Option<Arc<dyn AgentDispatcher>>,
    pub call_depth: u32,
    pub call_chain: Vec<String>,
    pub time_provider: Arc<dyn TimeProvider>,
}
```

### 4. TLA+ Verified Invariants

The protocol is formally verified in `KelpieMultiAgentInvocation.tla`:

| Invariant | Description |
|-----------|-------------|
| `NoDeadlock` | No agent appears twice in any call stack |
| `SingleActivationDuringCall` | At most one node hosts each agent |
| `DepthBounded` | All call stacks ≤ MAX_DEPTH |
| `BoundedPendingCalls` | Pending calls ≤ Agents × MAX_DEPTH |

### 5. DST Test Coverage

The following scenarios are tested with fault injection:

1. `test_agent_calls_agent_success` - Basic A→B call works
2. `test_agent_call_cycle_detection` - A→B→A rejected immediately
3. `test_agent_call_timeout` - Slow agent triggers timeout
4. `test_agent_call_depth_limit` - Chain exceeding depth fails
5. `test_agent_call_under_network_partition` - Graceful failure
6. `test_single_activation_during_cross_call` - SAG maintained
7. `test_agent_call_with_storage_faults` - Fault tolerance
8. `test_determinism_multi_agent` - Same seed = same result

## Consequences

### Positive

- **Minimal Changes**: Leverages existing dispatcher infrastructure
- **LLM Control**: The LLM decides when to call other agents (transparent)
- **Verifiable**: TLA+ spec and DST tests provide strong guarantees
- **Fail-Fast**: Cycles and depth violations detected immediately
- **Observable**: Call chains logged for debugging

### Negative

- **Latency**: Cross-agent calls add network round-trips
- **No Streaming**: Responses are complete (no streaming in v1)
- **JSON Overhead**: All payloads serialized through JSON

### Neutral

- **Timeout Required**: All calls must have bounded timeout
- **Explicit IDs**: Callers must know target agent IDs

## Alternatives Considered

### Alternative 1: Service-Based Orchestration

Add an orchestration layer above AgentService:

```rust
struct OrchestrationService {
    agent_service: AgentService,
    task_graph: TaskGraph,
}
```

**Rejected because**: Adds complexity when dispatcher infrastructure already exists. Would require significant new code (~500 LOC) versus tool-based approach (~200 LOC).

### Alternative 2: Native Runtime Messaging

Add actor mailbox for agent-to-agent messages with typed message passing:

```rust
impl AgentActor {
    async fn send(&self, target: ActorId, message: AgentMessage) -> Response;
}
```

**Rejected because**: Breaks virtual actor model simplicity (no persistent mailboxes), requires major runtime changes, and would need separate DST harness.

### Alternative 3: Event-Based Communication

Use pub/sub events for agent communication:

```rust
event_bus.publish("agent.research.complete", result);
event_bus.subscribe("agent.research.*", handler);
```

**Rejected because**: Too loosely coupled for request/response patterns, harder to trace call chains for debugging, and doesn't support synchronous workflows well.

## Implementation Details

### Files to Create/Modify

| File | Action | Description |
|------|--------|-------------|
| `tools/agent_call.rs` | CREATE | `call_agent` tool implementation |
| `tools/mod.rs` | MODIFY | Export agent_call, extend context |
| `tools/registry.rs` | MODIFY | Thread context to handlers |
| `dispatcher.rs` | MODIFY | Add `invoke_with_timeout()` |

### DST Harness Extension

New fault types added to `kelpie-dst/src/fault.rs`:

```rust
pub enum FaultType {
    // ... existing ...

    // Multi-agent specific
    AgentCallTimeout,      // Called agent doesn't respond
    AgentCallRejected,     // Called agent refuses call
    AgentNotFound,         // Target agent doesn't exist
    AgentBusy,             // Target at max concurrent calls
    AgentCallNetworkDelay, // Network delay on agent calls
}
```

## References

- [TLA+ Spec](../tla/KelpieMultiAgentInvocation.tla) - Formal verification
- [ADR-001](./001-virtual-actor-model.md) - Virtual Actor Model
- [ADR-005](./005-dst-framework.md) - DST Framework
- [ADR-013](./013-actor-based-agent-server.md) - Actor-Based Agent Server
- [Issue #75](https://github.com/kelpie/issues/75) - Original feature request
