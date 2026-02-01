# Kelpie Project Constitution

## Project Identity

- **Name**: Kelpie
- **Description**: Distributed virtual actor system with linearizability guarantees for AI agent orchestration
- **Version**: 0.1.0
- **Created**: 2025-01-27

## Core Principles

### 1. TigerStyle Engineering (Safety > Performance > DX)
- Explicit constants with units (`TIMEOUT_MS_MAX` not `TIMEOUT`)
- Big-endian naming (`actor_id_length_bytes_max`)
- 2+ assertions per function (preconditions and postconditions)
- No silent truncation - explicit conversions with checks
- No `unwrap()` in production code

### 2. DST-First Development
- All features tested through DST harness with fault injection
- All randomness flows from `DST_SEED`
- All I/O through injected providers (`TimeProvider`, `RngProvider`)
- Never use `tokio::time::sleep`, `SystemTime::now()`, or `rand::random()` directly

### 3. Verification Before Completion
- Run `cargo test`, `cargo clippy`, `cargo fmt` before any commit
- DST tests must pass with reproducible seeds
- No placeholders, TODOs, or FIXMEs in production code

## Technical Stack

- **Language**: Rust 2021 Edition (MSRV 1.75)
- **Async Runtime**: Tokio
- **Testing**: madsim + Stateright + proptest
- **Storage**: FoundationDB (feature-gated)
- **Source Location**: `crates/`

## Ralph Loop Settings

### Autonomy
- **YOLO Mode**: ENABLED
- **Git Autonomy**: ENABLED

### Work Item Sources (Priority Order)
1. `specs/` folder - Feature specifications
2. `IMPLEMENTATION_PLAN.md` - Unchecked tasks
3. `.progress/` folder - Active plans

### Validation Commands
```bash
cargo build --all-features
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

## Context Detection

### Ralph Loop Mode
When operating within the automated Ralph loop:
- Be fully autonomous - don't ask for permission
- Pick one task, complete it fully, verify all acceptance criteria
- Only output `<promise>DONE</promise>` when 100% complete
- Commit and push after each completed task

### Interactive Mode
When chatting directly with a human:
- Ask clarifying questions when needed
- Explain reasoning and trade-offs
- Follow the planning workflow in `.progress/`

## Mandatory Constraints

Read and follow `.vision/CONSTRAINTS.md` for non-negotiable development rules.

Key constraints:
- All code must be DST-testable
- No direct system calls for time/randomness
- Explicit error handling everywhere
- Property-based tests for serialization
