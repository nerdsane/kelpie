# ADR-030: HTTP API Linearizability

## Status

Accepted

## Context

While Kelpie provides linearizability guarantees at the actor layer (ADR-004), the HTTP API layer currently lacks these guarantees. This creates several gaps:

1. **Duplicate Creation**: A client retrying a POST request after a timeout might create duplicate agents
2. **Lost Responses**: If the server responds but the client doesn't receive it, retry semantics are undefined
3. **Partial State**: Multi-step operations (agent + blocks) could leave partial state on failure
4. **No Exactly-Once**: Mutations can execute multiple times for the same logical request

These gaps violate the FoundationDB-style verification pyramid where guarantees must be maintained at every layer.

## Decision

We will implement HTTP linearizability through idempotency tokens and atomic operations.

### 1. Idempotency Token Mechanism

**Header-Based Tokens**
```
POST /v1/agents
Idempotency-Key: user-generated-uuid-12345
```

- Clients provide `Idempotency-Key` header for mutating operations
- Token must be unique per logical operation
- Server caches response by token for configurable TTL

**Token Storage**
```rust
pub const IDEMPOTENCY_TOKEN_EXPIRY_MS: u64 = 3_600_000;  // 1 hour
pub const IDEMPOTENCY_CACHE_ENTRIES_MAX: usize = 100_000;
```

### 2. Cached Response Format

```rust
pub struct CachedResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub created_at_ms: u64,
}
```

The cache is:
- **Durable**: Persisted to FDB for crash recovery
- **Bounded**: LRU eviction when `IDEMPOTENCY_CACHE_ENTRIES_MAX` reached
- **TTL-based**: Entries expire after `IDEMPOTENCY_TOKEN_EXPIRY_MS`

### 3. Request Processing Flow

```
Client Request
      │
      ▼
┌─────────────────┐
│ Extract Token   │
│ from Header     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     Yes    ┌──────────────┐
│ Token in Cache? │───────────►│ Return Cached│
└────────┬────────┘            │ Response     │
         │ No                  └──────────────┘
         ▼
┌─────────────────┐
│ Begin FDB       │
│ Transaction     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Execute Request │
│ Atomically      │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Cache Response  │
│ + Commit        │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Return Response │
└─────────────────┘
```

### 4. Atomic Operations

Multi-step operations are wrapped in FDB transactions:

```rust
pub async fn create_agent_atomic(&self, request: CreateAgentRequest) -> Result<AgentState> {
    let txn = self.storage.begin_transaction().await?;

    // All writes in single transaction
    txn.set_agent(&agent).await?;
    txn.set_blocks(&blocks).await?;
    txn.set_idempotency(token, &response).await?;

    txn.commit().await?;  // Linearization point

    Ok(agent)
}
```

### 5. TLA+ Specification

The HTTP API linearizability is specified in `docs/tla/KelpieHttpApi.tla` with these invariants:

| Invariant | Description |
|-----------|-------------|
| `IdempotencyGuarantee` | Same token → same response |
| `ExactlyOnceExecution` | Mutations execute ≤1 time per token |
| `ReadAfterWriteConsistency` | POST then GET returns entity |
| `AtomicOperation` | Multi-step appears atomic |
| `DurableOnSuccess` | Success → state survives restart |

### 6. Affected Endpoints

| Endpoint | Requires Idempotency | Reason |
|----------|---------------------|--------|
| `POST /v1/agents` | Yes | Creates agent |
| `PUT /v1/agents/:id` | No | Idempotent by ID |
| `DELETE /v1/agents/:id` | Yes | State-changing |
| `POST /v1/agents/:id/messages` | Yes | Triggers execution |
| `GET /v1/agents/:id` | No | Read-only |

## Consequences

### Positive

- **Exactly-once semantics**: Clients can safely retry without side effects
- **Verification**: TLA+ spec enables formal verification
- **DST testing**: Linearizability can be tested under fault injection

### Negative

- **Storage overhead**: ~1KB per cached response × 100K entries = ~100MB
- **Client complexity**: Clients must generate and manage tokens

### Known Limitations

The current implementation uses in-memory storage for the idempotency cache. This means:

- **DurableOnSuccess partial**: Cached responses are lost on server restart. The TLA+ invariant
  `DurableOnSuccess` is only satisfied within a single server lifetime.
- **Single-node only**: The cache is not shared across server instances. For multi-node
  deployments, implement FDB-backed persistent storage.

For production deployments requiring full durability guarantees across restarts:
1. Implement `AgentStorage` methods for idempotency (`set_idempotency`, `get_idempotency`)
2. Use FDB transactions to atomically store both the operation result and cached response

### Mitigation

- **Storage**: LRU eviction + 1-hour TTL limits cache size
- **Latency**: Cache lookup parallelized with request validation
- **Client complexity**: SDK provides automatic token generation

## Implementation

1. Phase 1: TLA+ specification (this ADR)
2. Phase 2: DST tests for invariants
3. Phase 3: Idempotency layer implementation
4. Phase 4: HTTP DST tests
5. Phase 5: Documentation updates

## References

- [ADR-004: Linearizability Guarantees](004-linearizability-guarantees.md)
- [Stripe Idempotency](https://stripe.com/docs/api/idempotent_requests)
- [RFC 7231 Section 4.2.2: Idempotent Methods](https://tools.ietf.org/html/rfc7231#section-4.2.2)
- [TLA+ Spec: KelpieHttpApi.tla](../tla/KelpieHttpApi.tla)
