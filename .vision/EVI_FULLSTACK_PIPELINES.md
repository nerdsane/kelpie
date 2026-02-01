# EVI Full-Stack Pipelines

**Version:** 0.1.0
**Last Updated:** 2026-01-22

This document details:
1. The ADR → TLA+ → DST pipeline for full-stack (backend + frontend)
2. The closed-loop observability feedback system

---

## 0. What's Actually Valuable (Honest Assessment)

**Not all verification is equal.** Here's what's essential vs. overengineered:

### Essential (High ROI)

| Layer | What | Why |
|-------|------|-----|
| **Backend** | ADR → TLA+ → DST | Distributed bugs are **invisible**. You can't see race conditions by using the app. Specs find bugs tests miss. |
| **Frontend** | Basic indexes + E2E tests | Most UI bugs are **visible**. You see them when you use the app. |
| **Both** | Learning tools (obs_to_spec, obs_to_dst) | Production teaches you what matters. Close the loop. |

### Probably Overengineered (Use Sparingly)

| What | When It's Useful | When It's Overkill |
|------|------------------|-------------------|
| **XState state machines** | Complex stateful UIs (wizard flows, multi-step forms, real-time collaboration) | Simple CRUD apps, static pages |
| **Property-based UI testing** | Components with many states/combinations | Basic components with 2-3 states |
| **TLA+ for frontend** | Distributed frontend state (CRDTs, collaborative editing) | Almost never needed |

### The Core Insight

**Backend bugs are invisible. Frontend bugs are visible.**

- A race condition in actor activation? You'll never see it by clicking around.
- A broken button? You'll see it immediately.

This is why:
- Backend gets formal specs (TLA+) and simulation testing (DST)
- Frontend gets simple E2E tests and learns from production errors

### Pragmatic Frontend Verification

For most apps, this is enough:

```
┌──────────────────────────────────────────────────────────────────┐
│  Pragmatic Frontend Verification                                 │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. SPEC (define constraints)                                    │
│     └─ specs/app.spec.edn (P0 invariants, states, flows)        │
│                                                                  │
│  2. GENERATE (tests from spec)                                   │
│     ├─ evi generate-tests --type invariant                      │
│     ├─ evi generate-tests --type e2e                            │
│     └─ evi generate-eslint (lint rules)                         │
│                                                                  │
│  3. HOOK (P0s at every commit)                                   │
│     └─ Pre-commit shows P0 constraints, runs tests              │
│                                                                  │
│  4. LEARN (close the loop)                                       │
│     ├─ Production error → add invariant to spec                 │
│     └─ obs_to_spec → generate spec from incident                │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

**See [EVI_FULLSTACK.md](./EVI_FULLSTACK.md) Section 1.3 for the full EDN spec format.**

### Pragmatic Frontend: EDN Spec → Tests

For most apps, this replaces the full XState/property-test pipeline:

**Single source of truth:** `specs/app.spec.edn` defines P0 constraints, component states, flows, and design tokens. See [EVI_FULLSTACK.md](./EVI_FULLSTACK.md) Section 1.3 for the full spec format.

**Key points:**
1. **P0 invariants** are extracted and shown at every commit via pre-commit hook
2. **Tests are generated** from the spec, not hand-written
3. **XState only when needed** - most components don't need full state machines

**The workflow:**
```bash
# 1. Define constraints in spec
vim specs/app.spec.edn

# 2. Generate tests
evi generate-tests specs/app.spec.edn --type invariant
evi generate-tests specs/app.spec.edn --type e2e

# 3. Pre-commit hook shows P0s at every commit
# (automatically installed)
```

**No separate ADRs for frontend invariants.** They live in the spec file.

---

## 1. ADR → Spec → Test Pipeline (Full-Stack)

### 1.1 Pipeline Overview (Full Approach)

*Note: The diagram below shows the FULL approach. For most apps, use the pragmatic approach above instead.*

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Full-Stack Specification Pipeline                         │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│                              DECISIONS                                      │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐            │
│  │   Backend ADR   │  │  Frontend ADR   │  │    UX Decision  │            │
│  │                 │  │                 │  │                 │            │
│  │ "Actors have    │  │ "Actor list     │  │ "Create actor   │            │
│  │  single         │  │  shows real-    │  │  shows instant  │            │
│  │  activation"    │  │  time updates"  │  │  feedback"      │            │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘            │
│           │                    │                    │                      │
│           ▼                    ▼                    ▼                      │
│                           SPECIFICATIONS                                    │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐            │
│  │    TLA+ Spec    │  │  Invariant Test │  │  UX Heuristic   │            │
│  │                 │  │   (or XState    │  │     Check       │            │
│  │ SingleActivation│  │   for complex)  │  │ "System status  │            │
│  │ Invariant       │  │                 │  │  must be        │            │
│  │                 │  │                 │  │  visible"       │            │
│  └────────┬────────┘  └────────┬────────┘  └────────┬────────┘            │
│           │                    │                    │                      │
│           ▼                    ▼                    ▼                      │
│                              TESTS                                         │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐            │
│  │    DST Test     │  │ Invariant Test  │  │    E2E Test     │            │
│  │                 │  │  (or Property)  │  │                 │            │
│  │ Verify under    │  │ Verify rules    │  │ Verify user     │            │
│  │ network faults  │  │ are followed    │  │ sees feedback   │            │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘            │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Backend Pipeline: ADR → TLA+ → DST

**Step 1: Architectural Decision Record**

```markdown
# docs/adr/007-actor-activation-guarantee.md

## Status
Accepted

## Context
Actors represent stateful entities. If two instances of the same actor
run simultaneously, they could corrupt shared state.

## Decision
Enforce single-activation guarantee: at most one instance of any actor
can be active at any time, cluster-wide.

## Consequences
- Registry must coordinate activation across nodes
- Activation may fail if actor is active elsewhere
- Need distributed locking or consensus

## Properties to Verify
1. SingleActivation: ∀ actor: |active_instances(actor)| ≤ 1
2. ActivationEventuallySucceeds: activation request → eventually active or rejected
3. DeactivationCleanup: deactivation → no orphaned state
```

**Step 2: Extract Properties → TLA+ Specification**

```tla
(* specs/tla/actor_activation.tla *)

---------------------------- MODULE ActorActivation ----------------------------
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS Actors, Nodes

VARIABLES
    activeOn,        \* activeOn[actor] = node where actor is active (or NULL)
    pendingActivations,
    pendingDeactivations

TypeInvariant ==
    /\ activeOn \in [Actors -> Nodes \cup {NULL}]
    /\ pendingActivations \subseteq Actors
    /\ pendingDeactivations \subseteq Actors

-----------------------------------------------------------------------------
(* INVARIANT 1: Single Activation *)
SingleActivation ==
    \A a \in Actors:
        Cardinality({n \in Nodes : activeOn[a] = n}) <= 1

(* INVARIANT 2: No Ghost Activations *)
NoGhostActivations ==
    \A a \in Actors:
        a \in pendingDeactivations => activeOn[a] # NULL

-----------------------------------------------------------------------------
(* ACTIONS *)

