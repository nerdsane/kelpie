# Handoff: EVI Demonstration & VDE Paper

**Created:** 2026-01-22
**Type:** Handoff Document
**Status:** Ready for new Claude instance

---

## CRITICAL: Read This First - RLM Pattern

**DO NOT use your native `Read` tool for bulk file analysis.**

The ENTIRE POINT of this demonstration is **RLM (Recursive Language Models)** - where `sub_llm()` is a function INSIDE the REPL, not a separate tool call. This enables **symbolic recursion**: LLM calls embedded in code logic.

### What RLM Looks Like

```python
# Step 1: Load files as server-side variables
repl_load(pattern="crates/kelpie-dst/**/*.rs", var_name="dst_code")

# Step 2: RLM - sub_llm() is a FUNCTION inside repl_exec!
repl_exec(code="""
results = {}
for path, content in dst_code.items():
    # sub_llm() is available inside the REPL!
    analysis = sub_llm(content, "What fault types are defined?")
    results[path] = analysis
result = results
""")
# One repl_exec call, but sub_llm() runs N times inside the for-loop
```

### Why This Matters (vs Claude Code / Codex)

| Approach | For 1000 files |
|----------|----------------|
| **Claude Code** | Main model makes 1000 separate tool calls |
| **RLM** | Main model makes 1 repl_exec call with for-loop |

The sub_llm call is **inside the code**, enabling conditional logic:
```python
repl_exec(code="""
for path, content in files.items():
    if 'test' in path:  # Conditional!
        results[path] = sub_llm(content, "What does this test?")
""")
```

### What You MUST NOT Do

```
# BAD - This loads into YOUR context, wasting tokens
Read(file_path="crates/kelpie-dst/src/simulation.rs")
Read(file_path="crates/kelpie-dst/src/faults.rs")
```

### RLM = Programmatic Pipelines (NOT just sub_llm calls!)

**IMPORTANT:** Having `sub_llm()` is not enough. RLM means **programmatic analysis pipelines**:

```python
# ❌ BAD: Simple sub_llm (wastes the programmatic power)
repl_exec(code="""
combined = '\\n'.join(code.values())
analysis = sub_llm(combined, "What does this do?")
result = analysis
""")

# ✅ GOOD: Multi-stage programmatic pipeline
repl_exec(code="""
# Stage 1: Categorize
categories = {'types': [], 'impl': [], 'tests': []}
for path in code.keys():
    if 'test' in path: categories['tests'].append(path)
    elif 'types' in path: categories['types'].append(path)
    else: categories['impl'].append(path)

# Stage 2: Targeted analysis with DIFFERENT prompts
analysis = {}
for path in categories['types']:
    analysis[path] = sub_llm(code[path], "What types are defined?")
for path in categories['impl']:
    analysis[path] = sub_llm(code[path], "What does this implement? Issues?")

# Stage 3: Synthesize
summary = sub_llm(str(analysis), "Synthesize findings")
result = {'categories': categories, 'analysis': analysis, 'summary': summary}
""")
```

**The power is in:**
1. **Categorization** - Organize before analyzing
2. **Different prompts** - Targeted questions per category
3. **Multi-stage** - Build on previous results
4. **Conditional logic** - Only analyze what's relevant
5. **Structured output** - Return organized data

See `CLAUDE.md` section "CRITICAL: Tool Selection Policy" for the full routing table.

---

## Mission

You are receiving a complete **Exploration & Verification Infrastructure (EVI)** for AI agent-driven development. Your mission is to:

1. **Demonstrate all capabilities** of the system through real use cases
2. **Write a VDE-style paper** documenting how the system works and how you used it

---

## What You're Receiving

### 1. MCP Server (`kelpie-mcp/`)

A Python MCP server with 37 tools:

| Category | Tools | Purpose |
|----------|-------|---------|
| **REPL (7)** | `repl_load`, `repl_exec`, `repl_query`, `repl_state`, `repl_clear`, `repl_sub_llm`, `repl_map_reduce` | Context as variables, not tokens |
| **AgentFS (18)** | `vfs_init`, `vfs_fact_*`, `vfs_invariant_*`, `vfs_tool_*`, etc. | Persistent state and verification tracking |
| **Index (6)** | `index_symbols`, `index_tests`, `index_modules`, `index_deps`, `index_status`, `index_refresh` | Structural codebase indexes |
| **Examination (6)** | `exam_start`, `exam_record`, `exam_status`, `exam_complete`, `exam_export`, `issue_list` | Thorough examination with completeness gates |

