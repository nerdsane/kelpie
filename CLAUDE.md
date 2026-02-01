# CLAUDE.md - Kelpie Development Guide

## Project Overview

Kelpie is a distributed virtual actor system with linearizability guarantees, designed for AI agent orchestration. Built with DST-first (Deterministic Simulation Testing) and TigerStyle engineering principles.

## Quick Commands

```bash
cargo build                              # Build
cargo test                               # Test all
cargo test -p kelpie-dst                 # DST tests
DST_SEED=12345 cargo test -p kelpie-dst  # Reproducible DST
cargo clippy --all-targets               # Lint
cargo fmt                                # Format
cargo bench -p kelpie-runtime            # Benchmarks
```

## Architecture

```
kelpie/
├── crates/
│   ├── kelpie-core/      # Core types, errors, constants
│   ├── kelpie-runtime/   # Actor runtime and dispatcher
│   ├── kelpie-registry/  # Actor placement and discovery
│   ├── kelpie-storage/   # Per-actor KV storage
│   ├── kelpie-dst/       # Deterministic Simulation Testing
│   ├── kelpie-agent/     # AI agent abstractions
│   └── kelpie-server/    # Standalone server binary
├── docs/guides/          # Detailed documentation (see below)
└── tests/                # Integration tests
```

**Detailed docs:** [Base Images](docs/guides/BASE_IMAGES.md) | [VM Backends](docs/guides/VM_BACKENDS.md) | [Code Style](docs/guides/CODE_STYLE.md)

---

## TigerStyle Engineering Principles

Kelpie follows TigerBeetle's TigerStyle: **Safety > Performance > DX**

### 1. Explicit Constants with Units
```rust
pub const ACTOR_INVOCATION_TIMEOUT_MS_MAX: u64 = 30_000;  // Good
pub const TIMEOUT: u64 = 30000;                            // Bad
```

### 2. Big-Endian Naming
```rust
actor_id_length_bytes_max    // Good - big to small
max_actor_id_length          // Bad - small to big
```

### 3. Assertions (2+ per Function)
```rust
pub fn set_timeout(&mut self, timeout_ms: u64) {
    assert!(timeout_ms > 0, "timeout must be positive");
    assert!(timeout_ms <= TIMEOUT_MS_MAX, "timeout exceeds maximum");
    self.timeout_ms = timeout_ms;
    assert!(self.timeout_ms > 0);  // Postcondition
}
```

### 4. Prefer u64 Over usize
Fixed-width integers for portability.

### 5. No Silent Truncation
Explicit conversion with assertions.

### 6. Explicit Error Handling
No `unwrap()` in production code.

### 7. Debug Assertions for Expensive Checks
Use `debug_assert!` for checks too expensive for release.

---

## Verification Pyramid

```bash
# Level 1: Unit Tests (~5s)
cargo test -p kelpie-core

# Level 2: DST (~30s)
cargo test -p kelpie-dst --release

# Level 3: Integration (~60s)
cargo test -p kelpie-server --test '*'

# Full verification (before commit)
cargo test && cargo clippy -- -D warnings && cargo fmt --check
```

---

## DST (Deterministic Simulation Testing)

**Core Principles:**
1. All randomness from single seed (`DST_SEED`)
2. Simulated time (`SimClock`)
3. Explicit fault injection (16+ fault types)
4. Deterministic network and task scheduling (madsim)

**MANDATORY: I/O Abstraction**

```rust
// ❌ FORBIDDEN - Breaks determinism
tokio::time::sleep(Duration::from_secs(1)).await;
std::time::SystemTime::now();
rand::random::<u64>();

// ✅ CORRECT - Use injected providers
time_provider.sleep_ms(1000).await;
time_provider.now_ms();
rng_provider.next_u64();
```

| Forbidden | Use Instead |
|-----------|-------------|
| `tokio::time::sleep()` | `time_provider.sleep_ms()` |
| `SystemTime::now()` | `time_provider.now_ms()` |
| `rand::random()` | `rng_provider.next_u64()` |

**Full DST guide:** [docs/guides/DST.md](docs/guides/DST.md)

---

## RLM Tool Selection Policy (CRITICAL)

**The point of RLM is to keep context on the server, not in your context window.**

### NEVER Do This
```
Read(file_path="file1.rs")
Read(file_path="file2.rs")
Read(file_path="file3.rs")
# Fills your context with 3000+ tokens
```

### ALWAYS Do This
```python
# Load files server-side
repl_load(pattern="crates/**/*.rs", var_name="all_code")

# Analyze with sub_llm() inside REPL
repl_exec(code="""
results = {}
for path, content in all_code.items():
    results[path] = sub_llm(content, "What does this do?")
result = results
""")
```

### Task-to-Tool Routing

| Task | Don't Use | Use Instead |
|------|-----------|-------------|
| Read multiple files | `Read` tool | `repl_load` + `repl_sub_llm` |
| Find patterns | `Grep` + `Read` | `repl_load` + `repl_exec` |
| Build codebase map | `Read` every file | `exam_start` + examination workflow |

### RLM Pitfalls

| Pitfall | Problem | Fix |
|---------|---------|-----|
| Shallow glob | `src/*.rs` misses subdirs | Use `**/*.rs` |
| Incomplete scope | Loading only some crates | Load all: `crates/**/*.rs` |
| No file count check | Unknown coverage | `len(files)` before analysis |

```python
# Always verify coverage first
repl_load(pattern="crates/**/*.rs", var_name="all_code")
repl_exec(code="result = len(all_code)")  # Expect ~200+ files
```

**Full EVI guide:** [docs/guides/EVI.md](docs/guides/EVI.md)

---

## Vision-Aligned Planning (MANDATORY)

**Before ANY non-trivial task (3+ steps, multi-file, research):**

1. **Read `.vision/CONSTRAINTS.md`** - Non-negotiable rules
2. **Read existing `.progress/` plans** - Current state
3. **Create plan file** - `.progress/NNN_YYYYMMDD_HHMMSS_task-name.md`

**During execution:**
- Update plan after each phase
- Log decisions in Quick Decision Log
- Update "What to Try" section

**Before completion:**
```bash
cargo test && cargo clippy && cargo fmt --check
```

### Plan File Required Sections

1. **Options & Decisions** - 2-3 options per decision, pros/cons, rationale
2. **Quick Decision Log** - Time | Decision | Rationale | Trade-off
3. **What to Try** - Works Now / Doesn't Work Yet / Known Limitations

---

## Commit Policy

**Never commit broken code.**

```bash
# Required before EVERY commit
cargo test           # All tests pass
cargo clippy         # No warnings
cargo fmt --check    # Formatted
```

**Conventional commits:** `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`

**Full acceptance criteria:** [docs/guides/ACCEPTANCE_CRITERIA.md](docs/guides/ACCEPTANCE_CRITERIA.md)

---

## Contributing

1. Create branch from `main`
2. Follow TigerStyle and DST guidelines
3. Run `cargo test && cargo clippy && cargo fmt`
4. Verify feature works end-to-end
5. Create PR with clear description

**GitHub @claude integration:** [docs/guides/GITHUB_INTEGRATION.md](docs/guides/GITHUB_INTEGRATION.md)

## References

- [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)
- [FoundationDB Testing](https://www.foundationdb.org/files/fdb-paper.pdf)
- [Stateright Model Checker](https://github.com/stateright/stateright)