Activate(actor, node) ==
    /\ activeOn[actor] = NULL
    /\ activeOn' = [activeOn EXCEPT ![actor] = node]
    /\ UNCHANGED <<pendingActivations, pendingDeactivations>>

Deactivate(actor) ==
    /\ activeOn[actor] # NULL
    /\ activeOn' = [activeOn EXCEPT ![actor] = NULL]
    /\ UNCHANGED <<pendingActivations, pendingDeactivations>>

(* Node crash - actor becomes orphaned until timeout *)
NodeCrash(node) ==
    /\ activeOn' = [a \in Actors |->
        IF activeOn[a] = node THEN NULL ELSE activeOn[a]]
    /\ UNCHANGED <<pendingActivations, pendingDeactivations>>

-----------------------------------------------------------------------------
Init ==
    /\ activeOn = [a \in Actors |-> NULL]
    /\ pendingActivations = {}
    /\ pendingDeactivations = {}

Next ==
    \/ \E a \in Actors, n \in Nodes: Activate(a, n)
    \/ \E a \in Actors: Deactivate(a)
    \/ \E n \in Nodes: NodeCrash(n)

Spec == Init /\ [][Next]_<<activeOn, pendingActivations, pendingDeactivations>>

-----------------------------------------------------------------------------
(* PROPERTIES TO CHECK *)
THEOREM Spec => []SingleActivation
THEOREM Spec => []NoGhostActivations
================================================================================
```

**Step 3: Generate DST Test from TLA+ Properties**

```rust
// crates/kelpie-dst/tests/actor_activation_dst.rs

use kelpie_dst::{Simulation, SimConfig, FaultConfig, FaultType};

/// DST test derived from TLA+ spec: actor_activation.tla
/// Verifies: SingleActivation invariant under faults
#[test]
fn test_single_activation_invariant() {
    let config = SimConfig::from_env_or_random();
    println!("DST seed: {}", config.seed);

    let result = Simulation::new(config)
        // Inject faults that TLA+ models
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.1))
        .with_fault(FaultConfig::new(FaultType::NodeCrash, 0.05))
        .with_fault(FaultConfig::new(FaultType::MessageDelay, 0.2))
        .run(|env| async move {
            let actor_id = ActorId::new("test", "actor1");

            // Spawn concurrent activation attempts (like TLA+ Activate action)
            let handles: Vec<_> = (0..10).map(|i| {
                let id = actor_id.clone();
                let registry = env.registry.clone();
                env.spawn(async move {
                    registry.activate(&id).await
                })
            }).collect();

            // Run simulation with invariant checking
            for step in 0..1000 {
                env.advance_time_ms(10);

                // INVARIANT CHECK: SingleActivation (from TLA+)
                let active_count = env.registry
                    .active_instances(&actor_id)
                    .await
                    .len();

                assert!(
                    active_count <= 1,
                    "SingleActivation violated at step {}: {} instances active",
                    step, active_count
                );

                // Randomly inject node crash (like TLA+ NodeCrash action)
                if env.rng.gen_bool(0.01) {
                    env.crash_random_node().await;
                }
            }

            Ok(())
        });

    assert!(result.is_ok(), "DST failed: {:?}", result.err());
}

/// DST test derived from TLA+ spec: actor_activation.tla
/// Verifies: ActivationEventuallySucceeds under faults
#[test]
fn test_activation_eventually_succeeds() {
    let config = SimConfig::from_env_or_random();

    let result = Simulation::new(config)
        .with_fault(FaultConfig::new(FaultType::NetworkPartition, 0.1))
        .run(|env| async move {
            let actor_id = ActorId::new("test", "actor1");

            // Request activation
            let start = env.clock.now();
            let mut activated = false;

            // Should eventually succeed or explicitly reject
            for _ in 0..100 {
                match env.registry.activate(&actor_id).await {
                    Ok(_) => {
                        activated = true;
                        break;
                    }
                    Err(e) if e.is_retriable() => {
                        env.advance_time_ms(100);
                        continue;
                    }
                    Err(e) => {
                        // Explicit rejection is acceptable
                        return Ok(());
                    }
                }
            }

            // PROPERTY: Eventually succeeds (bounded liveness)
            assert!(
                activated || env.clock.elapsed_since(start) < Duration::from_secs(30),
                "ActivationEventuallySucceeds violated: neither activated nor rejected"
            );

            Ok(())
        });

    assert!(result.is_ok());
}
```

### 1.3 Frontend Pipeline: Decision → State Machine → Property Test

**Step 1: Frontend/UX Decision Record**

```markdown
# docs/decisions/ui-003-actor-list-realtime.md

## Status
Accepted

## Context
Users need to see actor state changes in real-time without manual refresh.
The dashboard shows a list of actors with their current status.

## Decision
- Actor list subscribes to WebSocket updates
- Updates are applied optimistically for user actions
- Stale data is visually indicated
- Connection loss shows reconnecting state

## UX Requirements
1. User always sees current connection status
2. User's own actions reflect immediately (optimistic)
3. External updates appear within 1 second
4. Errors are clearly communicated

## States to Model
- disconnected → connecting → connected → updating → error
- Each state has specific UI representation

## Properties to Verify
1. ConnectionStatusVisible: user always knows connection state
2. OptimisticUpdate: user action → immediate visual feedback
3. EventualConsistency: server state → UI state within 1s
4. ErrorRecovery: error → retry → eventually connected or clear error
```

**Step 2: State Machine Specification (XState)**

```typescript
// specs/ui/actorList.machine.ts

import { createMachine, assign } from 'xstate';

interface ActorListContext {
  actors: Actor[];
  error: string | null;
  lastUpdate: number;
  pendingOptimistic: Map<string, Actor>;
}

type ActorListEvent =
  | { type: 'CONNECT' }
  | { type: 'CONNECTED' }
  | { type: 'DISCONNECT' }
  | { type: 'ERROR'; error: string }
  | { type: 'ACTORS_UPDATE'; actors: Actor[] }
  | { type: 'OPTIMISTIC_CREATE'; actor: Actor }
  | { type: 'OPTIMISTIC_CONFIRM'; actorId: string }
  | { type: 'OPTIMISTIC_REJECT'; actorId: string; error: string }
  | { type: 'RETRY' };

