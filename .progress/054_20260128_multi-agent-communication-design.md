# Multi-Agent Communication Design (Issue #75)

**Created:** 2026-01-28
**Status:** COMPLETE
**Completed:** 2026-01-28
**Issue:** https://github.com/kelpie/issues/75
**Related:** ADR-001, ADR-013, ADR-014, CONSTRAINTS.md

---

## Executive Summary

This plan designs agent-to-agent communication for Kelpie, following FoundationDB's DST-first methodology. The key insight from research: **the infrastructure is 90% there** - AgentActor already holds a DispatcherHandle. The gaps are safety mechanisms, tools, and DST coverage.

---

## Research Findings (Verified via RLM Analysis)

### What EXISTS (Contrary to Issue Assumptions)

| Component | Status | Location | Evidence |
|-----------|--------|----------|----------|
| AgentActor has DispatcherHandle | ✅ | `agent_actor.rs:27-29` | `dispatcher: Option<DispatcherHandle>` |
| Builder method for dispatcher | ✅ | `agent_actor.rs:40-46` | `with_dispatcher()` |
| Dispatcher cross-actor invoke | ✅ | `dispatcher.rs:138-180` | `invoke(actor_id, operation, payload)` |
| Backpressure mechanism | ✅ | `dispatcher.rs:145-152` | `PendingGuard` with per-actor limits |
| Distributed forwarding | ✅ | `dispatcher.rs:426-448` | Forwards to remote nodes |
| Tool registry extensible | ✅ | `tools/registry.rs` | `register_builtin()` with custom handlers |

### What's MISSING (Actual Gaps from RLM Analysis)

| Gap | Impact | Location | Priority |
|-----|--------|----------|----------|
| `call_agent` tool | Agents can't invoke other agents | `tools/` | P0 |
| Dispatcher in ToolExecutionContext | Tools can't access dispatcher | `tools/registry.rs` | P0 |
| Cycle detection | A→B→A infinite loop | `dispatcher.rs` | P0 |
| Timeout on invoke | Stuck calls block forever | `dispatcher.rs:178` | P0 |
| Call depth tracking | No recursion limit | - | P0 |
| TLA+ spec | No formal verification | `docs/tla/` | P1 |
| DST test coverage | No fault injection tests | `tests/` | P0 |

### Critical Finding: Deadlock Risk

**RLM Analysis identified a major deadlock scenario:**

```
Agent A → calls Agent B (waits on reply_rx line 178)
Agent B → calls Agent A (waits on reply_rx line 178)
Dispatcher → single-threaded event loop (line 288)
Result: DEADLOCK - both blocked on oneshot channels
```

The single-threaded dispatcher (`while let Some(command) = self.command_rx.recv().await`) processes commands sequentially. If handler A waits for B's response, and B's handler waits for A, both block forever.

### ToolExecutionContext Gap

Current context (from RLM):
```rust
pub struct ToolExecutionContext {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    // NO DISPATCHER - tools cannot call other agents!
}
```

### kelpie-agent Crate Status

**CONFIRMED STUB** - Only contains:
```rust
pub struct Agent;  // Placeholder
// pub mod orchestrator;  // Commented out
```

The multi-agent orchestration was planned but never implemented.

---

## Options Analysis

### Option 1: Tool-Based (`call_agent` tool) ✅ RECOMMENDED

**Approach:** Add `call_agent(agent_id, message)` tool that LLM can invoke.

```rust
// tools/agent_call.rs
async fn call_agent(input: &Value, ctx: &ToolExecutionContext) -> String {
    let target_id = input.get("agent_id")?.as_str()?;
    let message = input.get("message")?.as_str()?;

    let actor_id = ActorId::new("agents", target_id)?;
    let payload = HandleMessageFullRequest { content: message.to_string() };

    dispatcher.invoke(actor_id, "handle_message_full", payload).await
}
```

**Pros:**
- Minimal code changes (~200 LOC)
- Uses existing infrastructure
- LLM decides when to call other agents
- Transparent - call chain visible in tool calls

