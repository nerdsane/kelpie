# Semantic Index

**Status: Infrastructure Ready, LLM Generation Deferred**

This directory contains LLM-generated semantic understanding of the codebase. Unlike the structural indexes (which are deterministic), semantic indexes are variable and git-ignored.

## Directory Structure

```
semantic/
├── summaries/          # Hierarchical code summaries
│   ├── {crate}/
│   │   ├── crate_summary.json
│   │   └── {module}/
│   │       └── module_summary.json
│   └── README.md
└── validation_issues.json  # Cross-validation findings
```

## Phase 3.1: Hierarchical Summaries

**Goal:** Generate human-readable summaries of crates, modules, and functions using LLMs.

### Approach (HCGS - Hierarchical Code Graph Summarization)

1. **Bottom-up summarization:**
   - Function level → File level → Module level → Crate level
   - Each summary uses context from child summaries

2. **Input sources:**
   - Structural indexes (symbols, dependencies, tests, modules)
   - Function signatures and visibility
   - Test names and topics
   - Module relationships

3. **Output format:**
   ```json
   {
     "crate": "kelpie-storage",
     "summary": "Per-actor key-value storage abstraction with multiple backend support",
     "key_concepts": ["ActorKV trait", "storage backends", "transactions"],
     "modules": [
       {
         "path": "kelpie_storage::fdb",
         "summary": "FoundationDB storage backend with ACID transactions",
         "files": [
           {
             "path": "src/fdb.rs",
             "summary": "FdbStorage struct implementing ActorKV trait",
             "key_functions": [
               {
                 "name": "get",
                 "summary": "Retrieves value by key from FoundationDB"
               },
               {
                 "name": "put",
                 "summary": "Stores key-value pair with transaction support"
               }
             ]
           }
         ]
       }
     ]
   }
   ```

### Trust Level: LOW

- LLM summaries are **guidance, not ground truth**
- Use for navigation and understanding, not verification
- Always cross-reference with structural indexes for facts
- Summaries may become stale as code changes

## Phase 3.2: Constraint Extraction

**Goal:** Extract structured constraints from `.vision/CONSTRAINTS.md` and verify them.

### Output Format

```json
{
  "id": "simulation-first",
  "category": "P0",
  "rule": "Every feature must be DST tested before complete",
  "verification": {
    "type": "test",
    "command": "cargo test -p kelpie-dst",
    "pass_criteria": "all tests pass"
  },
  "enforcement": "hard",
  "source_line": 17,
  "last_verified": "2026-01-20T10:30:00Z",
  "status": "passing"
}
```

Stored in: `../constraints/extracted.json`

## Phase 3.3: Cross-Validation

**Goal:** Compare structural vs semantic indexes to find inconsistencies.

### Validation Checks

1. **Unused code claims:**
   - Summary: "function X is unused"
   - Verify: Check symbol references in dependency graph
   - Flag if contradictory

2. **Module purpose claims:**
   - Summary: "module Y handles Z"
   - Verify: Check if Z appears in module's symbols
   - Flag if Z not found

3. **Test coverage claims:**
   - Summary: "feature W is well-tested"
   - Verify: Count tests with topic "W"
   - Flag if < 3 tests

### Output Format

```json
{
  "issues": [
    {
      "type": "unused_claim_contradiction",
      "severity": "warning",
      "summary_claim": "Function bar() is unused",
      "structural_evidence": "Found 3 references to bar() in call graph",
      "recommendation": "Review summary or verify references are test-only"
    }
  ]
}
```

Stored in: `validation_issues.json`

## Why Deferred?

Phase 3 (Semantic Indexing) requires:

1. **LLM API integration** - Anthropic API client, rate limiting, cost management
2. **Multi-agent orchestration** - Coordinator agent dispatching multiple summarization agents
3. **Prompt engineering** - Careful prompt design for accurate summaries
4. **Cost considerations** - Summarizing 186 files × multiple levels = significant API costs
5. **Freshness management** - Summaries become stale, need refresh strategy

**Decision:** Build Phase 4 (MCP Server) first using structural indexes only. Phase 3 can be added later when:
- MCP server is working and validated
- LLM summarization strategy is refined
- Cost/benefit is clear for this specific codebase

## Next Steps

To implement Phase 3 when ready:

1. Add LLM client to `kelpie-indexer`:
   ```rust
   // Add to Cargo.toml
   reqwest = { version = "0.12", features = ["json"] }

   // Add Commands::Semantic
   async fn generate_summaries(workspace_root: &Path, api_key: &str) -> Result<()>
   ```

2. Implement hierarchical summarization:
   - Read structural indexes
   - For each crate/module/file, generate summary
   - Store in semantic/summaries/

3. Run: `cargo run -p kelpie-indexer -- semantic --api-key $ANTHROPIC_API_KEY`

4. Cross-validate against structural indexes

## Current Status

- ✅ Directory structure created
- ✅ Schema designed
- ✅ Git-ignore configured
- ❌ LLM integration (deferred to later)
- ❌ Summary generation (deferred to later)
- ❌ Cross-validation (deferred to later)

Structural indexes (Phase 2) are complete and sufficient for Phase 4 (MCP Server).