export const actorListMachine = createMachine<ActorListContext, ActorListEvent>({
  id: 'actorList',
  initial: 'disconnected',

  context: {
    actors: [],
    error: null,
    lastUpdate: 0,
    pendingOptimistic: new Map(),
  },

  states: {
    disconnected: {
      // PROPERTY: ConnectionStatusVisible - UI shows "Disconnected"
      meta: { uiState: 'disconnected', statusVisible: true },
      on: {
        CONNECT: 'connecting',
      },
    },

    connecting: {
      // PROPERTY: ConnectionStatusVisible - UI shows "Connecting..."
      meta: { uiState: 'connecting', statusVisible: true },
      on: {
        CONNECTED: 'connected',
        ERROR: {
          target: 'error',
          actions: assign({ error: (_, e) => e.error }),
        },
      },
      after: {
        // Timeout after 10 seconds
        10000: {
          target: 'error',
          actions: assign({ error: 'Connection timeout' }),
        },
      },
    },

    connected: {
      // PROPERTY: ConnectionStatusVisible - UI shows "Connected" (can be subtle)
      meta: { uiState: 'connected', statusVisible: true },
      on: {
        DISCONNECT: 'disconnected',
        ERROR: {
          target: 'error',
          actions: assign({ error: (_, e) => e.error }),
        },
        ACTORS_UPDATE: {
          actions: assign({
            actors: (_, e) => e.actors,
            lastUpdate: () => Date.now(),
          }),
        },
        OPTIMISTIC_CREATE: {
          // PROPERTY: OptimisticUpdate - immediately add to list
          actions: assign({
            actors: (ctx, e) => [...ctx.actors, { ...e.actor, _optimistic: true }],
            pendingOptimistic: (ctx, e) =>
              new Map(ctx.pendingOptimistic).set(e.actor.id, e.actor),
          }),
        },
        OPTIMISTIC_CONFIRM: {
          // Server confirmed - remove optimistic flag
          actions: assign({
            actors: (ctx, e) =>
              ctx.actors.map(a =>
                a.id === e.actorId ? { ...a, _optimistic: false } : a
              ),
            pendingOptimistic: (ctx, e) => {
              const map = new Map(ctx.pendingOptimistic);
              map.delete(e.actorId);
              return map;
            },
          }),
        },
        OPTIMISTIC_REJECT: {
          // Server rejected - remove from list, show error
          actions: assign({
            actors: (ctx, e) => ctx.actors.filter(a => a.id !== e.actorId),
            pendingOptimistic: (ctx, e) => {
              const map = new Map(ctx.pendingOptimistic);
              map.delete(e.actorId);
              return map;
            },
            error: (_, e) => e.error,
          }),
        },
      },
    },

    error: {
      // PROPERTY: ConnectionStatusVisible - UI shows error message
      meta: { uiState: 'error', statusVisible: true },
      on: {
        RETRY: 'connecting',
        DISCONNECT: 'disconnected',
      },
      after: {
        // PROPERTY: ErrorRecovery - auto-retry after 5 seconds
        5000: 'connecting',
      },
    },
  },
});

// INVARIANTS (to be verified by tests)
export const actorListInvariants = {
  // From decision: "User always sees current connection status"
  connectionStatusVisible: (state: any) => state.meta?.statusVisible === true,

  // From decision: "User's own actions reflect immediately"
  optimisticUpdateImmediate: (context: ActorListContext, event: ActorListEvent) => {
    if (event.type === 'OPTIMISTIC_CREATE') {
      return context.actors.some(a => a.id === event.actor.id);
    }
    return true;
  },

  // From decision: "Errors are clearly communicated"
  errorStateHasMessage: (state: any, context: ActorListContext) => {
    if (state.matches('error')) {
      return context.error !== null && context.error.length > 0;
    }
    return true;
  },
};
```

**Step 3: Property-Based Tests for UI State Machine**

```typescript
// tests/properties/actorList.property.test.ts

import { fc } from 'fast-check';
import { interpret } from 'xstate';
import { actorListMachine, actorListInvariants } from '../../specs/ui/actorList.machine';

describe('ActorList State Machine Properties', () => {
  // Generate arbitrary event sequences
  const eventArbitrary = fc.oneof(
    fc.constant({ type: 'CONNECT' as const }),
    fc.constant({ type: 'CONNECTED' as const }),
    fc.constant({ type: 'DISCONNECT' as const }),
    fc.record({
      type: fc.constant('ERROR' as const),
      error: fc.string({ minLength: 1 }),
    }),
    fc.record({
      type: fc.constant('ACTORS_UPDATE' as const),
      actors: fc.array(fc.record({
        id: fc.uuid(),
        name: fc.string(),
        status: fc.oneof(fc.constant('active'), fc.constant('inactive')),
      })),
    }),
    fc.record({
      type: fc.constant('OPTIMISTIC_CREATE' as const),
      actor: fc.record({
        id: fc.uuid(),
        name: fc.string(),
        status: fc.constant('pending'),
      }),
    }),
    fc.constant({ type: 'RETRY' as const }),
  );

  /**
   * PROPERTY 1: ConnectionStatusVisible
   * Derived from: "User always sees current connection status"
   *
   * For ANY sequence of events, the UI always shows connection status.
   */
  it('connection status is always visible (any event sequence)', () => {
    fc.assert(
      fc.property(
        fc.array(eventArbitrary, { minLength: 0, maxLength: 50 }),
        (events) => {
          const service = interpret(actorListMachine).start();

          for (const event of events) {
            service.send(event);

            // INVARIANT: Status is always visible
            const state = service.getSnapshot();
            expect(actorListInvariants.connectionStatusVisible(state)).toBe(true);
          }

          service.stop();
        }
      ),
      { numRuns: 1000 }
    );
  });

  /**
   * PROPERTY 2: OptimisticUpdate
   * Derived from: "User's own actions reflect immediately"
   *
   * When user creates an actor, it appears in the list IMMEDIATELY
   * (same tick, before server response).
   */
  it('optimistic updates appear immediately', () => {
    fc.assert(
      fc.property(
        fc.uuid(),
        fc.string({ minLength: 1 }),
        (actorId, actorName) => {
          const service = interpret(actorListMachine).start();

          // Get to connected state
          service.send({ type: 'CONNECT' });
          service.send({ type: 'CONNECTED' });

          // Count actors before
          const beforeCount = service.getSnapshot().context.actors.length;

          // Optimistic create
          service.send({
            type: 'OPTIMISTIC_CREATE',
            actor: { id: actorId, name: actorName, status: 'pending' },
          });

          // PROPERTY: Actor appears immediately (same tick)
          const afterCount = service.getSnapshot().context.actors.length;
          expect(afterCount).toBe(beforeCount + 1);

          // PROPERTY: Actor is in the list
          const hasActor = service.getSnapshot().context.actors
            .some(a => a.id === actorId);
          expect(hasActor).toBe(true);

          service.stop();
        }
      ),
      { numRuns: 500 }
    );
  });

  /**
   * PROPERTY 3: ErrorRecovery
   * Derived from: "error → retry → eventually connected or clear error"
   *
   * From any error state, system eventually recovers or shows clear error.
   */
  it('errors lead to recovery or clear error state', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        fc.array(
          fc.oneof(
            fc.constant({ type: 'RETRY' as const }),
            fc.constant({ type: 'CONNECTED' as const }),
            fc.record({
              type: fc.constant('ERROR' as const),
              error: fc.string({ minLength: 1 }),
            }),
          ),
          { minLength: 1, maxLength: 20 }
        ),
        (initialError, recoveryEvents) => {
          const service = interpret(actorListMachine).start();

          // Get to error state
          service.send({ type: 'CONNECT' });
          service.send({ type: 'ERROR', error: initialError });
          expect(service.getSnapshot().matches('error')).toBe(true);

          // Apply recovery events
          for (const event of recoveryEvents) {
            service.send(event);
          }

          // PROPERTY: Either recovered (connected) or still has clear error
          const state = service.getSnapshot();
          const isRecovered = state.matches('connected') || state.matches('connecting');
          const hasClearError = state.matches('error') &&
            state.context.error !== null &&
            state.context.error.length > 0;

          expect(isRecovered || hasClearError).toBe(true);

          service.stop();
        }
      ),
      { numRuns: 500 }
    );
  });

  /**
   * PROPERTY 4: NoOrphanedOptimistic
   * Derived from: Consistency requirement
   *
   * Optimistic updates are either confirmed or rejected, never orphaned.
   */
  it('optimistic updates are never orphaned', () => {
    fc.assert(
      fc.property(
        fc.uuid(),
        fc.boolean(), // true = confirm, false = reject
        (actorId, willConfirm) => {
          const service = interpret(actorListMachine).start();

          service.send({ type: 'CONNECT' });
          service.send({ type: 'CONNECTED' });

          // Create optimistic
          service.send({
            type: 'OPTIMISTIC_CREATE',
            actor: { id: actorId, name: 'Test', status: 'pending' },
          });

          // Pending should exist
          expect(service.getSnapshot().context.pendingOptimistic.has(actorId)).toBe(true);

          // Confirm or reject
          if (willConfirm) {
            service.send({ type: 'OPTIMISTIC_CONFIRM', actorId });
          } else {
            service.send({ type: 'OPTIMISTIC_REJECT', actorId, error: 'Failed' });
          }

          // PROPERTY: No longer pending
          expect(service.getSnapshot().context.pendingOptimistic.has(actorId)).toBe(false);

          service.stop();
        }
      ),
      { numRuns: 500 }
    );
  });
});
```

### 1.4 Cross-Stack Pipeline: Contract → Test

**Step 1: API Contract Decision**

```markdown
# docs/adr/008-api-contract-versioning.md

