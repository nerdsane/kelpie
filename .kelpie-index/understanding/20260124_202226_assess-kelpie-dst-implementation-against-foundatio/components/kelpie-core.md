# kelpie-core

**Examined:** 2026-01-24T20:22:20.148795+00:00

## Summary

Core types and traits support DST but enforcement is discipline-based, not compile-time

## Connections

- kelpie-dst

## Details

**DST Support in Core:**
- TimeProvider trait for clock injection ✓
- RngProvider trait for random injection ✓
- Error types with is_retriable() for fault handling ✓

**Gaps:**
- No compile-time enforcement that business logic uses injected providers
- Code can still call `tokio::time::sleep()` or `rand::random()` directly
- No static analysis to detect non-deterministic escapes

**TigerStyle Compliance:**
- Constants with units ✓
- Big-endian naming ✓
- Assertions expected but not verified in DST context

## Issues

### [MEDIUM] No compile-time enforcement of deterministic I/O

**Evidence:** TimeProvider/RngProvider are traits but nothing prevents direct tokio::time::sleep() calls

### [LOW] TigerStyle assertions not systematically verified under DST

**Evidence:** assert! statements exist but DST doesn't specifically exercise assertion paths
