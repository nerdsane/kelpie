# Spec: Multi-Agent Communication (v1 - FAILED)

**ID**: SPEC-001
**Status**: FAILED
**Priority**: P0
**Failed**: 2026-01-28
**Failure Reason**: Self-certification allowed false completion claims
**Superseded By**: [multi-agent-communication-v2.md](./multi-agent-communication-v2.md)
**Issue**: https://github.com/kelpie/issues/75
**Plan**: [.progress/054_20260128_multi-agent-communication-design.md](../../.progress/054_20260128_multi-agent-communication-design.md)

---

## POST-MORTEM: Why This Spec Failed

**What Happened**: Ralph marked this spec COMPLETE despite critical deliverables missing:
- ADR-028 does not exist (marked as [x] complete)
- TLA+ spec does not exist (marked as [x] complete)
- TLC was never executed (marked as [x] complete)
- New fault types not added (marked as [x] complete)
- Dispatcher not wired (call_agent always fails)

**Root Cause**: Spec relied on checkbox self-certification without verification gates.

**Lesson Learned**: Specs must include HARD VERIFICATION COMMANDS that must be run and output pasted as evidence. No self-certification.

**Resolution**: See [multi-agent-communication-v2.md](./multi-agent-communication-v2.md) which requires command output as evidence.

---

---

## Overview

Implement agent-to-agent communication for Kelpie, enabling one agent to invoke another agent and receive a response. This enables multi-agent workflows, delegation, and orchestration patterns.

---

## Mandatory Deliverables

### 1. Architecture Decision Record (ADR)

**File**: `docs/adr/028-multi-agent-communication.md`

**Required Sections**:
- Context and problem statement
- Decision drivers
- Options considered (minimum 3)
- Decision outcome with rationale
- Consequences (positive, negative, neutral)
- Implementation details
- DST coverage requirements
- TLA+ spec reference

**Acceptance Criteria**:
- [ ] ADR exists at `docs/adr/028-multi-agent-communication.md`
- [ ] All required sections are present and substantive
- [ ] References TLA+ spec and DST tests
- [ ] Documents safety mechanisms (cycle detection, timeout, depth limit)

---

### 2. TLA+ Specification

**File**: `docs/tla/KelpieMultiAgentInvocation.tla`

**Required Invariants**:

```tla
(* SAFETY - Must NEVER be violated *)

\* No circular calls - agent cannot be in its own call stack
NoDeadlock ==
    \A a \in Agents:
        LET stack == callStack[a]
        IN Cardinality(ToSet(stack)) = Len(stack)

\* Single activation guarantee holds during cross-agent calls
SingleActivationDuringCall ==
    \A a \in Agents:
        Cardinality({n \in Nodes : agentState[n][a] \in {"Running", "WaitingForCall"}}) <= 1

\* Call depth bounded
DepthBounded ==
    \A a \in Agents: Len(callStack[a]) <= MAX_DEPTH

\* Timeout fires before deadlock
TimeoutPreventsHang ==
    \A call \in ActiveCalls:
        call.elapsed_ms <= TIMEOUT_MS_MAX
```

**Required Liveness Properties**:

```tla
(* LIVENESS - Must EVENTUALLY happen *)

\* Every call eventually completes, times out, or fails
CallEventuallyCompletes ==
    \A call \in Calls:
        callState[call] = "Pending" ~>
            callState[call] \in {"Completed", "TimedOut", "Failed", "CycleRejected"}

\* Cycle detection happens before first iteration
CycleDetectedEarly ==
    \A call \in Calls:
        (call.creates_cycle) => <>(callState[call] = "CycleRejected")
```

**Acceptance Criteria**:
- [ ] TLA+ spec exists at `docs/tla/KelpieMultiAgentInvocation.tla`
- [ ] All 4 safety invariants defined
- [ ] All 2 liveness properties defined
- [ ] **TLC model checker executed successfully**
- [ ] TLC output saved to `docs/tla/KelpieMultiAgentInvocation_TLC_output.txt`
- [ ] No invariant violations in TLC output
- [ ] State space explored documented (states, depth)

**TLC Execution Command**:
```bash
cd docs/tla
tlc KelpieMultiAgentInvocation.tla -config KelpieMultiAgentInvocation.cfg
```

---

### 3. DST Harness Extension

**Location**: `crates/kelpie-dst/`

**Required Fault Types** (add to `faults.rs`):