## Decision
Use TypeScript contracts (ts-rest + zod) shared between frontend and backend.
Backend Rust code generates OpenAPI from the contract.

## Properties
1. ContractMatch: Frontend usage matches contract schema
2. BackendCompliance: Backend responses match contract schema
3. NoBreakingChanges: New versions are backward compatible
```

**Step 2: Typed Contract**

```typescript
// contracts/actors.contract.ts

import { initContract } from '@ts-rest/core';
import { z } from 'zod';

const c = initContract();

// Shared schemas
export const ActorSchema = z.object({
  id: z.string().uuid(),
  name: z.string().min(1).max(100),
  status: z.enum(['inactive', 'activating', 'active', 'deactivating', 'error']),
  createdAt: z.string().datetime(),
  updatedAt: z.string().datetime(),
});

export const CreateActorSchema = z.object({
  name: z.string().min(1).max(100),
  config: z.record(z.unknown()).optional(),
});

export const ErrorSchema = z.object({
  code: z.string(),
  message: z.string(),
  details: z.record(z.unknown()).optional(),
});

// Contract definition
export const actorsContract = c.router({
  list: {
    method: 'GET',
    path: '/api/actors',
    responses: {
      200: z.object({
        actors: z.array(ActorSchema),
        total: z.number(),
      }),
    },
  },

  create: {
    method: 'POST',
    path: '/api/actors',
    body: CreateActorSchema,
    responses: {
      201: ActorSchema,
      400: ErrorSchema,
      409: ErrorSchema, // Already exists
    },
  },

  get: {
    method: 'GET',
    path: '/api/actors/:id',
    pathParams: z.object({ id: z.string().uuid() }),
    responses: {
      200: ActorSchema,
      404: ErrorSchema,
    },
  },

  delete: {
    method: 'DELETE',
    path: '/api/actors/:id',
    pathParams: z.object({ id: z.string().uuid() }),
    responses: {
      204: z.void(),
      404: ErrorSchema,
    },
  },
});
```

**Step 3: Contract Compliance Tests**

```typescript
// tests/contracts/actors.contract.test.ts

import { actorsContract, ActorSchema } from '../../contracts/actors.contract';

