# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the Kelpie project.

## What is an ADR?

An ADR is a document that captures an important architectural decision made along with its context and consequences.

## ADR Index

| ADR | Title | Status | Implementation |
|-----|-------|--------|----------------|
| [000](./000-template.md) | ADR Template | - | - |
| [001](./001-virtual-actor-model.md) | Virtual Actor Model | Accepted | ✅ Complete |
| [002](./002-foundationdb-integration.md) | FoundationDB Integration | Accepted | ✅ Complete |
| [003](./003-wasm-actor-runtime.md) | WASM Actor Runtime | Proposed | ⏳ Not Started (P3) |
| [004](./004-linearizability-guarantees.md) | Linearizability Guarantees | Accepted | 🚧 Partial (needs FDB) |
| [005](./005-dst-framework.md) | DST Framework | Accepted | ✅ Complete |
| [006](./006-letta-code-compatibility.md) | Letta-Code Compatibility | Accepted | ✅ Complete |
| [007](./007-fdb-backend-implementation.md) | FDB Backend Implementation | Accepted | ✅ Complete |
| [008](./008-transaction-api.md) | Transaction API for Actor Storage | Accepted | ✅ Complete |
| [016](./016-vz-objc-bridge.md) | Apple VZ Backend via Objective-C Bridge | Accepted | 🚧 Partial |
| [017](./017-firecracker-backend-wrapper.md) | Firecracker Backend via Sandbox Wrapper | Superseded | ✅ Complete |
| [018](./018-vmconfig-kernel-initrd-fields.md) | Add Kernel/Initrd Paths to VmConfig | Accepted | ✅ Complete |
| [019](./019-vm-backends-crate.md) | Separate VM Backend Factory Crate | Superseded | ✅ Complete |
| [020](./020-consolidated-vm-crate.md) | Consolidate VM Core + Backends into kelpie-vm | Accepted | ✅ Complete |
| [021](./021-snapshot-type-system.md) | Snapshot Type System | Accepted | ✅ Complete |
| [022](./022-wal-design.md) | WAL Design | Accepted | 🚧 Partial |
| [023](./023-actor-registry-design.md) | Actor Registry Design | Accepted | ✅ Complete |
| [024](./024-actor-migration-protocol.md) | Actor Migration Protocol | Accepted | 🚧 Partial |
| [025](./025-cluster-membership-protocol.md) | Cluster Membership Protocol | Accepted | 🚧 Partial |
| [026](./026-mcp-tool-integration.md) | MCP Tool Integration | Accepted | ✅ Complete |
| [027](./027-sandbox-execution-design.md) | Sandbox Execution Design | Accepted | ✅ Complete |
| [028](./028-multi-agent-communication.md) | Multi-Agent Communication | Accepted | 🚧 Partial |
| [029](./029-federated-peer-architecture.md) | Federated Peer Architecture | Proposed | ⏳ Not Started |

## Creating a New ADR

1. Copy `000-template.md` to a new file with the next number
2. Fill in the sections
3. Update this README's index
4. Submit for review

## ADR Status

- **Proposed**: Under discussion
- **Accepted**: Approved and implemented
- **Deprecated**: No longer recommended
- **Superseded**: Replaced by another ADR

## References

- [ADR GitHub Organization](https://adr.github.io/)
- [Michael Nygard's ADR Article](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