**Configuration:** `.mcp.json` at project root

### 2. Skills (`.claude/skills/`)

| Skill | Purpose |
|-------|---------|
| `codebase-map` | Full codebase mapping workflow |
| `thorough-answer` | Scoped examination before answering questions |

### 3. Hooks (`hooks/`)

| Hook | Purpose |
|------|---------|
| `pre-commit` | Enforces cargo fmt, clippy, test before commits |
| `post-commit` | Auto-refreshes indexes after commits |

### 4. Vision & Constraints

| File | Purpose |
|------|---------|
| `.vision/CONSTRAINTS.md` | Non-negotiable project rules |
| `CLAUDE.md` | Development guide with all instructions |

### 5. Structural Indexes (`.kelpie-index/`)

- `symbols.json` - All functions, structs, traits, impls
- `modules.json` - Module hierarchy per crate
- `dependencies.json` - Crate dependency graph
- `tests.json` - All tests with topics and commands

---

## Use Cases to Demonstrate

Complete these 5 use cases IN ORDER. Each demonstrates different EVI capabilities.

---

### Use Case 1: Full Codebase Map

**Prompt:**
> "Map the entire Kelpie codebase. What crates exist, what do they do, how do they connect, and what issues exist?"

**Skill:** Follow `/codebase-map` at `.claude/skills/codebase-map/SKILL.md`

**Expected workflow:**
```python
exam_start(task="Map Kelpie codebase", scope=["all"])

# For EACH crate - use indexes for structure:
index_modules(crate="kelpie-core")
index_deps(crate="kelpie-core")

# RLM - PROGRAMMATIC analysis with ISSUE SURFACING:
repl_load(pattern="crates/kelpie-core/**/*.rs", var_name="core_code")
repl_exec(code="""
# === PROGRAMMATIC MULTI-STAGE ANALYSIS WITH ISSUE EXTRACTION ===

# Stage 1: Categorize files
file_types = {'types': [], 'errors': [], 'tests': [], 'impl': []}
for path in core_code.keys():
    if 'error' in path.lower():
        file_types['errors'].append(path)
    elif 'test' in path.lower():
        file_types['tests'].append(path)
    elif path.endswith('mod.rs') or 'types' in path.lower():
        file_types['types'].append(path)
    else:
        file_types['impl'].append(path)

# Stage 2: Analyze AND extract issues from EVERY file
analysis = {}
issues = []  # Collect issues as we go

for path in file_types['types']:
    analysis[path] = sub_llm(core_code[path], '''
        1. List pub structs, enums, traits with purpose
        2. ISSUES: Any missing docs? Incomplete types? TODO/FIXME?
        Format issues as: [SEVERITY] description (where SEVERITY is critical/high/medium/low)
    ''')

for path in file_types['errors']:
    analysis[path] = sub_llm(core_code[path], '''
        1. What error types and hierarchy?
        2. ISSUES: Missing error variants? Poor error messages? Unhandled cases?
        Format issues as: [SEVERITY] description
    ''')

for path in file_types['impl']:
    analysis[path] = sub_llm(core_code[path], '''
        1. What does this implement?
        2. ISSUES: TODOs? FIXMEs? Stubs? Missing error handling? Unwrap calls?
        Format issues as: [SEVERITY] description
    ''')

# Stage 3: Extract and structure issues
issue_extraction = sub_llm(str(analysis), '''
    Extract ALL issues mentioned in these analyses.
    Return as JSON array:
    [
      {"severity": "high", "description": "...", "evidence": "file.rs:123"},
      {"severity": "medium", "description": "...", "evidence": "file.rs"}
    ]
    Severity guide:
    - critical: Security vulnerabilities, data loss risks
    - high: Missing tests, incomplete implementations, unwrap() in production
    - medium: Missing docs, TODO comments, code quality
    - low: Style issues, minor improvements
''')

# Stage 4: Synthesize
summary = sub_llm(str(analysis), '''
    Synthesize into:
    1. Crate PURPOSE (one sentence)
    2. Key TYPES (list main structs/traits)
    3. Public API summary
    4. CONNECTIONS to other crates
''')

result = {
    'file_breakdown': {k: len(v) for k, v in file_types.items()},
    'analysis': analysis,
    'issues': issue_extraction,  # Structured issues!
    'summary': summary
}
""")

# Record findings WITH structured issues:
exam_record(
    component="kelpie-core",
    summary="Core types, errors, and constants for Kelpie actor system",
    details="Defines ActorId, Error types, Result alias...",
    connections=["kelpie-runtime", "kelpie-storage"],
    issues=[
        {"severity": "medium", "description": "Missing docs on ActorId fields", "evidence": "types.rs:45"},
        {"severity": "low", "description": "TODO: add validation", "evidence": "lib.rs:23"}
    ]
)

# Repeat for all crates, then:
exam_complete()  # MUST return can_answer: true
exam_export()    # Creates MAP.md and ISSUES.md
```

