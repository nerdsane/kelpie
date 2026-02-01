# kelpie-runtime

**Examined:** 2026-01-25T03:47:13.156608+00:00

## Summary

Actor runtime with lifecycle state machine. PARTIAL compliance with TLA+ ActorLifecycle spec - state transitions exist but idle timeout never enforced, lifecycle guard uses assert! not runtime check.

## Connections

- kelpie-core
- kelpie-registry
- kelpie-storage

## Details

**What's Implemented:**
- ActivationState enum: Activating → Active → Deactivating → Deactivated
- ActiveActor with idle_timeout field and should_deactivate() method
- process_invocation_with_time() with state assertion
- Deactivation drains mailbox and calls on_deactivate()

**Spec Violations:**
1. **Idle timeout never enforced**: should_deactivate() exists but is NEVER CALLED - dead code
2. **Assert not runtime check**: process_invocation_with_time() uses assert!(state == Active) which is optimized away in release builds
3. **No deactivation guard in dispatcher**: handle_invoke() doesn't check for Deactivating state before routing
4. **Race condition**: Between actors.contains_key() and process_invocation(), deactivation can occur

**What Works:**
- State transitions are properly ordered
- Deactivation drains pending messages
- on_activate/on_deactivate hooks called correctly

## Issues

### [CRITICAL] Idle timeout completely unenforced - should_deactivate() is dead code

**Evidence:** activation.rs:423-429 never called anywhere in codebase

### [HIGH] Lifecycle guard uses assert! - optimized away in release builds

**Evidence:** activation.rs:206 assert!(state == Active)

### [HIGH] No deactivation guard in dispatcher - invokes can race with deactivation

**Evidence:** dispatcher.rs handle_invoke() has no state check

### [MEDIUM] Race between contains_key and process_invocation during deactivation

**Evidence:** dispatcher.rs:285-348
