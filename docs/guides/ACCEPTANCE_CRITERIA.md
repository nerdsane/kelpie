# Acceptance Criteria: No Stubs, Verification First

**Every feature must have real implementation and empirical verification.**

## No Stubs Policy

Code must be functional, not placeholder:

```rust
// FORBIDDEN - stub implementation
fn execute_tool(&self, name: &str) -> String {
    "Tool execution not yet implemented".to_string()
}

// FORBIDDEN - TODO comments as implementation
async fn snapshot(&self) -> Result<Snapshot> {
    // TODO: implement snapshotting
    Ok(Snapshot::empty())
}

// REQUIRED - real implementation or don't merge
fn execute_tool(&self, name: &str, input: &Value) -> String {
    match name {
        "shell" => {
            let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
            self.sandbox.exec("sh", &["-c", command]).await
        }
        _ => format!("Unknown tool: {}", name),
    }
}
```

## Verification-First Development

You must **empirically prove** features work before considering them done:

1. **Unit tests** - Function-level correctness
2. **Integration tests** - Component interaction
3. **Manual verification** - Actually run it and see it work
4. **DST coverage** - Behavior under faults

## Verification Checklist

Before marking any feature complete:

| Check | How to Verify |
|-------|---------------|
| Code compiles | `cargo build` |
| Tests pass | `cargo test` |
| No warnings | `cargo clippy` |
| Actually works | Run the server, hit the endpoint, see real output |
| Edge cases handled | Test with empty input, large input, malformed input |
| Errors are meaningful | Trigger errors, verify messages are actionable |

## Example: Verifying LLM Integration

Don't just write the code. Prove it works:

```bash
# 1. Start the server
ANTHROPIC_API_KEY=sk-... cargo run -p kelpie-server

# 2. Create an agent with memory
curl -X POST http://localhost:8283/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name": "test", "memory_blocks": [{"label": "persona", "value": "You are helpful"}]}'

# 3. Send a message and verify LLM response (not stub)
curl -X POST http://localhost:8283/v1/agents/{id}/messages \
  -H "Content-Type: application/json" \
  -d '{"role": "user", "content": "What is 2+2?"}'

# 4. Verify response is from real LLM, not "stub response"
# 5. Verify memory blocks appear in the prompt (check logs)
# 6. Test tool execution - ask LLM to run a command
```

## What "Done" Means

A feature is done when:

- [ ] Implementation is complete (no TODOs, no stubs)
- [ ] Unit tests exist and pass
- [ ] Integration test exists and passes
- [ ] You have personally run it and seen it work
- [ ] Error paths have been tested
- [ ] Documentation updated if needed

## Current Codebase Audit

Run this evaluation periodically:

```bash
# Find stubs and TODOs
grep -r "TODO" --include="*.rs" crates/
grep -r "unimplemented!" --include="*.rs" crates/
grep -r "stub" --include="*.rs" crates/
grep -r "not yet implemented" --include="*.rs" crates/

# Find empty/placeholder implementations
grep -r "Ok(())" --include="*.rs" crates/ | grep -v test

# Verify all tests pass
cargo test

# Check test coverage (if installed)
cargo tarpaulin --out Html
```