describe('Actors API Contract Compliance', () => {
  const baseUrl = process.env.API_URL || 'http://localhost:8080';

  /**
   * PROPERTY: BackendCompliance
   * Backend responses must match contract schema
   */
  describe('Backend Response Compliance', () => {
    it('GET /api/actors returns valid ActorList', async () => {
      const response = await fetch(`${baseUrl}/api/actors`);
      const data = await response.json();

      // Parse with zod - throws if invalid
      const result = actorsContract.list.responses[200].safeParse(data);

      expect(result.success).toBe(true);
      if (!result.success) {
        console.error('Schema violation:', result.error.format());
      }
    });

    it('POST /api/actors returns valid Actor on success', async () => {
      const response = await fetch(`${baseUrl}/api/actors`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'test-actor' }),
      });

      if (response.status === 201) {
        const data = await response.json();
        const result = ActorSchema.safeParse(data);
        expect(result.success).toBe(true);
      } else if (response.status === 400 || response.status === 409) {
        const data = await response.json();
        const result = actorsContract.create.responses[400].safeParse(data);
        expect(result.success).toBe(true);
      }
    });
  });

  /**
   * PROPERTY: ContractMatch
   * Frontend client usage matches contract
   */
  describe('Frontend Client Compliance', () => {
    // These tests verify the frontend API client generates correct requests
    it('createActor sends valid request body', () => {
      const testBody = { name: 'test', config: { key: 'value' } };
      const result = actorsContract.create.body.safeParse(testBody);
      expect(result.success).toBe(true);
    });

    it('rejects invalid request body', () => {
      const invalidBody = { name: '' }; // Too short
      const result = actorsContract.create.body.safeParse(invalidBody);
      expect(result.success).toBe(false);
    });
  });
});
```

---

## 2. Closed-Loop Observability for Full-Stack

### 2.1 The Complete Feedback Loop

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Full-Stack Observability Closed Loop                      │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│                              USERS                                          │
│                                │                                            │
│                                ▼                                            │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                          FRONTEND                                    │   │
│  │                                                                      │   │
│  │   Collect:                                                           │   │
│  │   • Web Vitals (LCP, FID, CLS)                                      │   │
│  │   • JS Errors (stack traces)                                        │   │
│  │   • User Interactions (clicks, navigation)                          │   │
│  │   • Custom Metrics (feature usage)                                  │   │
│  │   • Session Recordings                                              │   │
│  │                                                                      │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 │ HTTP/WebSocket                            │
│                                 │ (with trace context)                      │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                          BACKEND                                     │   │
│  │                                                                      │   │
│  │   Collect:                                                           │   │
│  │   • Distributed Traces (spans)                                      │   │
│  │   • Metrics (latency, throughput, errors)                          │   │
│  │   • Structured Logs                                                 │   │
│  │   • Actor Metrics (activations, invocations)                       │   │
│  │   • Storage Metrics (reads, writes, latency)                       │   │
│  │                                                                      │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    OBSERVABILITY PLATFORM                            │   │
│  │                                                                      │   │
│  │   ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐               │   │
│  │   │ Traces  │  │ Metrics │  │  Logs   │  │   RUM   │               │   │
│  │   │ (Tempo) │  │(Prometh)│  │ (Loki)  │  │(Sentry) │               │   │
│  │   └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘               │   │
│  │        │            │            │            │                     │   │
│  │        └────────────┴─────┬──────┴────────────┘                     │   │
│  │                           │                                         │   │
│  │                           ▼                                         │   │
│  │              ┌────────────────────────┐                             │   │
│  │              │    Correlation &       │                             │   │
│  │              │    Anomaly Detection   │                             │   │
│  │              └───────────┬────────────┘                             │   │
│  │                          │                                          │   │
│  └──────────────────────────┼──────────────────────────────────────────┘   │
│                             │                                               │
│                             ▼                                               │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         EVI TOOLS                                    │   │
│  │                                                                      │   │
│  │   ┌─────────────────────────────────────────────────────────────┐   │   │
│  │   │  obs_anomaly_detect()                                        │   │   │
│  │   │  → "LCP degraded 50% on /actors page"                       │   │   │
│  │   │  → "Backend latency p99 increased from 100ms to 500ms"      │   │   │
│  │   │  → "Error rate spike in actor.create"                       │   │   │
│  │   └─────────────────────────────────────────────────────────────┘   │   │
│  │                              │                                       │   │
│  │                              ▼                                       │   │
│  │   ┌─────────────────────────────────────────────────────────────┐   │   │
│  │   │  obs_correlate()                                             │   │   │
│  │   │  → Frontend LCP correlates with backend storage.write        │   │   │
│  │   │  → Error spike correlates with deployment at 14:30          │   │   │
│  │   └─────────────────────────────────────────────────────────────┘   │   │
│  │                              │                                       │   │
│  │                              ▼                                       │   │
│  │   ┌─────────────────────────────────────────────────────────────┐   │   │
│  │   │  obs_to_hypothesis()                                         │   │   │
│  │   │  → "Storage contention causing slow actor creation"         │   │   │
│  │   │  → "Missing index on actor lookup query"                    │   │   │
│  │   └─────────────────────────────────────────────────────────────┘   │   │
│  │                                                                      │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                      INVESTIGATION                                   │   │
│  │                                                                      │   │
│  │   1. exam_start(task="Investigate LCP regression")                  │   │
│  │   2. Explore frontend: index_components, repl_load, sub_llm        │   │
│  │   3. Explore backend: index_symbols, repl_load, sub_llm            │   │
│  │   4. Verify hypothesis: dst_run(scenario="storage_contention")     │   │
│  │   5. Fix: Implement solution                                        │   │
│  │   6. Test: verify_e2e, dst_run                                      │   │
│  │                                                                      │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 ▼                                           │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                        DEPLOYMENT                                    │   │
│  │                                                                      │   │
│  │   1. Deploy fix to staging                                          │   │
│  │   2. Verify: obs_vitals_query (staging)                             │   │
│  │   3. Deploy to production                                           │   │
│  │   4. Monitor: obs_anomaly_detect (continuous)                       │   │
│  │                                                                      │   │
│  └──────────────────────────────┬──────────────────────────────────────┘   │
│                                 │                                           │
│                                 │                                           │
│                    ┌────────────┴────────────┐                              │
│                    │                         │                              │
│                    ▼                         │                              │
│              LOOP CLOSES                     │                              │
│           (back to USERS)                    │                              │
│                                              │                              │
│                    ┌─────────────────────────┘                              │
│                    │                                                        │
│                    ▼                                                        │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                     LEARNING FEEDBACK                                │   │
│  │                                                                      │   │
│  │   • If anomaly was from missing spec → Add spec/test                │   │
│  │   • If anomaly was from untested scenario → Add DST test            │   │
│  │   • If anomaly was from missing monitoring → Add metric/alert       │   │
│  │   • Update runbooks with investigation pattern                      │   │
│  │                                                                      │   │
│  │   obs_to_spec()     → Generate spec from incident                   │   │
│  │   obs_to_dst()      → Generate DST scenario from incident           │   │
│  │   obs_to_alert()    → Generate alert rule from incident             │   │
│  │                                                                      │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Observability Data Sources

#### Frontend Telemetry

```typescript
// instrumentation/frontend.ts

import * as Sentry from '@sentry/react';
import { onCLS, onFID, onLCP } from 'web-vitals';
import { trace, context, propagation } from '@opentelemetry/api';

// Initialize Sentry for errors + performance
Sentry.init({
  dsn: process.env.SENTRY_DSN,
  integrations: [
    new Sentry.BrowserTracing({
      // Propagate trace context to backend
      tracePropagationTargets: ['localhost', 'api.kelpie.dev'],
    }),
  ],
  tracesSampleRate: 0.1, // 10% of transactions
});

// Web Vitals → Custom metrics
function sendVital(metric: Metric) {
  // Send to metrics backend
  navigator.sendBeacon('/api/telemetry/vitals', JSON.stringify({
    name: metric.name,
    value: metric.value,
    id: metric.id,
    page: window.location.pathname,
    timestamp: Date.now(),
  }));

  // Also send to Sentry for correlation
  Sentry.setMeasurement(metric.name, metric.value, 'millisecond');
}

onLCP(sendVital);  // Largest Contentful Paint
onFID(sendVital);  // First Input Delay
onCLS(sendVital);  // Cumulative Layout Shift

// Custom span for user interactions
export function traceUserAction(actionName: string, fn: () => Promise<void>) {
  const tracer = trace.getTracer('frontend');

  return tracer.startActiveSpan(actionName, async (span) => {
    try {
      span.setAttribute('user.action', actionName);
      span.setAttribute('page', window.location.pathname);
      await fn();
    } catch (error) {
      span.recordException(error as Error);
      throw error;
    } finally {
      span.end();
    }
  });
}

// Example usage in React
function CreateActorButton() {
  const handleClick = () => {
    traceUserAction('actor.create.click', async () => {
      // This span will be connected to the backend trace
      const response = await fetch('/api/actors', {
        method: 'POST',
        // Trace context automatically propagated by Sentry/OTEL
      });
    });
  };
}
```

#### Backend Telemetry

```rust
// crates/kelpie-server/src/telemetry.rs

use opentelemetry::{global, trace::Tracer};
use tracing_subscriber::layer::SubscriberExt;

pub fn init_telemetry() {
    // OTLP exporter to Tempo/Jaeger
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic())
        .install_batch(opentelemetry::runtime::Tokio)
        .expect("Failed to initialize tracer");

    // Prometheus metrics
    let prometheus = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(prometheus.clone())
        .build()
        .expect("Failed to initialize Prometheus exporter");

    // Metrics we collect
    lazy_static! {
        pub static ref ACTOR_ACTIVATIONS: Counter = Counter::new(
            "kelpie_actor_activations_total",
            "Total actor activations"
        ).unwrap();

        pub static ref ACTOR_ACTIVATION_DURATION: Histogram = Histogram::with_opts(
            HistogramOpts::new(
                "kelpie_actor_activation_duration_seconds",
                "Actor activation duration"
            ).buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0])
        ).unwrap();

        pub static ref STORAGE_OPERATION_DURATION: HistogramVec = HistogramVec::new(
            HistogramOpts::new(
                "kelpie_storage_operation_duration_seconds",
                "Storage operation duration"
            ),
            &["operation", "result"]
        ).unwrap();
    }
}

