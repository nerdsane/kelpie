# ADR-027: Sandbox Execution Design

## Status

Accepted (supersedes ADR-007a)

## Date

2026-01-24

## Implementation Status

| Component | Status | Location |
|-----------|--------|----------|
| ProcessSandbox | ✅ Complete | `kelpie-server` |
| AppleVzSandbox | ✅ Complete | `kelpie-vm` (macOS) |
| FirecrackerSandbox | ✅ Complete | `kelpie-vm` (Linux) |
| MockVmSandbox | ✅ Complete | `kelpie-vm` (testing) |

**Note**: This ADR supersedes the deprecated ADR-007a (libkrun integration).

## Context

Kelpie agents execute code and tools that require isolation for:

1. **Security**: Untrusted code must not affect host system
2. **Resource Control**: Limit CPU, memory, disk usage
3. **Reproducibility**: Consistent execution environment
4. **Teleportation**: Support snapshot/restore for VM migration

The isolation mechanism must balance security with performance, supporting both lightweight sandboxing for simple tasks and full VM isolation for sensitive workloads.

## Decision

Implement a multi-level isolation architecture with pluggable sandbox backends.

### Isolation Levels

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Isolation Level Hierarchy                         │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ Level 3: Full VM Isolation (Firecracker/Apple Vz)              │ │
│  │ ────────────────────────────────────────────────────────────── │ │
│  │ - Separate kernel                                               │ │
│  │ - Hardware-enforced isolation                                   │ │
│  │ - Snapshot/restore support                                      │ │
│  │ - Use for: Untrusted code, teleportation                       │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ Level 2: Process Sandbox                                        │ │
│  │ ────────────────────────────────────────────────────────────── │ │
│  │ - Process isolation (fork/exec)                                 │ │
│  │ - Resource limits (ulimit, cgroups)                            │ │
│  │ - No snapshot support                                           │ │
│  │ - Use for: Trusted tools, quick commands                       │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ Level 1: In-Process (WASM)                                      │ │
│  │ ────────────────────────────────────────────────────────────── │ │
│  │ - WASM runtime sandbox                                          │ │
│  │ - Memory isolation                                              │ │
│  │ - Fast startup                                                  │ │
│  │ - Use for: Deterministic computations                          │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Sandbox Trait

```rust
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Execute a command in the sandbox
    async fn exec(
        &self,
        command: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<ExecResult>;

    /// Read a file from the sandbox
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;

    /// Write a file to the sandbox
    async fn write_file(&self, path: &str, content: &[u8]) -> Result<()>;

    /// Create a snapshot (if supported)
    async fn snapshot(&self) -> Result<Snapshot>;

    /// Restore from a snapshot (if supported)
    async fn restore(&self, snapshot: Snapshot) -> Result<()>;
}
```

### Backend Selection

| Backend | Platform | Snapshot | Performance | Security |
|---------|----------|----------|-------------|----------|
| MockVm | All | Simulated | Fast | None (testing) |
| ProcessSandbox | All | No | Fast | Medium |
| AppleVzSandbox | macOS 14+ | Yes | Medium | High |
| FirecrackerSandbox | Linux | Yes | Medium | High |

### Resource Limits

```rust
struct ResourceLimits {
    /// Maximum CPU time in milliseconds
    cpu_time_ms: u64,
    /// Maximum memory in bytes
    memory_bytes: u64,
    /// Maximum disk space in bytes
    disk_bytes: u64,
    /// Maximum execution time in milliseconds
    timeout_ms: u64,
    /// Maximum number of processes
    max_processes: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            cpu_time_ms: 30_000,      // 30 seconds
            memory_bytes: 512 * 1024 * 1024,  // 512 MB
            disk_bytes: 1024 * 1024 * 1024,   // 1 GB
            timeout_ms: 60_000,       // 1 minute
            max_processes: 10,
        }
    }
}
```

### Pool Management

VM sandboxes are expensive to create. Use pooling for efficiency:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Sandbox Pool Architecture                         │
│                                                                      │
│  ┌───────────────────┐                                              │
│  │   Pool Manager    │                                              │
│  └─────────┬─────────┘                                              │
│            │                                                         │
│    ┌───────┼───────────────────────────────┐                        │
│    │       │                               │                        │
│    ▼       ▼                               ▼                        │
│  ┌─────┐ ┌─────┐                        ┌─────┐                     │
│  │ VM1 │ │ VM2 │  ...                   │ VMn │                     │
│  │Idle │ │Busy │                        │Idle │                     │
│  └─────┘ └─────┘                        └─────┘                     │
│                                                                      │
│  Pre-warming: Keep MIN_POOL_SIZE VMs ready                          │
│  Scaling: Create up to MAX_POOL_SIZE on demand                      │
│  Cleanup: Destroy VMs after MAX_IDLE_TIME                           │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Snapshot/Restore for Teleportation

VM sandboxes support snapshot/restore for actor teleportation:

1. **Snapshot**: Capture VM memory state
2. **Serialize**: Convert to portable format
3. **Transfer**: Move to destination node
4. **Restore**: Resume VM from snapshot

See [ADR-015: VMInstance Teleport Backends](./015-vminstance-teleport-backends.md) for details.

## Consequences

### Positive

- **Defense in Depth**: Multiple isolation levels provide layered security
- **Flexibility**: Choose appropriate isolation for use case
- **Teleportation Support**: VM backends enable live migration
- **Platform Coverage**: Works on macOS and Linux

### Negative

- **VM Overhead**: Full VM isolation has startup latency (~100-500ms)
- **Resource Usage**: VMs consume more memory than processes
- **Complexity**: Multiple backends to maintain

### Neutral

- Trade-off between security and performance is configurable
- Pool management requires tuning for workload patterns

## Alternatives Considered

### Container Isolation (Docker)

- Use Docker containers for isolation
- Well-understood technology

**Rejected because**: Slower startup than VMs for our use case. Weaker isolation than VMs. No snapshot support without additional tooling.

### gVisor/Kata Containers

- Container-like UX with VM-like isolation
- Used by GKE sandboxed pods

**Rejected because**: Additional operational complexity. Our VM backends (Apple Vz, Firecracker) provide similar benefits with better snapshot support.

### No Isolation (Trust Agent Code)

- Run agent code directly in host process
- Fastest execution

**Rejected because**: Unacceptable security risk. Agent code is untrusted.

### WASM-Only Isolation

- Use WASM sandbox for everything
- Portable, fast, deterministic

**Rejected because**: WASM has limited system access. Cannot run arbitrary binaries. Supplement with VM backends.

## References

- [ADR-015: VMInstance Teleport Backends](./015-vminstance-teleport-backends.md) - VM backend architecture
- [ADR-019: VM Backends Crate](./019-vm-backends-crate.md) - Implementation details
- [Firecracker](https://firecracker-microvm.github.io/) - Linux VM backend
- [Apple Virtualization Framework](https://developer.apple.com/documentation/virtualization) - macOS VM backend