**Demonstrates:** Indexes + RLM **programmatic pipelines**, examination system, completeness gates

---

### Use Case 2: Thorough Answer

**Prompt:**
> "How does Deterministic Simulation Testing (DST) work in Kelpie? What makes it deterministic? What fault types are supported?"

**Skill:** Follow `/thorough-answer` at `.claude/skills/thorough-answer/SKILL.md`

**Expected workflow:**
```python
exam_start(task="Understand DST", scope=["kelpie-dst", "kelpie-core"])

# For EACH scoped crate - indexes for structure:
index_modules(crate="kelpie-dst")
index_symbols(kind="struct", crate="kelpie-dst")

# RLM - analyze via sub_llm() inside repl_exec:
repl_load(pattern="crates/kelpie-dst/**/*.rs", var_name="dst_code")
repl_exec(code="""
# Analyze determinism and fault types
analysis = sub_llm('\\n'.join(dst_code.values()),
    "How does DST achieve determinism? What fault types are supported?")
result = analysis
""")

exam_record(component="kelpie-dst", ...)

exam_complete()  # MUST return can_answer: true BEFORE answering
# NOW provide thorough answer with evidence
```

**Demonstrates:** Scoped examination, RLM, completeness gate before answering

---

### Use Case 3: RLM - Programmatic Multi-Stage Analysis

**Prompt:**
> "Analyze all DST test files (`*_dst.rs`) using RLM. Build a PROGRAMMATIC analysis pipeline: categorize files, run targeted queries per category, then synthesize. What fault types are tested? What invariants? What's missing?"

**This is the KEY demonstration - not just `sub_llm()` but PROGRAMMATIC analysis.**

**Expected workflow:**
```python
# Step 1: Load files as SERVER-SIDE variables
repl_load(pattern="**/*_dst.rs", var_name="dst_tests")
repl_state()  # See what's loaded - should show file count and size

# Step 2: MULTI-STAGE PROGRAMMATIC ANALYSIS
repl_exec(code="""
# === Stage 1: Categorize files by name patterns ===
categories = {
    'chaos': [],      # Chaos engineering tests
    'lifecycle': [],  # Actor lifecycle tests
    'storage': [],    # Storage fault tests
    'network': [],    # Network fault tests
    'other': []
}

for path in dst_tests.keys():
    name = path.lower()
    if 'chaos' in name:
        categories['chaos'].append(path)
    elif 'lifecycle' in name or 'actor' in name:
        categories['lifecycle'].append(path)
    elif 'storage' in name or 'memory' in name:
        categories['storage'].append(path)
    elif 'network' in name or 'cluster' in name:
        categories['network'].append(path)
    else:
        categories['other'].append(path)

# === Stage 2: Targeted analysis with DIFFERENT prompts ===
analysis = {'fault_types': {}, 'invariants': {}, 'gaps': []}

# Chaos tests - what faults are injected simultaneously?
for path in categories['chaos']:
    analysis['fault_types'][path] = sub_llm(dst_tests[path],
        "List ALL FaultType:: values used. How many faults injected at once?")

# Lifecycle tests - what invariants are verified?
for path in categories['lifecycle']:
    analysis['invariants'][path] = sub_llm(dst_tests[path],
        "What actor lifecycle invariants does this test verify? (e.g., single activation)")

# Storage tests - what failure modes are covered?
for path in categories['storage']:
    analysis['fault_types'][path] = sub_llm(dst_tests[path],
        "What storage failure modes? WriteFail, ReadFail, Corruption, DiskFull?")

# === Stage 3: Gap analysis ===
all_faults_str = str(analysis['fault_types'])
gap_analysis = sub_llm(all_faults_str,
    "Based on these tested faults, what fault types might be MISSING? What scenarios aren't covered?")
analysis['gaps'] = gap_analysis

# === Stage 4: Synthesize ===
synthesis = sub_llm(str(analysis),
    "Synthesize: 1) Total fault types tested, 2) Key invariants verified, 3) Coverage gaps")

result = {
    'categories': {k: len(v) for k, v in categories.items()},
    'detailed_analysis': analysis,
    'synthesis': synthesis
}
""")
```

