# kelpie-vm

**Examined:** 2026-01-24T23:28:02.632693+00:00

## Summary

COMPLETE - Mock, Apple VZ, and Firecracker backends all implemented with snapshot/restore

## Connections

- kelpie-core
- kelpie-sandbox

## Details

Backends:
- MockVm: Testing/CI - simulated lifecycle and commands
- Apple VZ: Production macOS - FFI to Virtualization.framework, vsock exec
- Firecracker: Production Linux - wraps kelpie-sandbox

All backends support:
- Full lifecycle (Created→Starting→Running→Paused→Stopped)
- Snapshot/restore with CRC32 validation
- Architecture compatibility checking
- Config validation with explicit error types

36 unit tests passing.

## Issues

### [LOW] No disk existence validation at config time - errors only at VM start

**Evidence:** RootDiskNotAccessible error defined but only Firecracker validates

### [LOW] Snapshot file cleanup is best-effort - orphaned files possible

**Evidence:** let _ = ... pattern in cleanup methods
