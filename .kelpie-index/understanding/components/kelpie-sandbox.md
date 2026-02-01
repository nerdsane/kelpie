# kelpie-sandbox

**Examined:** 2026-01-22T21:37:41.304848+00:00

## Summary

Sandbox abstraction layer for code execution - CRITICAL: No actual isolation enforcement, only configuration types

## Connections

- kelpie-core
- kelpie-server
- kelpie-tools
- kelpie-dst

## Details

12 files, 167KB. Provides SandboxIO trait and config types. Process-level only - NO namespaces, chroot, or containers. Full filesystem access, no fork bomb prevention, path traversal possible. Security depends entirely on unvalidated backend implementations.

## Issues

### [CRITICAL] No actual sandbox enforcement - full filesystem read/write access

**Evidence:** test_isolation_file_creation_outside_workdir

### [CRITICAL] No fork/resource limits - fork bombs, memory exhaustion possible

**Evidence:** No ulimit, cgroup, or namespace restrictions

### [CRITICAL] Path traversal in read_file/write_file - no path sanitization

**Evidence:** io.rs

### [CRITICAL] Command injection risk - exec() passes command as string

**Evidence:** io.rs

### [HIGH] Can see/signal host processes - no process namespace isolation

**Evidence:** ps aux enumeration possible

### [HIGH] Unrestricted environment variable injection including LD_PRELOAD

**Evidence:** exec.rs