// Instrument handlers with tracing
#[tracing::instrument(skip(state))]
pub async fn create_actor(
    State(state): State<AppState>,
    Json(request): Json<CreateActorRequest>,
) -> Result<Json<Actor>, ApiError> {
    ACTOR_ACTIVATIONS.inc();
    let timer = ACTOR_ACTIVATION_DURATION.start_timer();

    let result = state.registry.create_actor(&request.name).await;

    timer.observe_duration();

    match result {
        Ok(actor) => {
            tracing::info!(actor_id = %actor.id, "Actor created");
            Ok(Json(actor))
        }
        Err(e) => {
            tracing::error!(error = %e, "Actor creation failed");
            Err(e.into())
        }
    }
}
```

### 2.3 EVI Observability Tools

```python
# evi/tools/observability.py

class ObservabilityTools:
    """Tools for querying and analyzing observability data."""

    def __init__(self, config: ObservabilityConfig):
        self.traces_client = TempoClient(config.traces_endpoint)
        self.metrics_client = PrometheusClient(config.metrics_endpoint)
        self.logs_client = LokiClient(config.logs_endpoint)
        self.rum_client = SentryClient(config.sentry_dsn)

    # ─────────────────────────────────────────────────────────────────
    # QUERY TOOLS
    # ─────────────────────────────────────────────────────────────────

    async def obs_trace_query(
        self,
        service: str | None = None,
        operation: str | None = None,
        min_duration_ms: int | None = None,
        status: str | None = None,  # "ok" | "error"
        time_range: str = "1h",
        limit: int = 100,
    ) -> list[TraceSummary]:
        """Query distributed traces by criteria."""
        query = TraceQuery(
            service=service,
            operation=operation,
            min_duration=timedelta(milliseconds=min_duration_ms) if min_duration_ms else None,
            status=status,
            time_range=parse_time_range(time_range),
            limit=limit,
        )
        return await self.traces_client.query(query)

    async def obs_trace_get(self, trace_id: str) -> TraceDetail:
        """Get full trace with all spans."""
        return await self.traces_client.get_trace(trace_id)

    async def obs_metrics_query(
        self,
        query: str,  # PromQL
        time_range: str = "1h",
        step: str = "1m",
    ) -> MetricsResult:
        """Query Prometheus metrics."""
        return await self.metrics_client.query_range(
            query=query,
            start=parse_time_range(time_range)[0],
            end=datetime.now(),
            step=parse_duration(step),
        )

    async def obs_logs_query(
        self,
        query: str,  # LogQL
        time_range: str = "1h",
        limit: int = 1000,
    ) -> list[LogEntry]:
        """Query Loki logs."""
        return await self.logs_client.query(query, time_range, limit)

    async def obs_vitals_query(
        self,
        page: str | None = None,
        metric: str | None = None,  # "LCP" | "FID" | "CLS"
        percentile: int = 75,
        time_range: str = "24h",
    ) -> VitalsResult:
        """Query Core Web Vitals from RUM."""
        return await self.rum_client.query_vitals(
            page=page,
            metric=metric,
            percentile=percentile,
            time_range=time_range,
        )

    async def obs_errors_query(
        self,
        service: str | None = None,
        time_range: str = "1h",
        group_by: str = "message",
    ) -> list[ErrorGroup]:
        """Query error groups from frontend and backend."""
        frontend_errors = await self.rum_client.query_errors(time_range)
        backend_errors = await self.logs_client.query(
            '{level="error"}', time_range
        )
        return self._group_errors(frontend_errors + backend_errors, group_by)

    # ─────────────────────────────────────────────────────────────────
    # ANALYSIS TOOLS
    # ─────────────────────────────────────────────────────────────────

    async def obs_anomaly_detect(
        self,
        metrics: list[str] | None = None,
        time_range: str = "1h",
        sensitivity: str = "medium",  # "low" | "medium" | "high"
    ) -> list[Anomaly]:
        """Detect anomalies in metrics and traces.

        Checks:
        - Latency spikes (p99 > 2x baseline)
        - Error rate increases (> 1% or 5x baseline)
        - Web Vitals degradation (> 20% worse)
        - Throughput drops (> 50% decrease)
        """
        anomalies = []

        # Check latency
        latency_query = 'histogram_quantile(0.99, rate(kelpie_http_request_duration_seconds_bucket[5m]))'
        latency = await self.obs_metrics_query(latency_query, time_range)
        if latency.has_spike(threshold=2.0):
            anomalies.append(Anomaly(
                type="latency_spike",
                metric="http_request_duration_p99",
                current=latency.current_value,
                baseline=latency.baseline_value,
                severity="high" if latency.spike_ratio > 5 else "medium",
            ))

        # Check error rate
        error_query = 'rate(kelpie_http_requests_total{status=~"5.."}[5m]) / rate(kelpie_http_requests_total[5m])'
        errors = await self.obs_metrics_query(error_query, time_range)
        if errors.current_value > 0.01:  # > 1% error rate
            anomalies.append(Anomaly(
                type="error_rate_spike",
                metric="http_error_rate",
                current=errors.current_value,
                baseline=errors.baseline_value,
                severity="critical" if errors.current_value > 0.05 else "high",
            ))

        # Check Web Vitals
        vitals = await self.obs_vitals_query(time_range=time_range)
        for vital in ['LCP', 'FID', 'CLS']:
            if vitals.degraded(vital, threshold=0.2):  # 20% worse
                anomalies.append(Anomaly(
                    type="web_vital_degraded",
                    metric=vital,
                    current=vitals.current(vital),
                    baseline=vitals.baseline(vital),
                    severity="medium",
                ))

        return anomalies

    async def obs_correlate(
        self,
        anomaly: Anomaly,
        time_range: str = "1h",
    ) -> CorrelationResult:
        """Correlate an anomaly with other signals.

        Finds:
        - Deployments around the time
        - Related traces
        - Correlated metrics
        - Related log entries
        """
        timestamp = anomaly.detected_at
        window = parse_time_range(time_range)

        # Find traces around the anomaly time
        traces = await self.obs_trace_query(
            min_duration_ms=int(anomaly.current * 1000) if anomaly.type == "latency_spike" else None,
            status="error" if anomaly.type == "error_rate_spike" else None,
            time_range=time_range,
        )

        # Find deployments
        deployments = await self._find_deployments(window)

        # Find correlated metrics
        correlated_metrics = await self._find_correlated_metrics(
            anomaly.metric,
            window,
        )

        # Find related logs
        logs = await self.obs_logs_query(
            '{level=~"error|warn"}',
            time_range=time_range,
        )

        return CorrelationResult(
            anomaly=anomaly,
            traces=traces[:10],  # Top 10 relevant traces
            deployments=deployments,
            correlated_metrics=correlated_metrics,
            related_logs=logs[:50],
        )

    async def obs_to_hypothesis(
        self,
        anomaly: Anomaly,
        correlation: CorrelationResult,
    ) -> list[Hypothesis]:
        """Generate investigation hypotheses from anomaly and correlations.

        Uses sub-LLM to analyze the data and suggest hypotheses.
        """
        context = f"""
Anomaly detected:
- Type: {anomaly.type}
- Metric: {anomaly.metric}
- Current value: {anomaly.current}
- Baseline value: {anomaly.baseline}
- Severity: {anomaly.severity}

Correlated signals:
- Deployments: {[d.description for d in correlation.deployments]}
- Trace patterns: {self._summarize_traces(correlation.traces)}
- Correlated metrics: {correlation.correlated_metrics}
- Error patterns in logs: {self._summarize_logs(correlation.related_logs)}
"""

        # Use sub-LLM to generate hypotheses
        result = await self.sub_llm.analyze_content(
            content=context,
            query="""
Based on this observability data, generate 2-3 hypotheses for the root cause.
For each hypothesis provide:
1. Description of the suspected cause
2. Confidence level (high/medium/low)
3. What evidence supports this
4. What to investigate next

Format as JSON: [{"hypothesis": "...", "confidence": "...", "evidence": "...", "next_steps": "..."}]
""",
        )

        return [Hypothesis(**h) for h in json.loads(result.response)]

    # ─────────────────────────────────────────────────────────────────
    # LEARNING TOOLS (Closing the Loop)
    # ─────────────────────────────────────────────────────────────────

    async def obs_to_spec(
        self,
        incident: IncidentReport,
    ) -> SpecSuggestion:
        """Generate specification from incident to prevent recurrence.

        If an incident reveals a missing invariant, suggest adding it.
        """
        context = f"""
Incident: {incident.title}
Root cause: {incident.root_cause}
Timeline: {incident.timeline}
Resolution: {incident.resolution}
"""

        result = await self.sub_llm.analyze_content(
            content=context,
            query="""
Based on this incident, suggest a specification that would catch this earlier:

1. For backend issues: Suggest a TLA+ invariant or DST test scenario
2. For frontend issues: Suggest a state machine property or E2E test
3. For cross-stack issues: Suggest a contract test or integration test

Format as JSON:
{
  "spec_type": "tla+|state_machine|contract|dst|e2e",
  "description": "What the spec should verify",
  "pseudo_spec": "Rough specification code",
  "test_scenario": "How to test this"
}
""",
        )

        return SpecSuggestion(**json.loads(result.response))

    async def obs_to_dst(
        self,
        incident: IncidentReport,
    ) -> str:
        """Generate DST test scenario from incident.

        Creates a DST test that reproduces the incident conditions.
        """
        context = f"""
Incident: {incident.title}
Root cause: {incident.root_cause}
Conditions: {incident.conditions}
Affected components: {incident.affected_components}
"""

        result = await self.sub_llm.analyze_content(
            content=context,
            query="""
Generate a Rust DST test that reproduces these incident conditions.

The test should:
1. Set up the initial state
2. Inject the faults that caused the incident
3. Verify the invariant that was violated (or should have caught this)

Output valid Rust code for a DST test.
""",
        )

        return result.response

    async def obs_to_alert(
        self,
        incident: IncidentReport,
    ) -> AlertRule:
        """Generate alert rule from incident.

        Creates a Prometheus/Grafana alert that would catch this earlier.
        """
        context = f"""
Incident: {incident.title}
Detection delay: {incident.detection_delay}
Key metrics during incident: {incident.key_metrics}
Threshold that should have triggered: {incident.suggested_threshold}
"""

        result = await self.sub_llm.analyze_content(
            content=context,
            query="""
Generate a Prometheus alerting rule that would detect this incident earlier.

Output as YAML:
groups:
- name: ...
  rules:
  - alert: ...
    expr: ...
    for: ...
    labels: ...
    annotations: ...
""",
        )

        return AlertRule.from_yaml(result.response)
