---
name: codebase-map
description: Build a comprehensive map of the entire codebase. Use when user asks to understand the codebase, map out components, or needs a holistic view of what exists. Produces human-readable MAP.md and ISSUES.md files.
---

# Codebase Map Workflow

This skill guides you through building a comprehensive understanding of the entire codebase. It uses the examination tools to ensure thoroughness and produces human-readable documentation.

## When to Use

**Trigger conditions:**
- User asks to "map the codebase" or "understand the codebase"
- User asks "what does this project contain?"
- User needs a holistic view before making architectural decisions
- New to a codebase and needs orientation

## Prerequisites

This skill requires the Kelpie MCP server to be running with examination tools available:
- `exam_start` - Start examination
- `exam_record` - Record findings
- `exam_status` - Check progress
- `exam_complete` - Verify completeness
- `exam_export` - Generate documentation
- `issue_list` - Query issues

## Workflow

### Step 1: Initialize Examination

Start a full codebase examination:

```
exam_start(
  task="Build comprehensive codebase map",
  scope=["all"]
)
```

This will:
- Initialize a verification session
- Discover all crates/modules from the index
- Create a tracking list of components to examine

Note the list of components returned - you must examine ALL of them.

### Step 2: Examine Each Component

For EACH component in the scope, you MUST use **indexes for structure** and **RLM for understanding**:

#### 2a. Get Structure from Indexes (fast, no API calls)

```
# Get module hierarchy
index_modules(crate="kelpie-core")

# Get dependencies (what it uses and what uses it)
index_deps(crate="kelpie-core")

# Get key symbols (structs, traits, functions)
index_symbols(kind="struct", crate="kelpie-core")
index_symbols(kind="trait", crate="kelpie-core")
```

#### 2b. Get Understanding from RLM (DO NOT use native Read tool!)

**KEY CONCEPT: SYMBOLIC RECURSION**

RLM enables **symbolic recursion** - the sub-LLM call lives INSIDE the REPL code.
This is fundamentally different from Claude Code / Codex where sub-agents are separate tool calls.

Available functions inside `repl_exec()`:
- `sub_llm(content, query)` - Sequential sub-LLM call
- `parallel_sub_llm(items, query_or_fn)` - Parallel sub-LLM calls (up to 10 concurrent)

```python
# Load files as server-side variables
repl_load(pattern="crates/kelpie-core/**/*.rs", var_name="core_code")
```

**RLM means PROGRAMMATIC analysis with GUARANTEED execution** - use Python logic with `sub_llm()`:

```python
# GOOD: Multi-stage programmatic analysis with issue extraction
repl_exec(code="""
# Stage 1: Categorize files by purpose
categories = {'types': [], 'errors': [], 'tests': [], 'impl': []}
for path in core_code.keys():
    if 'error' in path.lower():
        categories['errors'].append(path)
    elif 'test' in path.lower():
        categories['tests'].append(path)
    elif 'types' in path.lower() or path.endswith('mod.rs'):
        categories['types'].append(path)
    else:
        categories['impl'].append(path)

# Stage 2: Analyze AND extract issues (ask about issues in EVERY prompt!)
analysis = {}

for path in categories['types']:
    analysis[path] = sub_llm(core_code[path], '''
        1. List pub structs, enums, traits with purpose
        2. ISSUES: Missing docs? Incomplete types? TODO/FIXME comments?
        Format issues as: [SEVERITY] description
    ''')

for path in categories['errors']:
    analysis[path] = sub_llm(core_code[path], '''
        1. What error types and hierarchy?
        2. ISSUES: Missing variants? Poor messages? Unhandled cases?
        Format issues as: [SEVERITY] description
    ''')

for path in categories['impl']:
    analysis[path] = sub_llm(core_code[path], '''
        1. What does this implement?
        2. ISSUES: TODOs? FIXMEs? Stubs? unwrap() calls? Missing error handling?
        Format issues as: [SEVERITY] description
    ''')

# Stage 3: Extract issues into structured format
issues_json = sub_llm(str(analysis), '''
    Extract ALL issues from these analyses as JSON:
    [{"severity": "high|medium|low", "description": "...", "evidence": "file:line"}]

    Severity guide:
    - critical: Security, data loss
    - high: Missing tests, incomplete impl, unwrap() in prod
    - medium: Missing docs, TODOs, code quality
    - low: Style, minor improvements
''')

# Stage 4: Synthesize
summary = sub_llm(str(analysis), '''
    Synthesize: 1) Crate purpose, 2) Key types, 3) Connections to other crates
''')

result = {
    'categories': {k: len(v) for k, v in categories.items()},
    'analysis': analysis,
    'issues': issues_json,  # Structured for exam_record!
    'summary': summary
}
""")
```

**BAD - Don't do this (wastes the programmatic power):**
```python
# BAD: Just concatenating and asking one question
repl_exec(code="""
combined = '\\n'.join(core_code.values())
analysis = sub_llm(combined, "What does this crate do?")
result = analysis
""")
```

**Why symbolic recursion is better:**
1. **Guaranteed execution** - Code runs, unlike hoping a model makes N tool calls
2. **Programmatic control** - for-loops, conditionals, transformations
3. **Parallel execution** - `parallel_sub_llm()` for concurrent processing
4. **Composability** - LLM calls embedded in arbitrary program logic