```rust
pub enum FaultType {
    // ... existing faults ...

    // Multi-agent specific
    AgentCallTimeout,        // Called agent doesn't respond in time
    AgentCallRejected,       // Called agent refuses the call
    AgentNotFound,           // Target agent doesn't exist
    AgentBusy,               // Target agent at max concurrent calls
    AgentCallNetworkDelay,   // Network delay specific to agent calls
}
```

**Required SimEnvironment Extension**:

```rust
pub struct SimMultiAgentEnv {
    pub agents: HashMap<String, SimAgent>,
    pub dispatcher: SimDispatcher,
    pub call_graph: CallGraph,  // For cycle detection verification
    pub time: SimClock,
    pub rng: DeterministicRng,
}
```

**Acceptance Criteria**:
- [ ] New fault types added to `FaultType` enum
- [ ] Fault injection works for agent-to-agent calls
- [ ] `SimMultiAgentEnv` or equivalent harness exists
- [ ] Harness can inject faults at agent call boundaries
- [ ] Call graph tracking available for verification

---

### 4. DST Tests (Simulation Runs Actual Implementation)

**File**: `crates/kelpie-server/tests/multi_agent_dst.rs`

**CRITICAL REQUIREMENT**: The simulation must run the **actual production implementation**, not disconnected mocks. The test creates real `AgentActor` instances with `SimStorage`, `SimClock`, and fault injection.

**Required Tests**:

```rust
#[madsim::test]
async fn test_agent_calls_agent_success() {
    // Agent A calls Agent B, receives response
    // Verifies: Basic functionality works
}

#[madsim::test]
async fn test_agent_call_cycle_detection() {
    // A calls B, B calls A - must be rejected, not deadlock
    // Verifies: NoDeadlock invariant
}

#[madsim::test]
async fn test_agent_call_timeout() {
    // Called agent is slow, caller times out
    // Verifies: TimeoutPreventsHang invariant
}

#[madsim::test]
async fn test_agent_call_depth_limit() {
    // A→B→C→D→E→F (depth 5), F→G must fail
    // Verifies: DepthBounded invariant
}

#[madsim::test]
async fn test_agent_call_under_network_partition() {
    // Network partition between agents
    // Verifies: Graceful failure, no corruption
}

#[madsim::test]
async fn test_single_activation_during_cross_call() {
    // Concurrent calls to same agent
    // Verifies: SingleActivationDuringCall invariant
}

#[madsim::test]
async fn test_agent_call_with_storage_faults() {
    // 10% storage failure during call
    // Verifies: Fault tolerance
}

#[madsim::test]
async fn test_determinism_multi_agent() {
    // Same seed produces identical call sequences
    // Verifies: DST determinism
}
```

**Acceptance Criteria**:
- [ ] All 8 DST tests exist
- [ ] Tests use `#[madsim::test]` for deterministic scheduling
- [ ] Tests run **actual** `AgentActor` implementation (not mocks)
- [ ] Tests use `Simulation` harness with fault injection
- [ ] All tests pass: `cargo test -p kelpie-server --test multi_agent_dst`
- [ ] Determinism verified: same `DST_SEED` produces identical results

**Verification Command**:
```bash
# Run all multi-agent DST tests
cargo test -p kelpie-server --test multi_agent_dst

# Verify determinism (run twice with same seed, diff output)
DST_SEED=12345 cargo test -p kelpie-server --test multi_agent_dst -- --nocapture > run1.txt
DST_SEED=12345 cargo test -p kelpie-server --test multi_agent_dst -- --nocapture > run2.txt
diff run1.txt run2.txt  # Must be empty
```

---

### 5. Implementation (TigerStyle + FoundationDB Best Practices)

**Files to Create/Modify**:

| File | Action | Description |
|------|--------|-------------|
| `tools/agent_call.rs` | CREATE | `call_agent` tool implementation |
| `tools/mod.rs` | MODIFY | Extend `ToolExecutionContext` |
| `tools/registry.rs` | MODIFY | Thread context to handlers |
| `dispatcher.rs` | MODIFY | Add `invoke_with_timeout()` |

**TigerStyle Requirements**:

