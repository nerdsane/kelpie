# kelpie-core

**Examined:** 2026-01-24T23:24:45.098331+00:00

## Summary

Core types and traits are COMPLETE - ActorId, Actor trait, Error enum, Runtime abstraction, I/O providers, KV interfaces all implemented

## Connections

- kelpie-runtime
- kelpie-storage
- kelpie-dst

## Details

Fully implemented:
- ActorId with validation
- Actor trait with lifecycle hooks (on_activate, on_deactivate, invoke)
- ActorContext for runtime access
- Error enum (40+ variants)
- Runtime abstraction (tokio + madsim)
- TimeProvider + RngProvider traits with implementations
- ContextKV trait with BufferingContextKV, ArcContextKV
- TeleportPackage and VmSnapshotBlob for snapshots
- KelpieConfig validation
- OpenTelemetry integration

Incomplete:
- TeleportStorage trait has no implementations
- Cross-actor invocation routing marked "future phase"

## Issues

### [LOW] TeleportStorage trait defined but no backends implemented

**Evidence:** teleport.rs defines trait but no S3/filesystem/sim implementations
