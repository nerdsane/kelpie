# EVI: Exploration & Verification Infrastructure

**Version:** 0.1.0 (Partial Implementation)
**Last Updated:** 2026-01-22
**Status:** Vision Document + Implementation Status

---

## Executive Summary

EVI (Exploration & Verification Infrastructure) is a comprehensive system for AI agent-driven software development. It provides the tools, workflows, and feedback loops that enable AI agents to:

1. **Explore** codebases systematically without overwhelming context windows
2. **Verify** correctness through formal specifications and deterministic testing
3. **Observe** production systems and feed learnings back into development
4. **Persist** state across sessions for continuity and audit trails

EVI is not a single tool but an integrated system of components that work together to enable rigorous, verification-first AI development.

---

## Table of Contents

1. [Philosophy](#1-philosophy)
2. [Architecture Overview](#2-architecture-overview)
3. [Current Components (Implemented)](#3-current-components-implemented)
4. [Missing Components (Not Yet Implemented)](#4-missing-components-not-yet-implemented)
5. [The Complete Investigation Loop](#5-the-complete-investigation-loop)
6. [Implementation Roadmap](#6-implementation-roadmap)
7. [Integration Points](#7-integration-points)

---

## 1. Philosophy

### 1.1 Core Principles

**Verification First, Not Documentation First**
- Trust execution output, not comments or documentation
- Every claim must have evidence from running code
- Completeness gates prevent premature answers

**Context as Variables, Not Tokens**
- Large codebases don't fit in context windows
- RLM (Recursive Language Models) keep data server-side
- Sub-LLM calls analyze without consuming main model context

**Closed-Loop Feedback**
- Production observations inform development
- Specifications derive from architectural decisions
- Tests prove specifications hold under faults

**Persistent State Across Sessions**
- AgentFS maintains facts, invariants, and verification records
- Work survives context window limits and session boundaries
- Full audit trail of what was examined and verified

### 1.2 The EVI Equation

```
EVI = Exploration + Verification + Observation + Persistence
    = (Indexes + RLM + Skills)
    + (Specs + TLA+ + DST)
    + (Traces + Metrics + Logs)
    + (AgentFS + Facts + Invariants)
```

### 1.3 What EVI Is NOT

- **Not a chatbot** - EVI is infrastructure, not a conversational agent
- **Not documentation** - EVI produces verified facts, not prose
- **Not one tool** - EVI is an integrated system of components
- **Not optional** - EVI enforces rigor through hard gates

---

## 2. Architecture Overview

### 2.1 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              EVI Architecture                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐    │
│  │   EXPLORE   │──▶│    SPEC     │──▶│   VERIFY    │──▶│   DEPLOY    │    │
│  │             │   │             │   │             │   │             │    │
│  │ • Indexes   │   │ • ADRs      │   │ • TLA+      │   │ • Runtime   │    │
│  │ • RLM       │   │ • Props     │   │ • DST       │   │ • Metrics   │    │
│  │ • AgentFS   │   │ • Specs     │   │ • Tests     │   │ • Traces    │    │
│  └─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘    │
│         ▲                                                     │            │
│         │                                                     │            │
│         │              ┌─────────────────────┐               │            │
│         │              │     PERSISTENCE     │               │            │
│         │              │                     │               │            │
│         │              │ • Facts             │               │            │
│         │              │ • Invariants        │               │            │
│         │              │ • Tool Calls        │               │            │
│         │              │ • Exam Sessions     │               │            │
│         │              └─────────────────────┘               │            │
│         │                                                     │            │
│         └─────────────────────────────────────────────────────┘            │
│                         OBSERVABILITY FEEDBACK LOOP                         │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Component Layers

| Layer | Purpose | Components |
|-------|---------|------------|
| **Exploration** | Understand codebases without context overflow | Indexes, RLM, MCP Tools |
| **Specification** | Formalize properties from architecture | ADRs, TLA+ Specs, Properties |
| **Verification** | Prove correctness under faults | TLA+ Model Checking, DST, Tests |
| **Observation** | Monitor production behavior | Traces, Metrics, Logs |
| **Persistence** | Maintain state across sessions | AgentFS, Facts, Invariants |
| **Instruction** | Guide agent behavior | CLAUDE.md, Skills, Hooks |

### 2.3 Data Flow

```
User Question
     │
     ▼
┌─────────────────┐
│  Instructions   │ ◀── CLAUDE.md, Skills, .vision/
│  (What to do)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Exploration   │ ◀── index_*, repl_load, sub_llm()
│  (Understand)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Examination    │ ◀── exam_start, exam_record, exam_complete
│  (Verify scope) │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Verification   │ ◀── TLA+ check, DST run, tests
│  (Prove correct)│
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Persistence   │ ◀── vfs_fact_add, vfs_invariant_verify
│  (Record proof) │
└────────┬────────┘
         │
         ▼
   Verified Answer
```

---

## 3. Current Components (Implemented)

### 3.1 MCP Server

**Location:** `kelpie-mcp/`
**Status:** ✅ Implemented (37 tools)

The MCP (Model Context Protocol) server provides tools that AI agents can call.

#### 3.1.1 REPL Tools (7 tools)

Server-side code execution with RLM capabilities.

| Tool | Purpose | Example |
|------|---------|---------|
| `repl_load` | Load files into server-side variables | `repl_load(pattern="**/*.rs", var_name="code")` |
| `repl_exec` | Execute Python code with `sub_llm()` available | `repl_exec(code="for f in code: sub_llm(...)")` |
| `repl_query` | Query loaded variables | `repl_query(var_name="code", query="file count")` |
| `repl_state` | Show all loaded variables | `repl_state()` |
| `repl_clear` | Clear loaded variables | `repl_clear()` or `repl_clear(var_name="code")` |
| `repl_sub_llm` | Analyze variable with sub-LLM | `repl_sub_llm(var_name="code", query="...")` |
| `repl_map_reduce` | Parallel analysis with aggregation | `repl_map_reduce(var_name="code", map_query="...", reduce_query="...")` |

**Key Feature: RLM (Recursive Language Models)**

```python
# sub_llm() is available INSIDE repl_exec code
repl_exec(code="""
for path, content in code.items():
    if 'test' in path:
        results[path] = sub_llm(content, "What does this test?")
result = results
""")
```

This enables:
- Programmatic analysis pipelines
- Conditional sub-LLM calls
- Multi-stage processing
- Structured output

#### 3.1.2 Index Tools (6 tools)

Pre-built structural indexes via tree-sitter parsing.

| Tool | Purpose | Data Source |
|------|---------|-------------|
| `index_symbols` | Query functions, structs, traits, impls | `symbols.json` |
| `index_modules` | Query module hierarchy | `modules.json` |
| `index_deps` | Query crate dependencies | `dependencies.json` |
| `index_tests` | Query test files and topics | `tests.json` |
| `index_status` | Check index freshness | Index metadata |
| `index_refresh` | Rebuild indexes | tree-sitter parsing |

**Index Location:** `.kelpie-index/structural/`

#### 3.1.3 AgentFS/VFS Tools (18 tools)

Persistent state via Turso AgentFS SDK.

| Category | Tools | Purpose |
|----------|-------|---------|
| **Session** | `vfs_init`, `vfs_status`, `vfs_export` | Manage verification sessions |
| **Facts** | `vfs_fact_add`, `vfs_fact_list`, `vfs_fact_get`, `vfs_fact_update`, `vfs_fact_delete` | Track verified facts with evidence |
| **Invariants** | `vfs_invariant_add`, `vfs_invariant_list`, `vfs_invariant_get`, `vfs_invariant_verify`, `vfs_invariant_update` | Track system invariants |
| **Tools** | `vfs_tool_start`, `vfs_tool_end`, `vfs_tool_list` | Audit trail of tool calls |
| **Query** | `vfs_query` | Query persistent state |

**Storage Location:** `.agentfs/agentfs-{session_id}.db`

#### 3.1.4 Examination Tools (6 tools)

Enforce thoroughness through completeness gates.

| Tool | Purpose | Gate? |
|------|---------|-------|
| `exam_start` | Start examination with scope | No |
| `exam_record` | Record findings for a component | No |
| `exam_status` | Check progress (examined vs remaining) | No |
| `exam_complete` | Verify all components examined | **YES - Hard Gate** |
| `exam_export` | Generate MAP.md and ISSUES.md | No |
| `issue_list` | Query discovered issues | No |

**Key Feature: Completeness Gate**

```python
exam_complete()
# Returns: {"can_answer": false, "remaining": ["kelpie-runtime", "kelpie-storage"]}
# Agent MUST NOT answer until can_answer is true
```

### 3.2 Structural Indexes

**Location:** `.kelpie-index/structural/`
**Status:** ✅ Implemented

| File | Contents | Size (typical) |
|------|----------|----------------|
| `symbols.json` | All functions, structs, traits, impls with locations | ~500KB |
| `modules.json` | Module hierarchy per crate | ~50KB |
| `dependencies.json` | Crate dependency graph | ~10KB |
| `tests.json` | All tests with topics and run commands | ~100KB |

**Built by:** Python indexer using tree-sitter-rust

### 3.3 Skills

**Location:** `.claude/skills/`
**Status:** ✅ Implemented (2 skills)

#### 3.3.1 codebase-map

**Purpose:** Build comprehensive map of entire codebase
**Trigger:** "map the codebase", "understand the codebase"
**Output:** MAP.md, ISSUES.md

**Workflow:**
1. `exam_start(scope=["all"])`
2. For each crate: indexes + RLM analysis with issue extraction
3. `exam_record()` for each component
4. `exam_complete()` - must pass
5. `exam_export()`

#### 3.3.2 thorough-answer

**Purpose:** Answer questions only after examining all relevant components
**Trigger:** "how does X work?", "where is Y?"
**Output:** Verified answer with evidence

**Workflow:**
1. Identify relevant components
2. `exam_start(scope=[relevant components])`
3. For each: indexes + RLM analysis
4. `exam_complete()` - must pass before answering
5. Provide answer with file:line citations

### 3.4 Instructions

**Status:** ✅ Implemented

| File | Purpose |
|------|---------|
| `CLAUDE.md` | Development guide, tool routing, RLM patterns |
| `.vision/CONSTRAINTS.md` | Non-negotiable project rules |
| `.vision/EVI.md` | This document |

### 3.5 Hooks

**Location:** `hooks/`
**Status:** ✅ Implemented

| Hook | Trigger | Actions |
|------|---------|---------|
| `pre-commit` | Before git commit | `cargo fmt --check`, `cargo clippy`, `cargo test` |
| `post-commit` | After git commit | `index_refresh` |

### 3.6 DST (Deterministic Simulation Testing)

**Location:** `crates/kelpie-dst/`
**Status:** ✅ Implemented

| Component | Purpose |
|-----------|---------|
| `Simulation` | Main DST harness |
| `SimClock` | Deterministic virtual time |
| `SimStorage` | Fault-injectable storage |
| `SimNetwork` | Fault-injectable network |
| `FaultConfig` | 16+ fault types with probabilities |
| `DeterministicRng` | Seeded PRNG for reproducibility |

**Reproducibility:**
```bash
DST_SEED=12345 cargo test -p kelpie-dst  # Same seed = same result
```

---

## 4. Missing Components (Not Yet Implemented)

### 4.1 Specification Pipeline

**Status:** ❌ Not Implemented

The pipeline from architectural decisions to formal specifications to tests.

#### 4.1.1 ADR → Properties Extraction

**Goal:** Extract formal properties from Architecture Decision Records

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `spec_extract` | Extract properties from ADR | ADR markdown | Property list |
| `spec_list` | List all extracted properties | - | Properties with sources |
| `spec_coverage` | Map properties to tests | - | Coverage report |

**Example Flow:**
```
docs/adr/004-linearizability.md
├─ "Actors have single-activation guarantee"
├─ "Storage operations are durable"
└─ "Messages are delivered at-most-once"
          │
          ▼ spec_extract
          │
specs/properties.json
├─ SingleActivation: "∀a ∈ Actors: |ActiveInstances(a)| ≤ 1"
├─ StorageDurability: "Write(k,v) ⇒ Eventually(Read(k) = v)"
└─ AtMostOnceDelivery: "∀m: DeliveryCount(m) ≤ 1"
```

#### 4.1.2 TLA+ Integration

**Goal:** Model check formal specifications

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `tla_generate` | Generate TLA+ from properties | Property | TLA+ spec file |
| `tla_check` | Run TLC model checker | TLA+ spec | Pass/fail + counterexample |
| `tla_coverage` | Map TLA+ specs to DST tests | - | Coverage gaps |

**Example Flow:**
```
specs/properties.json (SingleActivation)
          │
          ▼ tla_generate
          │
specs/tla/single_activation.tla
────────────────────────────────
VARIABLE actors, activeInstances

Invariant == \A a \in actors:
  Cardinality(activeInstances[a]) <= 1

Init == activeInstances = [a \in actors |-> {}]

Activate(a) ==
  /\ activeInstances[a] = {}
  /\ activeInstances' = [activeInstances EXCEPT ![a] = {self}]
────────────────────────────────
          │
          ▼ tla_check
          │
TLC Result: PASS (checked 10,000 states)
```

#### 4.1.3 Spec → DST Mapping

**Goal:** Ensure DST tests cover all formal properties

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `dst_from_spec` | Generate DST test skeleton | Property | Rust test file |
| `dst_coverage_report` | Map DST tests to specs | - | Coverage matrix |
| `dst_gaps` | Find untested properties | - | Gap list |

**Example Flow:**
```
specs/properties.json (SingleActivation)
          │
          ▼ dst_from_spec
          │
crates/kelpie-dst/tests/single_activation_dst.rs
─────────────────────────────────────────────────
#[test]
fn test_single_activation_under_faults() {
    let config = SimConfig::from_env_or_random();

    Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.1))
        .with_fault(FaultConfig::new(FaultType::CrashAfterWrite, 0.05))
        .run(|env| async move {
            // Property: SingleActivation
            // At no point should two instances of the same actor be active

            let actor_id = ActorId::new("test", "actor1");

            // Attempt concurrent activations
            let handles: Vec<_> = (0..10).map(|_| {
                env.spawn(activate_actor(actor_id.clone()))
            }).collect();

            // Verify invariant holds
            for _ in 0..100 {
                let active = env.registry.active_instances(&actor_id);
                assert!(active.len() <= 1, "SingleActivation violated!");
                env.advance_time_ms(10);
            }

            Ok(())
        });
}
─────────────────────────────────────────────────
```

### 4.2 Observability Integration

**Status:** ❌ Not Implemented

The feedback loop from production runtime back to development.

#### 4.2.1 Trace Query Tools

**Goal:** Query distributed traces from production

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `obs_trace_query` | Query traces by criteria | Filters (time, service, operation) | Trace list |
| `obs_trace_get` | Get full trace details | Trace ID | Span tree |
| `obs_trace_analyze` | Analyze trace patterns | Trace IDs | Pattern summary |

**Data Source:** OpenTelemetry → Tempo/Jaeger

**Example:**
```python
obs_trace_query(
    service="kelpie-server",
    operation="actor.activate",
    min_duration_ms=100,  # Slow activations
    time_range="1h"
)
# Returns: [trace_id_1, trace_id_2, ...]

obs_trace_analyze(trace_ids=[...])
# Returns: "Pattern: 80% of slow activations have storage.read as bottleneck"
```

#### 4.2.2 Metrics Query Tools

**Goal:** Query Prometheus metrics

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `obs_metrics_query` | Query metric values | PromQL | Time series |
| `obs_metrics_anomaly` | Detect anomalies | Metric name, time range | Anomaly list |
| `obs_metrics_compare` | Compare periods | Metric, period A, period B | Diff report |

**Data Source:** Prometheus

**Example:**
```python
obs_metrics_query(
    query="rate(kelpie_actor_activations_total[5m])",
    time_range="24h"
)
# Returns: Time series data

obs_metrics_anomaly(
    metric="kelpie_storage_latency_seconds",
    time_range="1h",
    sensitivity="high"
)
# Returns: [{"time": "...", "value": 0.5, "expected": 0.05, "severity": "high"}]
```

#### 4.2.3 Log Query Tools

**Goal:** Query structured logs

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `obs_logs_query` | Query logs by criteria | Filters (level, message pattern) | Log entries |
| `obs_logs_errors` | Get recent errors | Time range | Error summary |
| `obs_logs_context` | Get logs around an event | Timestamp, window | Log entries |

**Data Source:** Loki or similar

**Example:**
```python
obs_logs_errors(time_range="1h", service="kelpie-server")
# Returns: [
#   {"count": 15, "message": "Storage write timeout", "first": "...", "last": "..."},
#   {"count": 3, "message": "Actor activation failed", ...}
# ]
```

#### 4.2.4 Observation → Hypothesis Tools

**Goal:** Generate investigation hypotheses from observations

**Proposed Tools:**

| Tool | Purpose | Input | Output |
|------|---------|-------|--------|
| `obs_to_hypothesis` | Generate hypotheses from anomalies | Anomaly data | Hypothesis list |
| `obs_to_dst` | Generate DST scenario from incident | Incident details | DST test skeleton |
| `obs_correlate` | Correlate metrics, traces, logs | Time range | Correlation report |

**Example:**
```python
obs_to_hypothesis(
    anomaly={
        "type": "latency_spike",
        "service": "kelpie-server",
        "operation": "actor.activate",
        "time": "2026-01-22T10:00:00Z"
    }
)
# Returns: [
#   {"hypothesis": "Storage contention under load", "confidence": 0.8, "evidence": "..."},
#   {"hypothesis": "Network partition to registry", "confidence": 0.3, "evidence": "..."}
# ]

obs_to_dst(
    incident={
        "description": "Actor activation latency spike",
        "root_cause": "Storage contention",
        "evidence": ["trace_123", "metric_data"]
    }
)
# Returns: DST test skeleton that reproduces the scenario
```

### 4.3 Enhanced Investigation Loop

**Status:** ❌ Not Implemented

Tools to support iterative investigation.

#### 4.3.1 Hypothesis Tracking

**Proposed VFS Extensions:**

| Tool | Purpose |
|------|---------|
| `vfs_hypothesis_add` | Add investigation hypothesis |
| `vfs_hypothesis_update` | Update hypothesis status (confirmed/rejected) |
| `vfs_hypothesis_list` | List hypotheses with evidence |

#### 4.3.2 Investigation Session

**Proposed Tools:**

| Tool | Purpose |
|------|---------|
| `investigate_start` | Start investigation session with trigger |
| `investigate_status` | Show investigation progress |
| `investigate_conclude` | Conclude with root cause and fix |

### 4.4 Spec Coverage Dashboard

**Status:** ❌ Not Implemented

Visual representation of specification coverage.

**Proposed Output:** `.kelpie-index/coverage/`

```
specs/
├── properties.json         # All extracted properties
├── tla/                    # TLA+ specifications
├── coverage.json           # Property → Test mapping
└── gaps.md                 # Untested properties

Coverage Report:
┌────────────────────┬───────────┬─────────────┬───────────┐
│ Property           │ TLA+ Spec │ DST Test    │ Coverage  │
├────────────────────┼───────────┼─────────────┼───────────┤
│ SingleActivation   │ ✅        │ ✅          │ 100%      │
│ StorageDurability  │ ✅        │ ⚠️ Partial  │ 60%       │
│ AtMostOnceDelivery │ ❌        │ ❌          │ 0%        │
└────────────────────┴───────────┴─────────────┴───────────┘
```

---

## 5. The Complete Investigation Loop

### 5.1 Triggers

An investigation can be triggered by:

1. **User Question** - "Why is actor X slow?"
2. **Anomaly Alert** - `obs_metrics_anomaly()` detects unusual pattern
3. **Spec Violation** - `tla_check()` finds counterexample
4. **DST Failure** - Test fails with seed
5. **Production Incident** - Error rate spike

### 5.2 Investigation Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Complete Investigation Loop                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. TRIGGER                                                                 │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ • User question    • Anomaly alert    • Spec violation          │    │
│     │ • DST failure      • Production incident                        │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                    │                                        │
│                                    ▼                                        │
│  2. EXPLORE                                                                 │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ exam_start(task="...", scope=[...])                             │    │
│     │ index_* for structure                                           │    │
│     │ repl_load + sub_llm() for understanding                         │    │
│     │ vfs_fact_add() to record findings                               │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                    │                                        │
│                                    ▼                                        │
│  3. OBSERVE (if production issue)                                          │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ obs_trace_query() for runtime behavior                          │    │
│     │ obs_metrics_query() for quantitative data                       │    │
│     │ obs_logs_query() for error details                              │    │
│     │ obs_correlate() to find patterns                                │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                    │                                        │
│                                    ▼                                        │
│  4. HYPOTHESIZE                                                            │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ obs_to_hypothesis() generates theories                          │    │
│     │ vfs_hypothesis_add() to track                                   │    │
│     │ Cross-reference with specs/ADRs                                 │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                    │                                        │
│                                    ▼                                        │
│  5. VERIFY                                                                  │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ spec_coverage() - Is this scenario covered by specs?            │    │
│     │ tla_check() - Does the model predict this?                      │    │
│     │ dst_run() - Can we reproduce in simulation?                     │    │
│     │ exam_complete() - Did we examine all relevant code?             │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                    │                                        │
│                                    ▼                                        │
│  6. FIX & CLOSE LOOP                                                       │
│     ┌─────────────────────────────────────────────────────────────────┐    │
│     │ Implement fix                                                   │    │
│     │ Add DST test for the scenario                                   │    │
│     │ Update spec if needed                                           │    │
│     │ vfs_invariant_verify() to record verification                   │    │
│     │ Deploy and monitor (back to OBSERVE)                            │    │
│     └─────────────────────────────────────────────────────────────────┘    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 5.3 Example: Production Latency Investigation

```python
# 1. TRIGGER: Anomaly detected
obs_metrics_anomaly(metric="kelpie_actor_activation_latency", time_range="1h")
# Result: {"severity": "high", "value": 500ms, "baseline": 50ms}

# 2. EXPLORE: Find relevant code
exam_start(task="Investigate activation latency", scope=["kelpie-runtime", "kelpie-registry"])
index_symbols(pattern="activate", kind="fn")
repl_load(pattern="crates/kelpie-runtime/src/dispatcher.rs", var_name="dispatcher")
repl_exec(code="""
analysis = sub_llm(dispatcher['dispatcher.rs'],
    "What could cause activation latency? Look for I/O, locks, network calls.")
result = analysis
""")

# 3. OBSERVE: Get production data
obs_trace_query(operation="actor.activate", min_duration_ms=100, limit=10)
obs_trace_analyze(trace_ids=[...])
# Result: "80% of slow traces have storage.ensure_schema as bottleneck"

# 4. HYPOTHESIZE
obs_to_hypothesis(anomaly_data)
# Result: [{"hypothesis": "Schema creation on every activation", "confidence": 0.9}]
vfs_hypothesis_add(name="SchemaCreation", description="...", evidence="trace analysis")

# 5. VERIFY: Check against specs and reproduce
spec_coverage(property="ActivationLatency")
# Result: "No spec covers activation latency bounds"

dst_run(scenario="high_activation_rate", faults=[])
# Result: SLOW - confirms latency even without faults

# 6. FIX & CLOSE
# - Fix: Cache schema check
# - Add DST test: test_activation_latency_dst.rs
# - Add spec: ActivationLatency < 100ms under normal conditions
# - Deploy and monitor
obs_metrics_query(query="kelpie_actor_activation_latency", time_range="1h")
# Result: Back to baseline
vfs_invariant_verify(name="ActivationLatency", evidence="Post-fix metrics show 50ms avg")
```

---

## 6. Implementation Roadmap

### Phase 1: Current State (Complete)

| Component | Status |
|-----------|--------|
| MCP Server (37 tools) | ✅ |
| Structural Indexes | ✅ |
| RLM (sub_llm inside REPL) | ✅ |
| AgentFS Persistence | ✅ |
| Examination System | ✅ |
| Skills (2) | ✅ |
| Hooks | ✅ |
| Instructions | ✅ |
| DST Framework | ✅ |

### Phase 2: Specification Pipeline

**Goal:** ADR → Properties → TLA+ → DST

| Task | Tools | Effort |
|------|-------|--------|
| Property extraction from ADRs | `spec_extract`, `spec_list` | Medium |
| TLA+ generation | `tla_generate` | Medium |
| TLC integration | `tla_check` | Medium |
| Spec-to-DST mapping | `dst_from_spec`, `dst_coverage_report` | Medium |
| Coverage dashboard | `spec_coverage` | Low |

### Phase 3: Observability Integration

**Goal:** Production feedback loop

| Task | Tools | Effort |
|------|-------|--------|
| Trace query (Tempo/Jaeger) | `obs_trace_*` | Medium |
| Metrics query (Prometheus) | `obs_metrics_*` | Medium |
| Log query (Loki) | `obs_logs_*` | Medium |
| Anomaly detection | `obs_*_anomaly` | High |
| Hypothesis generation | `obs_to_hypothesis` | High |
| DST from incident | `obs_to_dst` | Medium |

### Phase 4: Enhanced Investigation

**Goal:** Iterative investigation support

| Task | Tools | Effort |
|------|-------|--------|
| Hypothesis tracking | `vfs_hypothesis_*` | Low |
| Investigation sessions | `investigate_*` | Medium |
| Correlation analysis | `obs_correlate` | High |

### Phase 5: Integration & Polish

**Goal:** Seamless end-to-end workflows

| Task | Description | Effort |
|------|-------------|--------|
| Skills for spec workflow | `/spec-check`, `/verify-property` | Medium |
| Skills for investigation | `/investigate`, `/root-cause` | Medium |
| Dashboard generation | Coverage, investigation status | Medium |
| Documentation | Complete guides for each workflow | Low |

---

## 7. Integration Points

### 7.1 MCP Server Configuration

**Location:** `.mcp.json`

```json
{
  "mcpServers": {
    "kelpie": {
      "command": "uv",
      "args": ["run", "--directory", "./kelpie-mcp", "--prerelease=allow", "mcp-kelpie"],
      "env": {
        "KELPIE_CODEBASE_PATH": "..",
        "KELPIE_SUB_LLM_MODEL": "claude-haiku-4-5-20251001",
        "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"
      }
    }
  }
}
```

### 7.2 Future: Observability Configuration

```json
{
  "observability": {
    "traces": {
      "backend": "tempo",
      "endpoint": "http://localhost:3200"
    },
    "metrics": {
      "backend": "prometheus",
      "endpoint": "http://localhost:9090"
    },
    "logs": {
      "backend": "loki",
      "endpoint": "http://localhost:3100"
    }
  }
}
```

### 7.3 Future: Spec Configuration

```json
{
  "specs": {
    "adr_path": "docs/adr/",
    "properties_path": "specs/properties.json",
    "tla_path": "specs/tla/",
    "tlc_path": "/usr/local/bin/tlc"
  }
}
```

---

## Appendix A: Tool Quick Reference

### Current Tools (37)

| Category | Count | Tools |
|----------|-------|-------|
| REPL | 7 | `repl_load`, `repl_exec`, `repl_query`, `repl_state`, `repl_clear`, `repl_sub_llm`, `repl_map_reduce` |
| Index | 6 | `index_symbols`, `index_modules`, `index_deps`, `index_tests`, `index_status`, `index_refresh` |
| AgentFS | 18 | `vfs_init`, `vfs_status`, `vfs_export`, `vfs_fact_*` (5), `vfs_invariant_*` (5), `vfs_tool_*` (3), `vfs_query` |
| Examination | 6 | `exam_start`, `exam_record`, `exam_status`, `exam_complete`, `exam_export`, `issue_list` |

### Proposed Tools (Future)

| Category | Count | Tools |
|----------|-------|-------|
| Spec | 6 | `spec_extract`, `spec_list`, `spec_coverage`, `tla_generate`, `tla_check`, `dst_from_spec` |
| Observability | 12 | `obs_trace_*` (3), `obs_metrics_*` (3), `obs_logs_*` (3), `obs_to_hypothesis`, `obs_to_dst`, `obs_correlate` |
| Investigation | 5 | `vfs_hypothesis_*` (3), `investigate_start`, `investigate_conclude` |

---

## Appendix B: Glossary

| Term | Definition |
|------|------------|
| **EVI** | Exploration & Verification Infrastructure |
| **RLM** | Recursive Language Models - sub_llm() inside code |
| **DST** | Deterministic Simulation Testing |
| **AgentFS** | Persistent state storage for AI agents |
| **MCP** | Model Context Protocol |
| **Examination** | Scoped analysis with completeness gates |
| **VFS** | Virtual File System (AgentFS wrapper) |
| **TLA+** | Temporal Logic of Actions (formal specification language) |
| **TLC** | TLA+ model checker |

---

## Appendix C: File Locations

| Component | Location |
|-----------|----------|
| MCP Server | `kelpie-mcp/` |
| Indexes | `.kelpie-index/structural/` |
| AgentFS Data | `.agentfs/` |
| Skills | `.claude/skills/` |
| Hooks | `hooks/` |
| Instructions | `CLAUDE.md`, `.vision/` |
| DST | `crates/kelpie-dst/` |
| ADRs | `docs/adr/` |
| Specs (future) | `specs/` |

---

*This document is the authoritative reference for EVI. Update it as components are implemented.*
