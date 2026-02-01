# kelpie-vm

**Examined:** 2026-01-24T15:23:23.983102+00:00

## Summary

VM abstraction layer with Mock, Firecracker, Apple VZ backends; snapshot/restore with checksum verification

## Connections

- kelpie-sandbox
- kelpie-dst
- kelpie-core

## Details

**VM Backends:**
- Mock: Testing with configurable failures
- Firecracker: Linux microVMs (feature-gated)
- Apple VZ: macOS virtualization (feature-gated)

**State Machine:**
Created → Starting → Running ⇄ Paused → Stopped → (restart) Created
Any → Crashed (terminal)

**Snapshot Guarantees:**
- CRC32 checksum verification
- Architecture validation (full snapshots require matching arch)
- 1 GiB max size
- Restore only from Created/Stopped states

**Invariants:**
- vcpu_count >= 1
- VM must be Running for exec()
- Checksum match for valid snapshot

**Error Recovery:**
- Retriable: BootTimeout, ExecTimeout, Crashed
- Requires recreate: Crashed, SnapshotCorrupted

## Issues

### [LOW] Snapshot checksum is CRC32 - weak for integrity, consider SHA256

**Evidence:** snapshot.rs:85-87
