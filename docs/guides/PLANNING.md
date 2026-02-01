# Vision-Aligned Planning Guide

## Before Starting ANY Non-Trivial Task

**STOP.** Before starting work that requires 3+ steps, touches multiple files, or needs research, you MUST:

### 1. Check for Vision Files

- **Read `.vision/CONSTRAINTS.md`** - Non-negotiable rules and principles
- **Read `docs/VISION.md`** - Project goals and architecture
- **Read existing `.progress/` plans** - Understand current state

### 2. Create a Numbered Plan File

**ALWAYS** save to `.progress/NNN_YYYYMMDD_HHMMSS_task-name.md` BEFORE writing code.

- `NNN` = next sequence number (001, 002, etc.)
- Use `.progress/templates/plan.md` as the template
- Fill in ALL required sections (see template)

**DO NOT skip planning. DO NOT start coding without a plan file.**

### 3. Required Plan Sections (DO NOT SKIP)

These sections are **MANDATORY**:

1. **Options & Decisions**
   - List 2-3 options considered for each major decision
   - Explain pros/cons of each
   - State which option chosen and WHY (reasoning)
   - List trade-offs accepted

2. **Quick Decision Log**
   - Log ALL decisions, even small ones
   - Format: Time | Decision | Rationale | Trade-off
   - This is your audit trail

3. **What to Try** (UPDATE AFTER EVERY PHASE)
   - Works Now: What user can test, exact steps, expected result
   - Doesn't Work Yet: What's missing, why, when expected
   - Known Limitations: Caveats, edge cases

**If you skip these sections, the plan is incomplete.**

## During Execution

1. **Update plan after each phase** - Mark phases complete, log findings
2. **Log decisions in Quick Decision Log** - Every choice, with rationale
3. **Update "What to Try" after EVERY phase** - Not just at the end
4. **Re-read plan before major decisions** - Keeps goals in attention window
5. **Document deviations** - If implementation differs from plan, note why

**The 2-Action Rule:** After every 2 significant operations, save key findings to the plan file.

## Before Completion

1. **Verify required sections are filled** - Options, Decision Log, What to Try
2. **Run verification checks:**
   ```bash
   cargo test           # All tests must pass
   cargo clippy         # Fix all warnings
   cargo fmt --check    # Code must be formatted
   ```
3. **Run `/no-cap`** - Verify no hacks, placeholders, or incomplete code
4. **Check vision alignment** - Does result match CONSTRAINTS.md requirements?
5. **Verify DST coverage** - Critical paths have simulation tests?
6. **Update plan status** - Mark as complete with verification status
7. **Commit and push** - Use conventional commit format

## Multi-Instance Coordination

When multiple Claude instances work on shared tasks:
- Read `.progress/` plans before starting work
- Claim phases in the Instance Log section
- Update status frequently to avoid conflicts
- Use findings section for shared discoveries

## Plan File Format

`.progress/NNN_YYYYMMDD_HHMMSS_descriptive-task-name.md`

Where:
- `NNN` = sequence number (001, 002, 003, ...)
- `YYYYMMDD_HHMMSS` = timestamp
- `descriptive-task-name` = kebab-case description

Example: `.progress/001_20260112_120000_add-fdb-backend.md`

## Quick Workflow Reference

```
┌─────────────────────────────────────────────────────────────┐
│  Before Starting                                            │
│  1. Read .vision/CONSTRAINTS.md                             │
│  2. Read existing .progress/ plans                          │
│  3. Create new numbered plan file                           │
│  4. Fill in: Options, Decisions, Quick Log                  │
├─────────────────────────────────────────────────────────────┤
│  During Work                                                │
│  1. Update plan after each phase                            │
│  2. Log all decisions                                       │
│  3. Update "What to Try" section                            │
│  4. Re-read plan before big decisions                       │
├─────────────────────────────────────────────────────────────┤
│  Before Completing                                          │
│  1. cargo test && cargo clippy && cargo fmt                 │
│  2. Run /no-cap                                             │
│  3. Verify DST coverage                                     │
│  4. Update plan completion notes                            │
│  5. Commit and push                                         │
└─────────────────────────────────────────────────────────────┘
```
