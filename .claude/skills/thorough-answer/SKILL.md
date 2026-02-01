---
name: thorough-answer
description: Answer questions thoroughly by examining all relevant components before responding. Use when user asks complex questions about how things work, where code is, or needs authoritative answers. Enforces completeness before answering.
---

# Thorough Answer Workflow

This skill ensures you examine all relevant components before answering a question. It prevents superficial answers by enforcing a completeness gate.

## When to Use

**Trigger conditions:**
- User asks "how does X work?"
- User asks "where is the code for X?"
- User needs to understand a specific subsystem
- User asks a question that could have a shallow or deep answer
- Any question where incomplete examination could lead to wrong answers

**Do NOT use for:**
- Simple factual questions that don't require code examination
- Questions where the answer is in a single known file
- Quick lookups that don't need verification

## Prerequisites

This skill requires the Kelpie MCP server with examination tools:
- `exam_start` - Start scoped examination
- `exam_record` - Record findings
- `exam_status` - Check progress
- `exam_complete` - Verify completeness (REQUIRED before answering)
- `issue_list` - Query related issues

## Workflow

### Step 1: Identify Relevant Components

Before starting, determine which components are relevant to the question.

**For "how does storage work?":**
- `kelpie-storage` (definitely)
- `kelpie-core` (types and errors)
- `kelpie-dst` (testing patterns)

**For "how does the actor lifecycle work?":**
- `kelpie-runtime` (main implementation)
- `kelpie-core` (actor types)
- `kelpie-cluster` (distributed activation)

Use `index_modules()` to see available components if unsure.

### Step 2: Start Scoped Examination

```
exam_start(
  task="<Rephrase user's question as a task>",
  scope=["relevant", "components"]
)
```

Example:
```
exam_start(
  task="Understand how actor storage works",
  scope=["kelpie-storage", "kelpie-core"]
)
```

### Step 3: Examine Each Component

For EACH component in your scope, use **indexes for structure** and **RLM for understanding**:

#### 3a. Get Structure from Indexes (fast)
```
index_modules(crate="kelpie-storage")
index_deps(crate="kelpie-storage")
index_symbols(kind="trait", crate="kelpie-storage")
```

#### 3b. Get Understanding from RLM (NOT native Read!)

**KEY CONCEPT: SYMBOLIC RECURSION**

RLM enables **symbolic recursion** - the sub-LLM call lives INSIDE the REPL code.
This is fundamentally different from Claude Code / Codex where sub-agents are separate tool calls.

Available functions inside `repl_exec()`:
- `sub_llm(content, query)` - Sequential sub-LLM call
- `parallel_sub_llm(items, query_or_fn)` - Parallel sub-LLM calls (up to 10 concurrent)

```python
repl_load(pattern="crates/kelpie-storage/**/*.rs", var_name="storage_code")
```

**RLM means PROGRAMMATIC analysis with GUARANTEED execution** - use Python logic with `sub_llm()`:

```python
# GOOD: Multi-stage programmatic analysis with issue extraction
repl_exec(code="""
# Stage 1: Categorize files
categories = {'traits': [], 'impl': [], 'tests': []}
for path in storage_code.keys():
    if 'test' in path.lower():
        categories['tests'].append(path)
    elif 'trait' in storage_code[path] or path.endswith('mod.rs'):
        categories['traits'].append(path)
    else:
        categories['impl'].append(path)

# Stage 2: Analyze AND extract issues (always ask about issues!)
analysis = {}

for path in categories['traits']:
    analysis[path] = sub_llm(storage_code[path], '''
        1. What traits defined? Contract/interface?
        2. ISSUES: Missing methods? Unclear contracts? TODO/FIXME?
        Format issues as: [SEVERITY] description
    ''')

for path in categories['impl']:
    analysis[path] = sub_llm(storage_code[path], '''
        1. How does this implement storage? Key patterns?
        2. ISSUES: Error handling gaps? unwrap() calls? Performance concerns?
        Format issues as: [SEVERITY] description
    ''')

# Stage 3: Extract issues into structured format
issues = sub_llm(str(analysis), '''
    Extract ALL issues as JSON:
    [{"severity": "high|medium|low", "description": "...", "evidence": "file:line"}]
''')

# Stage 4: Synthesize for the user's question
summary = sub_llm(str(analysis),
    "Synthesize: How does storage work? Key components? Architecture?")

result = {
    'categories': categories,
    'analysis': analysis,
    'issues': issues,  # Surface issues found!
    'summary': summary
}
""")
```

**BAD - Don't do this (wastes programmatic power):**
```python
combined = '\\n'.join(storage_code.values())
analysis = sub_llm(combined, "How does storage work?")
```

