# kelpie-runtime

**Examined:** 2026-01-22T23:58:18.070918+00:00

## Summary

Local actor runtime with on-demand activation, single-threaded dispatcher, and transactional state persistence

## Connections

- kelpie-core
- kelpie-storage
- kelpie-cluster

## Details

- Virtual actor activation: On-demand via Dispatcher.handle_invoke() - activates on first message
- Single activation guarantee: Enforced via single-threaded dispatcher loop (local only)
- Location transparency: ActorHandle routes by ActorId, physical location hidden
- Linearizability: FIFO mailbox + sequential dispatcher + transactional save_all_transactional()
- State atomicity: Snapshot/rollback on failure, atomic state+KV persistence
- Missing: Distributed coordination, cross-node routing, lease-based activation

## Issues

### [MEDIUM] Single activation guarantee is local-only, no distributed lock/lease

**Evidence:** dispatcher.rs uses HashMap check, no distributed coordination

### [MEDIUM] max_pending_per_actor config defined but unused

**Evidence:** dispatcher.rs DispatcherConfig has field but never checked