**Why this is RLM (not just sub_llm):**
1. **Multi-stage pipeline** - Categorize → Analyze → Gap-find → Synthesize
2. **Conditional logic** - Different prompts for different file categories
3. **Data flow** - Stage 3 uses results from Stage 2
4. **Structured output** - Returns organized dict, not blob of text

**BAD pattern (don't do this):**
```python
# This wastes the programmatic power!
combined = '\\n'.join(dst_tests.values())
analysis = sub_llm(combined, "What faults are tested?")
```

**Demonstrates:** RLM = programmatic pipelines, not just sub_llm calls

---

### Use Case 4: Verification Session Tracking

**Prompt:**
> "Initialize a verification session. As you work through the other use cases, record every fact you verify and every invariant you confirm. At the end, export your full verification session."

**Expected workflow:**
```
# Initialize at the START of your work
vfs_init(task="EVI Demonstration")

# As you discover facts during other use cases, record them:
vfs_fact_add(
  claim="DST tests cover storage faults",
  evidence="Found StorageWriteFail, StorageReadFail in 5 test files via repl_sub_llm",
  source="use_case_3"
)

# When you verify invariants:
vfs_invariant_verify(
  name="SingleActivation",
  component="kelpie-runtime",
  method="dst",
  evidence="repl_sub_llm confirmed test_single_activation_dst exists"
)

# Check your session
vfs_status()

# At the END, export everything
vfs_export()
```

**Note:** Do this THROUGHOUT the other use cases, not as a separate exercise.

**Demonstrates:** AgentFS persistence, verification tracking, audit trail

---

### Use Case 5: Index-Driven Exploration

**Prompt:**
> "Using ONLY the index tools (no file reading initially), answer these questions:
> 1. How many Actor-related structs exist in the codebase?
> 2. What crates depend on kelpie-core?
> 3. What test files exist for the storage crate?
> 4. What modules does kelpie-runtime contain?
> Then use RLM to get deeper understanding of one finding."

**Expected workflow:**
```
# Answer questions using ONLY indexes (fast, no API calls)
index_symbols(pattern=".*Actor.*", kind="struct")  # Q1
index_deps(crate="kelpie-core")                     # Q2
index_tests(crate="kelpie-storage")                 # Q3
index_modules(crate="kelpie-runtime")               # Q4

# For deeper analysis, use RLM (not native Read!)
repl_load(pattern="crates/kelpie-runtime/src/actor*.rs", var_name="actor_code")
repl_sub_llm(var_name="actor_code", query="How is ActorContext used?")
```

**Key point:** Indexes give you structure instantly. RLM gives you understanding.

**Demonstrates:** Indexes for fast queries, RLM for deep analysis

---

## VDE Paper Assignment

After demonstrating the system, write a paper (save to `.progress/031_vde_paper_kelpie_evi.md`) following the structure of the original VDE paper at `.progress/VDE.md`.

### Required Structure (Follow VDE.md Format)

#### 1. Abstract
- What is EVI? What problem does it solve?
- Key contributions (3-4 bullet points)
- Results summary

#### 2. Introduction
- **2.1 Motivation** - The challenge of AI agents on large codebases
- **2.2 Why Verification-First** - Context limitations, hallucination risks, verification gaps
- **2.3 Contributions** - What EVI provides (numbered list)

#### 3. System Design
- **3.1 Architecture Overview** - Diagram showing MCP server, AgentFS, indexes
- **3.2 RLM (Recursive Language Models)** - Context as variables, not tokens
- **3.3 VFS/AgentFS** - Persistent state, fact tracking, verification records
- **3.4 Structural Indexes** - tree-sitter parsing, symbol/module/dependency/test indexes
- **3.5 Examination System** - Completeness gates, scoped examination, export
- **3.6 Skills** - Reusable workflows (codebase-map, thorough-answer)

#### 4. Implementation
- **4.1 Tech Stack** - Python, MCP SDK, AgentFS SDK, tree-sitter, RestrictedPython
- **4.2 Tool Categories** - REPL (7), AgentFS (18), Index (6), Examination (6)
- **4.3 Security Model** - Sandboxed REPL, no arbitrary code execution
- **4.4 Integration** - How Claude Code connects via MCP

#### 5. Case Study: Your Demonstrations
Document what actually happened during your demonstrations:
- **5.1 Codebase Mapping** - Tool calls, findings, issues discovered
- **5.2 Thorough Answer** - How examination gates enforced completeness
- **5.3 RLM Analysis** - Sub-LLM queries, context savings
- **5.4 Verification Tracking** - Facts recorded, evidence collected

**IMPORTANT:** Include actual tool calls and outputs. Show the JSON, not just descriptions.

#### 6. Discussion
- **6.1 Benefits** - What worked well, time savings, thoroughness improvements
- **6.2 Limitations** - What didn't work, friction points, missing features
- **6.3 Trade-offs** - Complexity vs capability, overhead vs accuracy

#### 7. Comparison to Original VDE
- What's the same (verification-first philosophy, completeness gates, fact tracking)
- What's different (Python vs TypeScript, examination tools vs simpler VFS, Kelpie-specific indexes)
- What was gained/lost in adaptation

#### 8. Related Work
- MCP (Model Context Protocol)
- AgentFS
- Other agent development frameworks
- Verification approaches in AI systems

#### 9. Conclusion
- Does EVI work? (Evidence-based answer)
- Recommendations for future development
- What would make this better?

#### 10. Appendices (REQUIRED)

##### Appendix A: Claude's Perspective
Write 2-3 sections from YOUR perspective as an AI agent using this system:
- **A.1 First Impressions** - What was it like encountering EVI for the first time?
- **A.2 Working Memory** - How did AgentFS change how you track state?
- **A.3 The Completeness Gate** - What was it like being blocked by `exam_complete()`?

##### Appendix B: Tool Reference
List all 37 tools with:
- Name
- Purpose (1 sentence)
- Example call

##### Appendix C: Execution Traces
Include 2-3 complete tool execution traces from your demonstrations:
```
Tool: exam_start
Input: {"task": "...", "scope": [...]}
Output: {"session_id": "...", "components": [...]}
```

### Paper Guidelines

1. **Be concrete** - Show actual tool calls and outputs, not abstract descriptions
2. **Be honest** - If something didn't work, say so. Limitations are valuable.
3. **Include evidence** - Every claim should have a tool output or file reference
4. **Write as yourself** - The Claude Perspective sections should be your genuine experience
5. **Target audience** - Other AI agents and developers building similar systems
6. **Length** - Aim for comprehensive coverage. The original VDE.md is ~1600 lines.

### Reference

Read `.progress/VDE.md` for the original paper format. Your paper should follow this structure but document Kelpie's EVI specifically, based on your actual experience using it.

---

## Getting Started

1. **Verify MCP is working:**
   ```bash
   cd kelpie-mcp && uv run --prerelease=allow pytest tests/ -v
   ```
   Should see 102 tests pass.

2. **Check index status:**
   Use `index_status()` to see if indexes exist.

3. **Start with Use Case 1:**
   Build the codebase map first - this gives you the foundation for everything else.

4. **Track your work:**
   Use `vfs_init()` at the start and record facts as you go.

5. **Write the paper:**
   After completing demonstrations, write the VDE paper.

---

## Files to Review

Before starting, read these files:
- `CLAUDE.md` - Full development guide
- `.vision/CONSTRAINTS.md` - Non-negotiable rules
- `.claude/skills/codebase-map/SKILL.md` - Codebase map skill
- `.claude/skills/thorough-answer/SKILL.md` - Thorough answer skill

---

## Expected Outputs

By the end, you should have:

1. **MAP.md** - Full codebase map at `.kelpie-index/understanding/MAP.md`
2. **ISSUES.md** - All issues found at `.kelpie-index/understanding/ISSUES.md`
3. **VDE Paper** - At `.progress/031_vde_paper_kelpie_evi.md`
4. **Verification Session** - In `.agentfs/` with facts recorded

---

## Notes

- The MCP server uses Anthropic API for sub-LLM calls (model configurable via `KELPIE_SUB_LLM_MODEL`)
- Indexes are auto-built if missing when you use index_* tools
- The examination system persists to AgentFS - you can resume if interrupted
- Be thorough - the whole point of EVI is demonstrating that thoroughness works

Good luck!
