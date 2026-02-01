# kelpie-sandbox

**Examined:** 2026-01-24T15:23:23.663401+00:00

## Summary

Sandbox execution environment with Mock, Firecracker backends; GenericSandbox<IO> pattern for DST

## Connections

- kelpie-vm
- kelpie-dst
- kelpie-core

## Details

**Sandbox Types:**
- MockSandbox: In-memory simulation (testing)
- FirecrackerSandbox: MicroVM via KVM
- GenericSandbox<IO>: Pluggable I/O for DST

**State Machine:**
Stopped → Creating → Running ⇄ Paused → Stopped
Any → Destroying

**Invariants:**
- State pre-conditions: can_start(), can_pause(), can_exec() guards
- Exec only in Running state
- Snapshot only Running/Paused
- Exit status: signal=Some(n) ⇒ code=128+n
- Architecture compatibility on restore

## Issues

### [HIGH] State TOCTOU in Firecracker: state read then released then written, allowing interleaving

**Evidence:** firecracker.rs:482-489

### [MEDIUM] Async I/O without atomicity - VM could be partially configured if task cancels

**Evidence:** firecracker.rs:540-582

### [LOW] Process cleanup race - process might be dead when kill() called

**Evidence:** firecracker.rs:608-612
