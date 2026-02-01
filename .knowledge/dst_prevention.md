# DST Prevention & Best Practices

To maintain Deterministic Simulation Testing (DST) integrity, follow these rules.

## 1. The Golden Rule: No Raw Tokio
**NEVER** use `tokio::spawn`, `tokio::time::sleep`, or `std::thread::sleep` in product code (`src/`).

*   **Why?** These bypass the simulation harness. The simulator cannot control time or task scheduling for these calls, breaking determinism.
*   **Instead**: Use the `kelpie_core::Runtime` trait.
    ```rust
    // BAD
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::spawn(async move { ... });

    // GOOD
    runtime.sleep(Duration::from_millis(100)).await;
    runtime.spawn(async move { ... });
    ```

## 2. Dependency Injection
All components must accept a `Runtime` generic or trait object.

```rust
pub struct MyService<R: Runtime> {
    runtime: R,
}

impl<R: Runtime> MyService<R> {
    pub fn new(runtime: R) -> Self {
        Self { runtime }
    }
}
```

## 3. Testing with DST
When writing DST tests:
1.  **Use `SimConfig`**: Always initialize with a seed.
2.  **Use `Simulation` Harness**: Don't just run `tokio::test`.
3.  **Feature Gate**: Use `#[cfg_attr(madsim, madsim::test)]` and `#[cfg_attr(not(madsim), tokio::test)]` if you need hybrid tests.

## 4. Randomness
**NEVER** use `rand::thread_rng()` or `Uuid::new_v4()`.
*   **Instead**: Use `DeterministicRng` from the simulation environment or passed via context.

## 5. System Time
**NEVER** use `std::time::SystemTime::now()` or `chrono::Utc::now()`.
*   **Instead**: Use `runtime.now()` or the `TimeProvider` trait.

## Checklist for Reviewers
- [ ] Does the PR introduce any `tokio::` calls?
- [ ] are `std::time` or `rand` used directly?
- [ ] Do new actors/services take a `Runtime` parameter?
