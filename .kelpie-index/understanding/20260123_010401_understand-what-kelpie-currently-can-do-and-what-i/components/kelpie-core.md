# kelpie-core

**Examined:** 2026-01-23T01:01:50.313407+00:00

## Summary

Core types and abstractions - Actor, ActorId, ActorRef, Config, Error, I/O context, Teleport types

## Connections

- kelpie-runtime
- kelpie-storage
- kelpie-dst

## Details

**WORKING (27 tests pass):**
- ActorId with namespace validation (length, character checks)
- ActorRef for location-transparent actor references
- BufferingContextKV with read-your-writes semantics
- Error types with retriable classification
- IoContext abstraction for time/RNG providers
- TeleportPackage and VmSnapshotBlob encode/decode with checksums
- Runtime abstraction (tokio + madsim support)
- Telemetry configuration

**Traits defined (implemented elsewhere):**
- Actor trait (invoke, on_activate, on_deactivate)
- ContextKV trait (get, set, delete, exists, list_keys)
- TimeProvider, RngProvider traits
- TeleportStorage trait
