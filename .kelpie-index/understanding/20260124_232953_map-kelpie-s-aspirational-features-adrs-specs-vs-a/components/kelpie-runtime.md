# kelpie-runtime

**Examined:** 2026-01-24T23:26:05.446579+00:00

## Summary

COMPLETE - Dispatcher with single-activation enforcement, actor lifecycle with state persistence, transactional KV operations

## Connections

- kelpie-core
- kelpie-storage
- kelpie-registry

## Details

Fully implemented:
- Single-activation: Local mode via HashMap, distributed via Registry claims
- Actor lifecycle: on_activate/on_deactivate hooks, state load/save
- State persistence: Transactional atomic state+KV commits with rollback on error
- Mailbox: Bounded queue with capacity limits
- Registry integration: try_claim_actor, get_placement, unregister_actor
- Backpressure: max_pending_per_actor with atomic counters
- Metrics: record_invocation, record_agent_activated

27 unit tests passing.

## Issues

### [LOW] No timeout on registry operations - could hang indefinitely

**Evidence:** dispatcher.rs:620 - registry.try_claim_actor().await not wrapped in timeout

### [LOW] Pending counter HashMap never cleaned up - minor memory growth

**Evidence:** dispatcher.rs:284-293 - entry never removed after actor deactivates
