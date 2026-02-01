# Plan: CI Enforcement for Determinism (Issue #23)

## Summary

Add `scripts/check-determinism.sh` and CI workflow to block non-deterministic I/O patterns like `tokio::time::sleep`, `rand::random`, etc. from being used directly in DST-sensitive code.

## Status: COMPLETE

## Options & Decisions

### Option 1: Simple grep-based CI check (Chosen)
- **Pros:** Quick to implement, catches obvious violations, easy to understand
- **Cons:** Can have false positives (comments, string literals)
- **Why chosen:** Issue explicitly requests this as immediate solution

### Option 2: Custom Clippy lints
- **Pros:** Better DX, IDE integration, fewer false positives
- **Cons:** More complex, requires separate crate, longer to implement
- **Why not chosen:** Marked as "long-term" in issue; out of scope

### Option 3: Wrapper crate approach
- **Pros:** Compile-time enforcement via imports
- **Cons:** Requires significant refactoring
- **Why not chosen:** Marked as "medium-term" in issue; out of scope

## Exception Strategy

**Allowed exceptions (legitimate uses):**
1. `kelpie-core/src/io.rs` - Production TimeProvider/RngProvider implementations
2. `kelpie-core/src/runtime.rs` - Production runtime with real time
3. `kelpie-dst/` - DST framework (seed generation, RealTime provider for comparison)
4. `kelpie-sandbox/` - Real VM interactions need real time
5. `kelpie-vm/` - VM backend implementations
6. `kelpie-tools/` - CLI tools run in production
7. `kelpie-cli/` - CLI tools run in production
8. `kelpie-cluster/` - Cluster heartbeats/gossip
9. Test files (`*_test.rs`, `tests/*.rs`, `#[cfg(test)]` blocks)

## Phases

### Phase 1: Create check-determinism.sh script [COMPLETE]
- Created script with forbidden patterns list
- Added exception handling for legitimate uses
- Added `--warn-only` and `--strict` modes
- Added `#[cfg(test)]` block detection
- Tested locally against codebase

### Phase 2: Add CI workflow job [COMPLETE]
- Added `determinism-check` job to `.github/workflows/ci.yml`
- Using `--warn-only` mode initially (to allow PR to merge)
- Can switch to `--strict` once existing violations are fixed

### Phase 3: Update CLAUDE.md documentation [COMPLETE]
- Added "I/O Abstraction Requirements" section to DST documentation
- Documented forbidden patterns and alternatives
- Documented exception locations
- Documented how to run the check locally

### Phase 4: Create PR [COMPLETE]
- Committed changes
- Pushed to branch
- Created PR

## Quick Decision Log

| Time | Decision | Rationale | Trade-off |
|------|----------|-----------|-----------|
| T+0 | Use grep-based approach | Issue explicitly requests CI script first | May have false positives |
| T+0 | Allow kelpie-core/io.rs exception | Production implementations must use real time | Need careful review |
| T+0 | Allow all test files | Tests may need real timing for benchmarks | Could hide violations |
| T+1 | Add `--warn-only` mode | Allow incremental adoption | Not blocking initially |
| T+1 | Add `#[cfg(test)]` detection | Filter out test code in src/ files | Heuristic, not perfect |
| T+1 | Add kelpie-cluster/ exception | Cluster uses real time for heartbeats | Legitimate use case |

## What to Try

### Works Now
1. Run `./scripts/check-determinism.sh` - Shows all violations
2. Run `./scripts/check-determinism.sh --warn-only` - Reports but doesn't fail
3. Run `./scripts/check-determinism.sh --help` - Shows usage

### Known Violations (26 total)
These are existing violations in the codebase that should be addressed:
- `kelpie-server/http.rs` - Network delay injection (2)
- `kelpie-registry/` - Node discovery (5)
- `kelpie-runtime/` - Activation tracking (5)
- `kelpie-server/state.rs` - Request tracking (8)
- `kelpie-server/tools/` - Various tools (4)
- `kelpie-server/service/` - Teleport service (1)
- `kelpie-server/actor/` - Registry actor (2)

### Known Limitations
- Grep-based detection can have false positives in comments/strings
- `#[cfg(test)]` detection is heuristic (looks for marker in prior 100 lines)
- Exception list requires maintenance as codebase evolves
- Currently using `--warn-only` in CI; switch to `--strict` once violations fixed

## Files Changed

1. `scripts/check-determinism.sh` - New script (enforcement check)
2. `.github/workflows/ci.yml` - Added determinism-check job
3. `CLAUDE.md` - Added I/O Abstraction Requirements documentation