```

### 2.4 Complete Investigation Example

```python
# Example: Full-stack investigation triggered by Web Vital degradation

async def investigate_lcp_degradation():
    """
    Complete investigation flow for LCP (Largest Contentful Paint) degradation.
    Demonstrates the closed-loop from detection to fix to prevention.
    """

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 1: DETECT ANOMALY
    # ═══════════════════════════════════════════════════════════════════════

    anomalies = await obs_anomaly_detect(time_range="1h")
    # Result: [Anomaly(type="web_vital_degraded", metric="LCP",
    #          current=3.5, baseline=1.2, severity="medium")]

    lcp_anomaly = next(a for a in anomalies if a.metric == "LCP")
    print(f"🚨 LCP degraded: {lcp_anomaly.baseline}s → {lcp_anomaly.current}s")

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 2: CORRELATE WITH OTHER SIGNALS
    # ═══════════════════════════════════════════════════════════════════════

    correlation = await obs_correlate(lcp_anomaly, time_range="2h")
    # Result:
    # - Deployment at 14:30 (30 min before anomaly)
    # - Backend traces show storage.read p99: 100ms → 800ms
    # - Correlated metric: kelpie_storage_operation_duration_seconds

    print(f"📊 Correlations found:")
    print(f"   - Deployments: {[d.version for d in correlation.deployments]}")
    print(f"   - Correlated metrics: {correlation.correlated_metrics}")
    print(f"   - Slow traces: {len(correlation.traces)}")

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 3: GENERATE HYPOTHESES
    # ═══════════════════════════════════════════════════════════════════════

    hypotheses = await obs_to_hypothesis(lcp_anomaly, correlation)
    # Result:
    # [
    #   Hypothesis(
    #     hypothesis="Missing database index on actor_states table",
    #     confidence="high",
    #     evidence="storage.read latency correlates with LCP, deployment added new query",
    #     next_steps="Check query plan for actor_states lookup"
    #   ),
    #   Hypothesis(
    #     hypothesis="N+1 query in actor list endpoint",
    #     confidence="medium",
    #     evidence="Multiple storage.read spans per request in traces",
    #     next_steps="Analyze trace waterfall for /api/actors"
    #   )
    # ]

    print(f"💡 Hypotheses:")
    for h in hypotheses:
        print(f"   - [{h.confidence}] {h.hypothesis}")

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 4: EXPLORE CODE (EVI EXPLORATION)
    # ═══════════════════════════════════════════════════════════════════════

    # Start examination
    await exam_start(task="Investigate LCP regression", scope=["kelpie-server", "dashboard"])

    # Explore backend - find the handler
    symbols = await index_symbols(pattern="list_actors", kind="fn")
    # Result: [Symbol(name="list_actors", file="handlers/actors.rs", line=45)]

    # Load and analyze with RLM
    await repl_load(pattern="crates/kelpie-server/src/handlers/actors.rs", var_name="handler_code")
    handler_analysis = await repl_exec(code="""
analysis = sub_llm(handler_code['actors.rs'], '''
    1. How does this handler fetch actor list?
    2. Are there any N+1 queries? (loop with individual fetches)
    3. What storage operations are called?
    4. ISSUES: Performance concerns?
''')
result = analysis
""")
    # Result: "Handler calls storage.get_actor() in a loop for each actor ID.
    #          This is an N+1 query pattern - should use batch fetch."

    # Explore frontend
    await repl_load(pattern="dashboard/src/pages/Actors.tsx", var_name="page_code")
    frontend_analysis = await repl_exec(code="""
analysis = sub_llm(page_code['Actors.tsx'], '''
    1. How does this page fetch actor data?
    2. What happens during loading?
    3. ISSUES: Missing loading optimization? Suspense boundaries?
''')
result = analysis
""")
    # Result: "Page fetches actor list on mount, no skeleton loading,
    #          entire page waits for data before rendering anything."

    # Record findings
    await exam_record(
        component="kelpie-server",
        summary="N+1 query in list_actors handler",
        issues=[{
            "severity": "high",
            "description": "Handler fetches each actor individually in a loop",
            "evidence": "handlers/actors.rs:67 - for loop with get_actor()"
        }]
    )

    await exam_record(
        component="dashboard",
        summary="No progressive loading for actor list",
        issues=[{
            "severity": "medium",
            "description": "Page blocks on full data fetch, no skeleton",
            "evidence": "pages/Actors.tsx:23 - single loading state for entire page"
        }]
    )

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 5: VERIFY HYPOTHESIS (DST)
    # ═══════════════════════════════════════════════════════════════════════

    # Run DST to confirm N+1 causes latency under load
    dst_result = await dst_run(
        scenario="high_actor_count",
        config={
            "actor_count": 100,
            "concurrent_requests": 10,
            "storage_latency_ms": 10,  # 10ms per storage call
        }
    )
    # Result: With 100 actors × 10ms = 1000ms latency (confirms N+1)

    print(f"✅ DST confirmed: {dst_result.p99_latency}ms latency with N+1")

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 6: FIX
    # ═══════════════════════════════════════════════════════════════════════

    # Backend fix: Batch fetch
    # (Human implements this)
    # storage.get_actors_batch(actor_ids) instead of loop

    # Frontend fix: Progressive loading
    # (Human implements this)
    # Add skeleton loading, maybe pagination

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 7: VERIFY FIX
    # ═══════════════════════════════════════════════════════════════════════

    # Run DST again
    dst_result_after = await dst_run(scenario="high_actor_count", config={...})
    assert dst_result_after.p99_latency < 100  # Now 100ms total (single batch)

    # Run E2E
    e2e_result = await verify_e2e(test="actor-list-loads-quickly")
    assert e2e_result.passed

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 8: CLOSE THE LOOP - PREVENT RECURRENCE
    # ═══════════════════════════════════════════════════════════════════════

    incident = IncidentReport(
        title="LCP regression due to N+1 query",
        root_cause="list_actors handler fetches actors individually in a loop",
        conditions="High actor count (>50)",
        resolution="Implemented batch fetch",
        affected_components=["kelpie-server", "dashboard"],
    )

    # Generate spec to catch this earlier
    spec = await obs_to_spec(incident)
    # Result: SpecSuggestion(
    #   spec_type="dst",
    #   description="Actor list latency should scale O(1), not O(n)",
    #   pseudo_spec="assert!(latency_100_actors < latency_10_actors * 2)",
    #   test_scenario="Measure list latency with varying actor counts"
    # )

    # Generate DST test
    dst_test = await obs_to_dst(incident)
    # Result: Rust code for test_actor_list_scales_correctly_dst

    # Generate alert
    alert = await obs_to_alert(incident)
    # Result: Alert rule for storage_read_count_per_request > 10

    print("🔄 LOOP CLOSED:")
    print(f"   - Added spec: {spec.description}")
    print(f"   - Added DST test: test_actor_list_scales_correctly_dst")
    print(f"   - Added alert: {alert.name}")

    # ═══════════════════════════════════════════════════════════════════════
    # STEP 9: DEPLOY AND MONITOR
    # ═══════════════════════════════════════════════════════════════════════

    # After deployment, continue monitoring
    await obs_vitals_query(metric="LCP", page="/actors", time_range="1h")
    # Result: LCP back to baseline (1.2s)

    print("✅ Fix verified in production. LCP back to baseline.")
