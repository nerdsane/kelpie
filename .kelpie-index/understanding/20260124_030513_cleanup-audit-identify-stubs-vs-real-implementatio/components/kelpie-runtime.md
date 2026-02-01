# kelpie-runtime

**Examined:** 2026-01-24T03:03:11.635590+00:00

## Summary

Single-activation guarantee is NOT ENFORCED. Local mode has TOCTOU race in HashMap check. Distributed mode has race window between get_placement() and try_claim_actor(). No lease/heartbeat mechanism.

## Connections

- kelpie-registry
- kelpie-cluster
- kelpie-storage

## Details

**Analysis of dispatcher.rs (main enforcement point):**

**Local Mode (registry=None):**
- TOCTOU race: `if !self.actors.contains_key(&key)` check followed by non-atomic `activate_actor()` call
- Multiple concurrent requests can pass the check and create duplicate instances
- No mutex protection around activation path

**Distributed Mode (registry=Some):**
- Better: Uses `registry.try_claim_actor()` for atomic placement decision
- Gap: Non-atomic window between `get_placement()` check (line 404) and `try_claim_actor()` (line 450)
- Gap: No lease/heartbeat - actor ownership is permanent until explicit unregister
- Node crash = orphaned actors forever in registry

**activation.rs confirms:**
- `ActiveActor::activate()` has NO locking mechanism
- Multiple concurrent calls succeed independently
- `on_activate()` hook can run multiple times for same actor

**The claim "only one ActiveActor per ActorId can exist" is ASPIRATIONAL, not enforced.**

## Issues

### [HIGH] Local mode TOCTOU race allows duplicate actor activations

**Evidence:** dispatcher.rs:408-427 check-then-act without mutex

### [HIGH] Distributed mode has race window between get_placement() and try_claim_actor()

**Evidence:** dispatcher.rs:404-450 non-atomic sequence

### [HIGH] No lease/heartbeat - node crash orphans actors forever

**Evidence:** dispatcher.rs:450-475 no TTL or health check

### [MEDIUM] unwrap() on mutex lock can panic if poisoned

**Evidence:** dispatcher.rs:411,462 .lock().unwrap()

### [MEDIUM] ActiveActor::activate() lacks any locking mechanism

**Evidence:** activation.rs:108-147 no distributed coordination
