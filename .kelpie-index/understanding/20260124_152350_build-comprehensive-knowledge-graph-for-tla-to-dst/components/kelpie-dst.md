# kelpie-dst

**Examined:** 2026-01-24T15:22:01.097903+00:00

## Summary

Deterministic Simulation Testing framework with SimClock, SimStorage, SimNetwork, FaultInjector (41 fault types), and deterministic RNG

## Connections

- kelpie-runtime
- kelpie-storage
- kelpie-core

## Details

**Components:**
- SimClock: Controlled time, no wall-clock dependencies
- DeterministicRng: ChaCha20-based, seeded, forkable
- SimStorage: In-memory KV with size limits
- SimNetwork: Message queue with latency simulation
- FaultInjector: Probabilistic injection (41 fault types)
- SimLlmClient: Deterministic LLM responses

**Fault Types (41 total):**
- Storage: WriteFail, ReadFail, Corruption, Latency, DiskFull
- Crash: BeforeWrite, AfterWrite, DuringTransaction
- Network: Partition, Delay, PacketLoss, MessageReorder
- Time: ClockSkew, ClockJump
- Resource: OutOfMemory, CPUStarvation
- MCP: ServerCrash, SlowStart, ToolTimeout, ToolFail
- LLM: Timeout, Failure, RateLimited, AgentLoopPanic
- Sandbox/VM: BootFail, Crash, PauseFail, ResumeFail, ExecFail, ExecTimeout
- Snapshot: CreateFail, Corruption, RestoreFail, TooLarge
- Teleport: UploadFail, DownloadFail, Timeout, ArchMismatch, ImageMismatch

**Determinism:**
- RNG seeding via DST_SEED env var
- SimTime auto-advances SimClock
- No std::time::SystemTime usage
- Tokio task scheduling still non-deterministic

**INVARIANT CHECKING GAP:**
- No dedicated invariant verification module
- Tests use weak assertions: is_ok()/is_err() without extracting values
- No property-based testing (Proptest/QuickCheck)
- No temporal logic (LTL/MTL)

**STATERIGHT: NOT INTEGRATED**
- No Model trait implementations
- No state space exploration
- Framework CAN support it but doesn't require it

## Issues

### [HIGH] No invariant verification helpers - tests use weak is_ok()/is_err() assertions

**Evidence:** sandbox_io.rs:348-373 shows pattern

### [HIGH] Stateright model checking not integrated - only pseudocode exists

**Evidence:** No stateright imports found, no Model trait implementations

### [MEDIUM] Missing fault types: ConcurrentAccessConflict, DeadlockDetection, DataRace, PartialWrite

**Evidence:** Gap analysis in fault.rs

### [MEDIUM] ClockSkew/ClockJump faults declared but never injected

**Evidence:** Time faults not integrated with SimClock