**Cons:**
- No compile-time type safety
- Requires cycle detection
- Timeout handling needed

**Trade-offs accepted:**
- Async-only (no sync calls) - acceptable for AI agents
- JSON serialization overhead - negligible for agent workloads

### Option 2: Service-Based Orchestration

**Approach:** Add orchestration layer above AgentService.

```rust
struct OrchestrationService {
    agent_service: AgentService,
    task_graph: TaskGraph,
}
```

**Pros:**
- Centralized control flow
- Easier to implement workflows
- Can optimize call patterns

**Cons:**
- New abstraction layer (+500 LOC)
- Doesn't leverage existing dispatcher
- Over-engineering for initial needs

**Rejected because:** Adds complexity when infrastructure already exists.

### Option 3: Native Runtime Messaging

**Approach:** Add actor mailbox for agent-to-agent messages.

**Pros:**
- Type-safe messages
- Efficient (no serialization)

**Cons:**
- Major runtime changes
- Breaks virtual actor simplicity
- Requires new DST harness

**Rejected because:** Virtual actor model intentionally avoids mailboxes.

---

## Chosen Approach: Tool-Based with Safety Mechanisms

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Agent A (LLM)                           │
│  "I need help from the research agent"                         │
│  → calls tool: call_agent("research-agent", "find papers on X")│
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                     call_agent Tool                              │
│  1. Validate target agent exists (registry lookup)              │
│  2. Check call depth (cycle detection)                          │
│  3. Set timeout (configurable, default 30s)                     │
│  4. Invoke via dispatcher                                        │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Dispatcher                                  │
│  invoke(ActorId::new("agents", "research-agent"),               │
│         "handle_message_full",                                   │
│         payload)                                                 │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Agent B (research-agent)                      │
│  Receives message, processes, returns response                   │
│  Can itself call other agents (up to depth limit)               │
└─────────────────────────────────────────────────────────────────┘
```

### Required Code Changes (from RLM Analysis)

#### 1. Extend ToolExecutionContext

```rust
// crates/kelpie-server/src/tools/mod.rs
pub struct ToolExecutionContext {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub dispatcher: Option<Arc<dyn AgentDispatcher>>,  // ADD
    pub call_depth: u32,                                // ADD
    pub call_chain: Vec<String>,                        // ADD
}
```

#### 2. Modify BuiltinToolHandler Signature

```rust
// Current: only receives input
pub type BuiltinToolHandler = Arc<
    dyn Fn(&Value) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync
>;

// New: receives input AND context
pub type BuiltinToolHandler = Arc<
    dyn Fn(&Value, Option<&ToolExecutionContext>)
        -> Pin<Box<dyn Future<Output = ToolExecutionResult> + Send>> + Send + Sync
>;
```

#### 3. Add invoke_with_timeout to DispatcherHandle

```rust
// crates/kelpie-runtime/src/dispatcher.rs
impl<R: kelpie_core::Runtime> DispatcherHandle<R> {
    pub async fn invoke_with_timeout(
        &self,
        actor_id: ActorId,
        operation: String,
        payload: Bytes,
        timeout: Duration,
    ) -> Result<Bytes> {
        // ... existing backpressure check ...

        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Internal {
                message: "reply channel closed".into(),
            }),
            Err(_) => Err(Error::Timeout {
                operation: format!("invoke {} {}", actor_id, operation),
                timeout_ms: timeout.as_millis() as u64,
            }),
        }
    }
}
```

### Safety Mechanisms (CRITICAL)

#### 1. Call Depth Limit (Cycle Prevention)

```rust
const AGENT_CALL_DEPTH_MAX: u32 = 5;

struct CallContext {
    depth: u32,
    call_chain: Vec<String>,  // ["agent-a", "agent-b", ...]
}