```rust
// Constants with units
pub const AGENT_CALL_DEPTH_MAX: u32 = 5;
pub const AGENT_CALL_TIMEOUT_MS_DEFAULT: u64 = 30_000;
pub const AGENT_CALL_TIMEOUT_MS_MAX: u64 = 300_000;
pub const AGENT_CONCURRENT_CALLS_MAX: usize = 10;

// 2+ assertions per function
pub async fn call_agent(ctx: &CallContext, target: &str, message: &str) -> Result<String> {
    // Preconditions
    assert!(!target.is_empty(), "target agent ID must not be empty");
    assert!(ctx.depth < AGENT_CALL_DEPTH_MAX, "call depth exceeded");
    assert!(!ctx.call_chain.contains(&target.to_string()), "cycle detected");

    // ... implementation ...

    // Postcondition
    debug_assert!(result.len() <= AGENT_RESPONSE_SIZE_BYTES_MAX);
    Ok(result)
}
```

**FoundationDB Best Practices**:

1. **All I/O through injected providers** - No `tokio::time::sleep`, use `time_provider.sleep_ms()`
2. **Deterministic task scheduling** - Use madsim for tests
3. **Explicit fault handling** - Every error case documented and tested
4. **Simulation runs production code** - No separate "sim" implementation

**Acceptance Criteria**:
- [ ] All TigerStyle constants defined with units
- [ ] 2+ assertions per non-trivial function
- [ ] No `unwrap()` in production code
- [ ] All I/O through injected providers
- [ ] `cargo clippy` passes with no warnings
- [ ] `cargo fmt --check` passes

---

### 6. Integration Verification

**End-to-End Test**:

```bash
# Start server
cargo run -p kelpie-server &

# Create two agents
AGENT_A=$(curl -s -X POST http://localhost:8283/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "coordinator"}' | jq -r '.id')

AGENT_B=$(curl -s -X POST http://localhost:8283/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "helper"}' | jq -r '.id')

# Coordinator calls helper (LLM uses call_agent tool)
curl -X POST "http://localhost:8283/v1/agents/${AGENT_A}/messages" \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": "Use the call_agent tool to ask agent '"${AGENT_B}"' what 2+2 equals"}'
```

**Acceptance Criteria**:
- [ ] Server starts without errors
- [ ] Agent A can call Agent B via the tool
- [ ] Response includes Agent B's output
- [ ] No crashes, hangs, or error responses

---

## Completion Checklist (AUDIT - ACTUAL STATE)

**Audited 2026-01-28**: The following shows ACTUAL state, not self-reported state.

```
[ ] ADR-028 exists and is complete           <- MISSING: file does not exist
[ ] TLA+ spec exists with all invariants     <- MISSING: file does not exist
[ ] TLC executed successfully (no violations) <- NEVER RAN: no TLC output
[ ] TLC output saved                         <- MISSING: file does not exist
[ ] DST harness extended with new fault types <- NOT DONE: fault types not added
[x] All 8 DST tests exist and pass           <- VERIFIED: tests pass
[x] DST determinism verified                 <- VERIFIED: same seed = same result
[x] Implementation follows TigerStyle        <- VERIFIED: constants, assertions present
[x] No clippy warnings                       <- VERIFIED: clippy clean
[x] No fmt issues                            <- VERIFIED: fmt clean
[ ] Integration test passes                  <- BLOCKED: dispatcher not wired
[x] Code committed and pushed                <- VERIFIED: commits exist
```

**RESULT: 5 of 12 deliverables missing. Spec FAILED.**

---

## References

- **Plan**: [.progress/054_20260128_multi-agent-communication-design.md](../../.progress/054_20260128_multi-agent-communication-design.md)
- **Constraints**: [.vision/CONSTRAINTS.md](../../.vision/CONSTRAINTS.md)
- **DST Framework**: [docs/adr/005-dst-framework.md](../../docs/adr/005-dst-framework.md)
- **Virtual Actor Model**: [docs/adr/001-virtual-actor-model.md](../../docs/adr/001-virtual-actor-model.md)
- **Existing TLA+ Specs**: `docs/tla/*.tla`

---

## Estimated Effort

- TLA+ Spec + TLC: 0.5 day
- DST Harness Extension: 0.5 day
- DST Tests: 1 day
- Implementation: 1.5 days
- ADR + Documentation: 0.5 day
- **Total**: ~4 days

---

## Notes for Ralph

1. **Order matters**: TLA+ spec MUST be verified with TLC BEFORE writing implementation
2. **DST tests MUST use actual implementation** - do not create separate mock implementations
3. **Fail fast on cycles** - cycle detection must happen before any invocation attempt
4. **Commit incrementally** - commit after each major deliverable (TLA+, harness, tests, impl)
5. **Verify determinism** - run DST tests twice with same seed, diff must be empty
