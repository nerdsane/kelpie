# Kelpie Development Guides

This directory contains detailed guides for developing Kelpie. These guides are referenced from the main [CLAUDE.md](../../CLAUDE.md) file.

## Quick Navigation

| Guide | Purpose | When to Read |
|-------|---------|--------------|
| **[Planning](PLANNING.md)** | Vision-aligned planning workflow | **MANDATORY** before starting any non-trivial task (3+ steps, multi-file) |
| **[Verification](VERIFICATION.md)** | Testing pyramid, verification-first development | Before claiming features complete, before commits |
| **[EVI](EVI.md)** | Exploration & Verification Infrastructure, RLM tools | When analyzing codebase, building understanding |

## Planning Guide

**Read first for any non-trivial task.** Covers:
- Vision file requirements (CONSTRAINTS.md, VISION.md)
- Creating numbered plan files (`.progress/NNN_YYYYMMDD_HHMMSS_task-name.md`)
- Required plan sections (Options, Quick Decision Log, What to Try)
- The 2-Action Rule
- Multi-instance coordination

**Key takeaway:** Never start coding without a plan file.

## Verification Guide

**Read before claiming "done".** Covers:
- Verification pyramid (Unit → DST → Integration → Stateright → Kani)
- Trust execution, not documentation
- Verification workflows
- Handoff protocol
- No Stubs Policy
- Commit policy (only working software)

**Key takeaway:** Empirical proof required for all features.

## EVI Guide

**Read when exploring codebase.** Covers:
- Structural indexes (symbols, tests, modules, dependencies)
- MCP server and 37 tools
- RLM (Recursive Language Model) pattern
- Tool selection policy (server-side context vs local context)
- Thorough examination system
- AgentFS storage

**Key takeaway:** Use RLM for multi-file analysis, not native Read tool.

## Quick Decision Trees

### "Should I create a plan file?"

```
Task requires 3+ steps? ────────────────────────────► YES → Read Planning Guide
Task touches multiple files? ──────────────────────► YES → Read Planning Guide
Task needs exploration/research? ──────────────────► YES → Read Planning Guide
Simple 1-file, 1-function change? ─────────────────► NO  → Code directly
```

### "How should I analyze multiple files?"

```
Need to analyze 1-2 specific files? ───────────────► Use Read tool
Need to analyze 3+ files? ─────────────────────────► Use RLM (repl_load + repl_sub_llm)
Need to answer "how does X work?" ─────────────────► Use examination workflow (EVI Guide)
Building codebase map? ────────────────────────────► Use exam_start(scope=["all"])
```

### "Is this feature done?"

```
Tests exist and pass? ──────────────────────────────► Check next
No stubs or TODOs? ─────────────────────────────────► Check next
Manually verified works? ───────────────────────────► Check next
cargo clippy passes? ───────────────────────────────► YES → Feature done
Any NO above? ──────────────────────────────────────► NOT done, keep working
```

## See Also

- [CLAUDE.md](../../CLAUDE.md) - Main development guide (optimized for performance)
- [VISION.md](../VISION.md) - Project goals and architecture
- [ADRs](../adr/) - Architecture Decision Records
