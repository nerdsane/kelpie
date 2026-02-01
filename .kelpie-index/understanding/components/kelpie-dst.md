# kelpie-dst

**Examined:** 2026-01-22T21:29:16.082871+00:00

## Summary

Deterministic Simulation Testing harness with 49 fault types across 10 categories (Storage, Crash, Network, Time, Resource, MCP, LLM, Sandbox, Snapshot, Teleport)

## Connections

- kelpie-core
- kelpie-runtime
- kelpie-storage
- kelpie-vm
- kelpie-agent

## Details

Determinism via: SimConfig seed, DeterministicRng (ChaCha20), SimClock (AtomicU64), seeded fault injection. 13 modules: simulation, fault, clock, rng, network, storage, sandbox, sandbox_io, teleport, time, vm, llm, agent. CRITICAL: Multiple non-determinism bugs found.

## Issues

### [HIGH] RNG state race condition - self.rng borrowed without Cell/Mutex in SimLlmClient

**Evidence:** llm.rs

### [HIGH] DeterministicRng accepted but never used in SimVmFactory::new()

**Evidence:** vm.rs

### [HIGH] HashMap iteration order non-determinism in capture_snapshot() - env_vars order undefined

**Evidence:** sandbox_io.rs

### [HIGH] Non-determinism in concurrent SimTime access - advance_ms() interleaved without sync

**Evidence:** time.rs

### [MEDIUM] Missing seed propagation to FaultInjector, SimNetwork, SimStorage

**Evidence:** lib.rs

### [MEDIUM] Canned response lookup non-deterministic - HashMap should be BTreeMap

**Evidence:** llm.rs

### [MEDIUM] AtomicU64 counter state leaks across tests - id_counter unresettable

**Evidence:** vm.rs