// Passed in ToolExecutionContext
if ctx.call_depth >= AGENT_CALL_DEPTH_MAX {
    return Err("Maximum call depth exceeded");
}
if ctx.call_chain.contains(&target_agent_id) {
    return Err(format!("Cycle detected: {} already in call chain", target_agent_id));
}
```

#### 2. Timeout Handling

```rust
const AGENT_CALL_TIMEOUT_MS_DEFAULT: u64 = 30_000;  // 30 seconds
const AGENT_CALL_TIMEOUT_MS_MAX: u64 = 300_000;     // 5 minutes

// In call_agent tool
let timeout = input.get("timeout_ms")
    .and_then(|v| v.as_u64())
    .unwrap_or(AGENT_CALL_TIMEOUT_MS_DEFAULT)
    .min(AGENT_CALL_TIMEOUT_MS_MAX);

tokio::time::timeout(
    Duration::from_millis(timeout),
    dispatcher.invoke(...)
).await
```

#### 3. Backpressure

Already exists in dispatcher (`max_pending_per_actor`). Extend to cross-agent:

```rust
const AGENT_CONCURRENT_CALLS_MAX: usize = 10;  // Per calling agent
```

---

## DST Strategy (MANDATORY - Per CONSTRAINTS.md)

### TLA+ Specification: KelpieMultiAgentInvocation.tla

**Must define before implementation:**

```tla
------------------------------ MODULE KelpieMultiAgentInvocation ------------------------------
CONSTANTS Agents, MAX_DEPTH, TIMEOUT_MS

VARIABLES
    callStack,      \* Per-agent call stack
    callState,      \* Pending | Completed | TimedOut | Failed
    agentState      \* Running | Waiting | Paused

(* SAFETY INVARIANTS *)

\* No circular calls in any call stack
NoDeadlock ==
    \A a \in Agents:
        LET stack == callStack[a]
        IN Cardinality(ToSet(stack)) = Len(stack)

\* Single activation maintained during cross-agent calls
SingleActivationDuringCall ==
    \A a \in Agents:
        Cardinality({n \in Nodes : agentState[n][a] = "Running"}) <= 1

\* Call depth never exceeds limit
DepthBounded ==
    \A a \in Agents: Len(callStack[a]) <= MAX_DEPTH

(* LIVENESS PROPERTIES *)

\* Every call eventually completes (or times out)
CallEventuallyCompletes ==
    \A call \in Calls:
        callState[call] = "Pending" ~>
            callState[call] \in {"Completed", "TimedOut", "Failed"}

=============================================================================
```

### DST Test Cases (Write BEFORE Implementation)

```rust
// crates/kelpie-server/tests/multi_agent_dst.rs

#[madsim::test]
async fn test_agent_calls_agent_success() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .run_async(|env| async move {
            // Create agent A and agent B
            let agent_a = create_test_agent(&env, "agent-a").await;
            let agent_b = create_test_agent(&env, "agent-b").await;

            // Agent A calls Agent B
            let response = agent_a.call_agent("agent-b", "Hello from A").await?;

            assert!(response.contains("response from agent-b"));
            Ok(())
        })
        .await
        .expect("Agent-to-agent call should succeed");
}

#[madsim::test]
async fn test_agent_call_cycle_detection() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .run_async(|env| async move {
            // Create agents that would create a cycle
            let agent_a = create_agent_that_calls(&env, "agent-a", "agent-b").await;
            let agent_b = create_agent_that_calls(&env, "agent-b", "agent-a").await;

            // Trigger the cycle
            let result = agent_a.send_message("start cycle").await;

            // Should fail with cycle detection, not hang
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("cycle detected"));
            Ok(())
        })
        .await
        .expect("Cycle should be detected");
}

#[madsim::test]
async fn test_agent_call_timeout() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkDelay, 0.5))  // 50% delayed
        .run_async(|env| async move {
            let agent_a = create_test_agent(&env, "agent-a").await;
            let agent_b = create_slow_agent(&env, "agent-b", 60_000).await;  // 60s response

            // Call with 1s timeout
            let result = agent_a.call_agent_with_timeout("agent-b", "hello", 1000).await;

            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("timeout"));
            Ok(())
        })
        .await
        .expect("Timeout should be handled");
}

