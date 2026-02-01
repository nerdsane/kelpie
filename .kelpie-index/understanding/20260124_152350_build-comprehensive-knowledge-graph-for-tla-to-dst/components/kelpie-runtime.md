# kelpie-runtime

**Examined:** 2026-01-24T15:19:23.476118+00:00

## Summary

Actor runtime with single-threaded dispatcher, ActivationState machine, backpressure, and transactional state persistence

## Connections

- kelpie-registry
- kelpie-storage
- kelpie-core

## Details

**State Machine (ActivationState):**
- Deactivated → Activating → Active → Deactivating → Deactivated
- Critical transitions require state load/save

**Single Activation (Local Mode):**
- HashMap membership check + activation is atomic due to single-threaded dispatcher
- NO TOCTOU race - commands processed sequentially via command loop

**Single Activation (Distributed Mode):**
- TOCTOU race DETECTED but not PREVENTED at dispatcher.rs:512-530
- get_placement() → try_claim_actor() window allows dual activation
- Race is detected via PlacementDecision::Existing check, client gets error

**Dispatcher Guarantees:**
- Single-threaded command processing (line 480)
- Per-actor single-threadedness
- FIFO message ordering
- Backpressure at handle level (max_pending_per_actor)

**Concurrency:**
- HashMap<String, ActiveActor> NOT shared (dispatcher only)
- pending_counts: Arc<Mutex<HashMap>> for backpressure
- Arc<AtomicUsize> per actor for pending tracking

**Failure Recovery:**
- Actor panic: state rolled back, no auto-reactivation
- Dispatcher crash: no auto-restart
- State persistence failure: transaction rolled back, actor stays active

## Issues

### [MEDIUM] Distributed mode TOCTOU race detected but not prevented - client retry required

**Evidence:** dispatcher.rs:512-530, 639-643

### [MEDIUM] Stale registry entries on node crash - no TTL-based cleanup

**Evidence:** dispatcher.rs:667 missing heartbeat coordination

### [LOW] No auto-restart of dispatcher task on crash

**Evidence:** runtime.rs:175-185
