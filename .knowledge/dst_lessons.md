# DST Implementation Lessons

## 1. The "Object-Store Mismatch" Trap

### Observation
Developers re-implemented `SimStorage` in the server layer (using `HashMap<String, Agent>`) instead of using the existing DST storage (using `Map<Bytes, Bytes>`).

### Root Cause
The DST layer provided a low-level primitive (KV Store) without providing the "glue" code to map high-level domain objects (Agents, Messages) to that primitive.
*   **Friction**: Serializing objects to bytes for every test felt like too much boilerplate.
*   **Result**: Developers chose the path of least resistance: a parallel, non-deterministic in-memory mock.

### Lesson
**"Glue" code is infrastructure.** When building a DST core, immediately provide an `ObjectStore` or `TypedKV` wrapper. If the DST system is harder to use than a `HashMap`, developers will ignore it.

## 2. The "Async Time" Disconnect

### Observation
Tests claimed to be DST but used `tokio::time::sleep`. `SimClock` existed but was a manual counter that didn't block the async runtime.

### Root Cause
Simulated time was implemented as a *passive* data source (`now()`) rather than an *active* driver of the runtime.
*   In async Rust, "Time" is the job of the Executor (Reactor).
*   You cannot implement Deterministic Time purely in user-space code if you rely on the standard `tokio` scheduler.

### Lesson
**Control the Scheduler.** True DST in async Rust requires replacing the Runtime (e.g., using `madsim` or `turmoil`) so that `sleep(1s)` is an instruction to the simulator, not a syscall to the OS.

## 3. The "Name Pollution" Effect

### Observation
Many tests were named `*_dst.rs` simply because they used *some* components from `kelpie-dst` (like the RNG), even if they weren't deterministic.

### Root Cause
"DST" became a synonym for "Integration Test with Mocks" rather than "Deterministic Simulation".

### Lesson
**Reserve the Name.** Use strict terminology:
*   `DST`: 100% reproducible, single-threaded execution, virtual time.
*   `Chaos`: Randomized fault injection on real time/IO.
*   `Integration`: Standard wiring tests.