#[madsim::test]
async fn test_agent_call_under_network_partition() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.3))
        .run_async(|env| async move {
            // Test that calls fail gracefully under partition
            // Not hang or corrupt state
            Ok(())
        })
        .await
        .expect("Should handle partition gracefully");
}

#[madsim::test]
async fn test_agent_call_depth_limit() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .run_async(|env| async move {
            // Create chain: A → B → C → D → E → F (depth 5)
            // F trying to call G should fail

            // Verify depth limit enforced
            Ok(())
        })
        .await
        .expect("Depth limit should be enforced");
}

#[madsim::test]
async fn test_single_activation_during_cross_call() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .with_registry()  // Use distributed registry
        .run_async(|env| async move {
            // Verify that during A calling B:
            // - Both A and B maintain single activation
            // - No race conditions in placement
            Ok(())
        })
        .await
        .expect("Single activation must hold during calls");
}
```

### New Fault Types Needed

```rust
// crates/kelpie-dst/src/faults.rs

pub enum FaultType {
    // ... existing faults ...

    // New for multi-agent
    AgentCallTimeout,           // Called agent doesn't respond
    AgentCallRejected,          // Called agent refuses call
    AgentNotFound,              // Target agent doesn't exist
    AgentBusy,                  // Target agent at max concurrent calls
}
```

---

## Implementation Phases

### Phase 0: TLA+ Specification (Day 1)
- [x] Write `KelpieMultiAgentInvocation.tla`
- [x] Model check with TLC
- [x] Document invariants for DST alignment

### Phase 1: DST Harness Extension (Day 1-2)
- [x] Add `AgentCallTimeout`, `AgentCallRejected` fault types
- [x] 8 DST tests written and passing

### Phase 2: Safety Mechanisms (Day 2-3)
- [x] Implement `CallContext` with depth tracking
- [x] Add cycle detection in call chain
- [x] Add to `ToolExecutionContext`

### Phase 3: call_agent Tool (Day 3-4)
- [x] Create `tools/agent_call.rs`
- [x] Register tool with TigerStyle constants
- [x] Validation functions for cycle/depth checking

### Phase 4: Verification (Day 4-5)
- [x] All DST tests pass
- [x] Determinism verified (same seed = same result)
- [x] TLA+ invariants align with DST assertions
- [x] No clippy warnings, fmt passes

### Phase 5: Documentation & ADR (Day 5)
- [x] Write ADR-028: Multi-Agent Communication
- [x] TLA+ spec with TLC output saved

---

## What to Try (After Each Phase)

### After Phase 1 (DST Harness)
**Works Now:** Run DST tests - they should compile and fail with "not implemented"
```bash
cargo test -p kelpie-server --test multi_agent_dst -- --nocapture
```
**Doesn't Work Yet:** Actual agent-to-agent calls
**Known Limitations:** None

### After Phase 3 (call_agent Tool)
**Works Now:**
```bash
# Create two agents
curl -X POST http://localhost:8283/v1/agents -d '{"name": "helper"}'
curl -X POST http://localhost:8283/v1/agents -d '{"name": "coordinator"}'

# Coordinator calls helper (via LLM tool use)
curl -X POST http://localhost:8283/v1/agents/{coordinator-id}/messages \
  -d '{"content": "Ask the helper agent for assistance"}'