```

### 2.5 Closed-Loop Summary

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Closed-Loop Summary                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   DETECT           CORRELATE         HYPOTHESIZE       EXPLORE              │
│   ───────          ─────────         ───────────       ───────              │
│   obs_anomaly  →   obs_correlate →   obs_to_hypo  →   exam_start           │
│   _detect          (traces,          thesis           index_*              │
│                    metrics,                           repl_load            │
│                    logs, RUM)                         sub_llm              │
│                                                                             │
│        │                                                   │                │
│        │                                                   │                │
│        ▼                                                   ▼                │
│                                                                             │
│   VERIFY           FIX              DEPLOY            MONITOR               │
│   ──────           ───              ──────            ───────               │
│   dst_run      →   (human)      →   CI/CD         →   obs_anomaly          │
│   verify_e2e       implements       pipeline          _detect              │
│   verify_a11y                                         (continuous)         │
│                                                                             │
│        │                                                   │                │
│        │                                                   │                │
│        └───────────────────────┬───────────────────────────┘                │
│                                │                                            │
│                                ▼                                            │
│                                                                             │
│                         LEARN & PREVENT                                     │
│                         ───────────────                                     │
│                         obs_to_spec   → Add invariant/property              │
│                         obs_to_dst    → Add regression test                 │
│                         obs_to_alert  → Add early detection                 │
│                                                                             │
│                                │                                            │
│                                │                                            │
│                                ▼                                            │
│                                                                             │
│                    ┌─────────────────────────────────┐                      │
│                    │  SPECIFICATIONS GROW OVER TIME  │                      │
│                    │                                 │                      │
│                    │  • TLA+ specs from incidents   │                      │
│                    │  • DST scenarios from prod     │                      │
│                    │  • State machines from bugs    │                      │
│                    │  • Alerts from outages         │                      │
│                    │                                 │                      │
│                    │  Each incident makes the        │                      │
│                    │  system more robust             │                      │
│                    └─────────────────────────────────┘                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Tool Summary

### 3.1 Specification Tools

| Tool | Layer | Purpose |
|------|-------|---------|
| `spec_extract` | Backend | Extract TLA+ properties from ADR |
| `tla_generate` | Backend | Generate TLA+ spec from properties |
| `tla_check` | Backend | Run TLC model checker |
| `spec_state_machine` | Frontend | Generate XState from UI decision |
| `spec_property_test` | Frontend | Generate property test from state machine |
| `spec_contract` | Cross-stack | Generate API contract from decision |

### 3.2 Observability Tools

| Tool | Scope | Purpose |
|------|-------|---------|
| `obs_trace_query` | Cross-stack | Query distributed traces |
| `obs_metrics_query` | Backend | Query Prometheus |
| `obs_logs_query` | Backend | Query Loki |
| `obs_vitals_query` | Frontend | Query Web Vitals |
| `obs_errors_query` | Cross-stack | Query errors |
| `obs_anomaly_detect` | Cross-stack | Find anomalies |
| `obs_correlate` | Cross-stack | Correlate signals |
| `obs_to_hypothesis` | Cross-stack | Generate hypotheses |

### 3.3 Learning Tools (Closing the Loop)

| Tool | Purpose |
|------|---------|
| `obs_to_spec` | Generate spec from incident |
| `obs_to_dst` | Generate DST test from incident |
| `obs_to_alert` | Generate alert rule from incident |

---

*This document details the ADR→Spec→Test pipeline and closed-loop observability for full-stack applications.*
