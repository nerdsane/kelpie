# kelpie-vm

**Examined:** 2026-01-22T21:33:19.128331+00:00

## Summary

VM abstraction layer with pluggable backends (Mock, Firecracker, Apple Vz) for lightweight VM lifecycle and snapshot/teleport operations

## Connections

- kelpie-core
- kelpie-dst
- kelpie-server

## Details

Backends: Mock (always), Firecracker (Linux, feature-gated), Apple Vz (macOS, feature-gated). Snapshot architecture fully implemented with checksum validation. Factory pattern with for_host() auto-selection.

## Issues

### [MEDIUM] Incomplete feature guard in firecracker() factory - wrong cfg attribute

**Evidence:** backend.rs

### [MEDIUM] Empty root_disk_path validation always fails - builder uses unwrap_or_default()

**Evidence:** config.rs

### [MEDIUM] Missing error context chain - no .source() in VmError

**Evidence:** error.rs

### [LOW] No validation that paths are valid/accessible - only checks length

**Evidence:** config.rs