```
**Doesn't Work Yet:** Supervisor/worker patterns, shared memory
**Known Limitations:** Max depth 5, max timeout 5 minutes

---

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| 2026-01-28 | Tool-based over service-based | Infrastructure exists, minimal changes | Less centralized control |
| 2026-01-28 | Depth limit of 5 | Prevents runaway recursion | May limit complex workflows |
| 2026-01-28 | 30s default timeout | Balance responsiveness vs agent thinking time | May timeout slow agents |
| 2026-01-28 | TLA+ spec first | FoundationDB best practice | Upfront time investment |
| 2026-01-28 | No shared memory in v1 | Complexity, can add later | Teams can't share context |

---

## Out of Scope (Future Work)

- **Shared memory between agents** - Requires new storage design
- **Supervisor/worker patterns** - Built on top of call_agent
- **Agent discovery service** - For now, use explicit IDs
- **Streaming responses** - Full response only in v1
- **Priority queues** - All calls equal priority

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Deadlock despite detection | Low | High | TLA+ model checking + DST stress tests |
| Performance degradation | Medium | Medium | Backpressure + monitoring |
| Complex debugging | Medium | Medium | Call chain logging + tracing |
| Scope creep to orchestration | High | Medium | Explicit "out of scope" list |

---

## Success Criteria

1. [x] TLA+ spec passes model checking (1.2M states, no violations)
2. [x] All 8 DST tests pass with fault injection
3. [x] Determinism verified (test_determinism_multi_agent passes)
4. [x] call_agent tool registered and validates input
5. [x] Cycles detected via validate_call_context()
6. [x] Timeouts bounded by AGENT_CALL_TIMEOUT_MS_MAX
7. [x] No clippy warnings, fmt passes

## Completion Notes

Implemented multi-agent communication foundation:
- **TLA+ spec** with 5 safety invariants verified by TLC
- **5 new fault types** for agent communication testing
- **8 DST tests** covering all safety invariants
- **call_agent tool** with TigerStyle constants and validation
- **Extended ToolExecutionContext** with call_depth and call_chain

### 2026-01-28 Update: Dispatcher Integration Complete

**Newly implemented:**
- **AgentDispatcher trait** - Abstraction for agent invocation (`tools/registry.rs`)
- **ContextAwareToolHandler type** - Handler type that receives execution context
- **DispatcherAdapter** - Bridges kelpie-runtime DispatcherHandle to AgentDispatcher trait
- **call_agent now invokes agents** - Uses dispatcher to actually call other agents via `handle_message_full`
- **ToolExecutionContext.dispatcher** - Field to pass dispatcher to context-aware tools
- **DST-compatible timeout** - Uses `kelpie_core::Runtime::timeout()` instead of tokio

**Issue #75 acceptance criteria now met:**
1. ✅ Design doc/ADR written (ADR-028)
2. ✅ Basic agent-to-agent call works via call_agent tool + dispatcher
3. ✅ Cycles, deadlocks, timeout handling implemented

**What's working:**
- `call_agent` tool can invoke other agents when dispatcher is provided
- Cycle detection prevents A→B→A deadlocks
- Call depth enforced (max 5 nested calls)
- Timeouts bounded and configurable

### 2026-01-28 Update: Dispatcher Wiring Complete

**Final implementation:**
- **agent_actor.rs**: `ToolExecutionContext.dispatcher` now wired from `self.dispatcher` via `DispatcherAdapter`
- **messages.rs**: Both `send_message` locations wire dispatcher from `state.dispatcher()`
- **state.rs**: Added `dispatcher()` getter to `AppState` for accessing the `DispatcherHandle`
- All TODO comments removed from production code

**Full Issue #75 implementation is now complete:**
1. ✅ Design doc/ADR written (ADR-028)
2. ✅ Basic agent-to-agent call works via call_agent tool + dispatcher
3. ✅ Cycles, deadlocks, timeout handling implemented
4. ✅ Dispatcher wired into all ToolExecutionContext call sites

---

## References

- [Issue #75](https://github.com/kelpie/issues/75)
- [ADR-001: Virtual Actor Model](docs/adr/001-virtual-actor-model.md)
- [ADR-013: Actor-Based Agent Server](docs/adr/013-actor-based-agent-server.md)
- [ADR-005: DST Framework](docs/adr/005-dst-framework.md)
- [CONSTRAINTS.md](.vision/CONSTRAINTS.md)
- [KelpieAgentActor.tla](docs/tla/KelpieAgentActor.tla)
