# kelpie-core

**Examined:** 2026-01-22T23:58:17.793043+00:00

## Summary

Core types for virtual actor system - ActorId, ActorRef, ActorContext with location transparency claims

## Connections

- kelpie-runtime
- kelpie-storage

## Details

- ActorRef provides location transparency via `qualified_name()` abstraction
- Single-threaded execution guarantee documented in Actor trait
- TransactionalKV for atomic state+KV operations
- Linearizability claimed in module docs but implementation is in runtime
- No AI-specific types at core level - generic actor infrastructure

## Issues

### [LOW] AI agent orchestration claimed but no agent-specific abstractions in core

**Evidence:** lib.rs mentions 'designed as infrastructure for AI agent orchestration' but no Agent types exported
