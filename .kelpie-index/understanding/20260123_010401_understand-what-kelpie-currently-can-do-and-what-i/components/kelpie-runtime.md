# kelpie-runtime

**Examined:** 2026-01-23T01:01:50.422965+00:00

## Summary

Actor runtime with dispatcher, activation lifecycle, mailbox, state persistence

## Connections

- kelpie-core
- kelpie-storage

## Details

**WORKING (23 tests pass):**
- RuntimeBuilder with fluent config (factory, KV store, tokio runtime)
- Dispatcher message routing and actor management
- Actor activation/deactivation lifecycle with state guards
- Mailbox with bounded FIFO queue and capacity limits
- ActorHandle for async invocation with timeouts
- State persistence via KV store (JSON serialization)
- Transactional KV - atomic state + KV persistence
- Multiple independent actors with separate state

**Key features:**
- Single-threaded execution per actor (mailbox queuing)
- Timeout enforcement on invocations
- Graceful error handling with rollback on failure
- Idle timeout detection

## Issues

### [MEDIUM] No distributed lock for single-activation guarantee - only works single-node

**Evidence:** activation.rs lacks distributed coordination

### [LOW] No actor cleanup policy - actors stay in HashMap indefinitely

**Evidence:** dispatcher.rs:max_actors check but no TTL/idle eviction
