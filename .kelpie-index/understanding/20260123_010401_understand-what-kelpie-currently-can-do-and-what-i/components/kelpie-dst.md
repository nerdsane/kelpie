# kelpie-dst

**Examined:** 2026-01-23T01:01:50.531219+00:00

## Summary

Deterministic Simulation Testing framework with 70 tests - clock, RNG, storage, network, faults, sandbox simulation

## Connections

- kelpie-core
- kelpie-storage
- kelpie-runtime
- kelpie-vm

## Details

**WORKING (70 tests pass):**
- SimClock - deterministic time control, async sleep primitives
- DeterministicRng - seeded RNG with fork support, reproducible
- SimStorage - in-memory KV with fault injection, transactions
- SimNetwork - message queuing, latency, partitions, reordering
- FaultInjector - 40+ fault types across storage/crash/network/time/resource/MCP/LLM/sandbox/snapshot/teleport
- SimSandbox - lifecycle, exec, snapshot/restore simulation
- SimLlmClient - deterministic LLM mocking with canned responses
- SimTeleportStorage - teleport package simulation
- Simulation harness - environment builder, determinism verification

**Fault types supported:**
- Storage: ReadFail, WriteFail, Corruption, Latency, DiskFull
- Crash: BeforeWrite, AfterWrite, DuringTransaction
- Network: Partition, Delay, PacketLoss, MessageReorder
- Time: ClockSkew, ClockJump
- Sandbox: BootFail, ExecFail, SnapshotFail, etc.

**DST Quality:**
- True determinism via DST_SEED reproducibility
- Explicit time control (no wall clock dependency)
- Comprehensive fault coverage

## Issues

### [LOW] SimTeleportStorage ignores DeterministicRng parameter (_rng unused)

**Evidence:** teleport.rs - HashMap iteration may be non-deterministic

### [LOW] Max steps/time limits defined but not enforced in simulation

**Evidence:** simulation.rs - limits in config but no runtime checks