**FASTER: Use parallel_sub_llm for bulk analysis:**
```python
# PARALLEL analysis - much faster for many files
repl_exec(code="""
items = [{'path': p, 'content': c} for p, c in core_code.items()]

# Same query for all files - runs up to 10 concurrently!
results = parallel_sub_llm(items, '''
    1. What does this file do?
    2. ISSUES: Any TODOs, stubs, unwrap() calls?
    Format issues as: [SEVERITY] description
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
repl_sub_llm(var_name="core_code", query="What patterns are used?")
```

**CRITICAL:** Do NOT use the native `Read` tool to load files into your context.

#### 2c. Record Your Findings

Combine what you learned from indexes (structure) and RLM (understanding):

Then record your findings:

```
exam_record(
  component="<component-name>",
  summary="Brief description of what this component does",
  details="Detailed explanation of how it works, key patterns, etc.",
  connections=["list", "of", "connected", "components"],
  issues=[
    {
      "severity": "high",
      "description": "Description of the issue",
      "evidence": "Where you found it"
    }
  ]
)
```

**Issue Severity Guide:**
- `critical` - Security vulnerabilities, data loss risks, broken functionality
- `high` - Missing tests, incomplete implementations, performance problems
- `medium` - Code quality issues, missing documentation, tech debt
- `low` - Style issues, minor improvements, nice-to-haves

### Step 3: Check Progress Regularly

After examining several components, check your progress:

```
exam_status()
```

This shows:
- How many components examined vs remaining
- Issue counts by severity
- List of remaining components

**DO NOT proceed to export until remaining_count is 0.**

### Step 4: Verify Completeness

Before answering any questions or exporting, verify you've examined everything:

```
exam_complete()
```

This returns:
- `can_answer: true` - All components examined, you may proceed
- `can_answer: false` - Still have remaining components, keep examining

**This is a hard gate. You MUST NOT skip components.**

### Step 5: Export Documentation

Once exam_complete returns `can_answer: true`, export your findings:

```
exam_export(include_details=true)
```

This creates:
- `.kelpie-index/understanding/MAP.md` - Full codebase map
- `.kelpie-index/understanding/ISSUES.md` - All issues found
- `.kelpie-index/understanding/components/<name>.md` - Per-component details

### Step 6: Present Results

After exporting, summarize for the user:

1. **Component Overview** - How many components, main categories
2. **Architecture** - How components connect
3. **Key Issues** - Critical and high severity issues that need attention
4. **Next Steps** - Recommendations based on what you found

Point them to the generated files for full details.

## Quality Standards

### Thoroughness Requirements

- Every component in scope MUST be examined
- Every component MUST have a summary
- Connections MUST be bidirectional (if A connects to B, record it in both)
- Issues MUST have evidence (file path, line number, or observation)

### What to Look For

When examining each component:

1. **Purpose** - What problem does it solve?
2. **Dependencies** - External crates, internal dependencies
3. **Public API** - What does it expose to other components?
4. **Testing** - Are there tests? What's the coverage like?
5. **Documentation** - Is it documented? Is the documentation accurate?
6. **Code Quality** - Follows project conventions? Clean architecture?
7. **Potential Issues** - TODOs, FIXMEs, stubs, missing error handling

### Issue Categories

Common issues to surface:
- Missing or inadequate tests
- Incomplete implementations (TODOs, stubs)
- Security concerns
- Performance problems
- Documentation gaps
- Inconsistent patterns
- Dead or orphaned code
- Missing error handling

## Output Files

### MAP.md Structure

```markdown
# Codebase Map

**Task:** Build comprehensive codebase map
**Generated:** <timestamp>
**Components:** <count>
**Issues Found:** <count>

---

## Components Overview

### <component-name>

**Summary:** Brief description
**Connects to:** list, of, connections

**Details:**
Detailed explanation...

**Issues (N):**
- [HIGH] Description
- [MEDIUM] Description

---

## Component Connections

```
component-a -> component-b, component-c
component-b -> component-a, component-d
```
```

### ISSUES.md Structure

```markdown
# Issues Found During Examination

**Task:** Build comprehensive codebase map
**Total Issues:** <count>

---

## CRITICAL (N)

### [component] Description
**Evidence:** Where found

---

## HIGH (N)
...
```

## Tool Selection (CRITICAL - Symbolic Recursion)

| Need | Tool | Why |
|------|------|-----|
| Module structure | `index_modules` | Fast, pre-built |
| Dependencies | `index_deps` | Fast, pre-built |
| Find symbols | `index_symbols` | Fast, pre-built |
| Bulk analysis (fast) | `repl_load` + `repl_exec` with `parallel_sub_llm()` | Parallel symbolic recursion |
| Sequential analysis | `repl_load` + `repl_exec` with `sub_llm()` | Sequential symbolic recursion |
| Simple query | `repl_sub_llm` tool | Convenience for single queries |
| Single specific file | Native `Read` | OK for 1-2 files you specifically need |

**Symbolic Recursion Pattern:**
- `sub_llm()` and `parallel_sub_llm()` are FUNCTIONS inside the REPL language
- The for-loop/parallel-map GUARANTEES execution (unlike hoping a model makes N tool calls)
- This enables programmatic control: conditionals, transformations, aggregation

**Rule:** If you're about to use `Read` more than 2 times, STOP and use RLM instead.

## Tips

1. **Start with indexes** - Get the structure before diving into code
2. **Use RLM for bulk analysis** - Never load many files into your context
3. **Start with core crates** - Understand the foundation first
4. **Follow dependencies** - Use `index_deps` to see relationships
5. **Read tests via RLM** - `repl_load(pattern="**/*_test.rs")` + sub-LLM
6. **Be honest about gaps** - If something is unclear, say so