**FASTER: Use parallel_sub_llm for bulk analysis:**
```python
repl_exec(code="""
items = [{'path': p, 'content': c} for p, c in storage_code.items()]

# Same query for all files - runs up to 10 concurrently!
results = parallel_sub_llm(items, '''
    1. What does this file do?
    2. ISSUES: Any problems, TODOs, or concerns?
''')

# Or custom query per file:
results = parallel_sub_llm(
    items,
    lambda item: (item['content'], f"Analyze {item['path']}: purpose and issues")
)

result = results
""")
```

**For simple single queries**, `repl_sub_llm` tool is fine:
```python
repl_sub_llm(var_name="storage_code", query="<your question>")
```

**CRITICAL:** Do NOT use native `Read` tool for bulk analysis.

#### 3c. Synthesize Findings
1. **Structure** - From indexes: modules, dependencies, key types
2. **Understanding** - From RLM: how it works, patterns used
3. **Connections** - How does it relate to other scoped components?
4. **Issues** - Any problems related to the question?

Record findings:
```
exam_record(
  component="kelpie-storage",
  summary="Per-actor KV storage with SQLite backend",
  details="Implements ActorKV trait. Uses WAL mode for durability...",
  connections=["kelpie-core", "kelpie-runtime"],
  issues=[
    {
      "severity": "medium",
      "description": "Missing compaction tests",
      "evidence": "No tests for compaction in storage_dst.rs"
    }
  ]
)
```

### Step 4: Verify Completeness

**BEFORE ANSWERING**, verify you've examined all scoped components:

```
exam_complete()
```

**If `can_answer: false`:**
- You still have remaining components
- DO NOT answer yet
- Continue examining

**If `can_answer: true`:**
- All components examined
- You may now provide your answer

### Step 5: Provide Thorough Answer

Now that you've examined all relevant components, provide your answer:

1. **Direct answer** - Answer the question clearly
2. **Supporting details** - Reference what you found in each component
3. **Connections** - How the pieces fit together
4. **Caveats** - Any limitations or uncertainty in your answer
5. **Related issues** - Surface any problems you found

### Step 6: Cite Evidence

Always include evidence for claims:

```markdown
The storage layer uses SQLite with WAL mode (kelpie-storage/src/sqlite.rs:45).
It connects to the runtime via the ActorKV trait (kelpie-core/src/traits.rs:120).
```

## Example Interaction

**User:** "How does the actor activation flow work?"

**You:**
1. Start examination with scope: `["kelpie-runtime", "kelpie-registry", "kelpie-core"]`
2. Examine kelpie-runtime: Find activation logic in dispatcher
3. Examine kelpie-registry: Find placement and discovery
4. Examine kelpie-core: Find ActorId and activation types
5. Call `exam_complete()` - verify can_answer: true
6. Provide thorough answer with references to all three components

## Quality Standards

### Completeness Requirements

- ALL scoped components must be examined before answering
- `exam_complete()` MUST return `can_answer: true`
- You MUST NOT skip the completeness check

### Answer Requirements

- Reference specific files and line numbers
- Explain how components work together
- Acknowledge uncertainty if present
- Surface related issues

### What Makes a Thorough Answer

**Shallow answer (BAD):**
> "Actor activation is handled by the runtime. It creates the actor when needed."

**Thorough answer (GOOD):**
> "Actor activation follows this flow:
>
> 1. **Request arrives** at the dispatcher (kelpie-runtime/src/dispatcher.rs:89)
> 2. **Registry lookup** determines if actor exists (kelpie-registry/src/placement.rs:45)
> 3. **If not active**, the runtime creates a new instance using the ActorFactory
> 4. **State is loaded** from storage if it exists (kelpie-storage/src/sqlite.rs:120)
> 5. **Actor starts** and processes the message
>
> The single-activation invariant is enforced by the registry's distributed lock (kelpie-registry/src/lock.rs:30).
>
> Related issue: The activation timeout is not configurable (medium severity)."

## Scope Selection Guide

| Question Type | Likely Scope |
|---------------|--------------|
| "How does X work?" | Component implementing X + dependencies |
| "Where is X?" | Use `index_symbols` first, then examine containing component |
| "Why does X happen?" | Component where X occurs + connected components |
| "Is X tested?" | Main component + kelpie-dst |
| "What's the architecture?" | Use `/codebase-map` instead |

## When to Expand Scope

If during examination you discover the answer requires additional components:

1. Note which additional components are needed
2. Call `exam_start` again with expanded scope (or continue with current)
3. Examine the additional components
4. Verify completeness again

## Tips

1. **Scope conservatively** - Start with obvious components, expand if needed
2. **Read tests** - Tests often explain intended behavior
3. **Follow the types** - Core types reveal the design
4. **Check for TODOs** - May indicate incomplete implementation
5. **Verify claims** - Don't assume, check the code
