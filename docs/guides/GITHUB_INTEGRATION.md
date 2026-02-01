# GitHub Integration

Kelpie has Claude Code integration via GitHub Actions. You can tag @claude in issues and pull requests to automatically trigger Claude Code workflows.

## Setup Requirements

1. **GitHub Secret**: `ANTHROPIC_API_KEY` must be set in repository settings
   - Go to Settings → Secrets and variables → Actions
   - Add new secret: `ANTHROPIC_API_KEY` with your Anthropic API key

2. **Workflow File**: `.github/workflows/claude.yml` (already configured)

## How to Use

**In Issues:**
```markdown
@claude please implement this feature following CLAUDE.md guidelines
```

**In Issue Comments:**
```markdown
@claude can you analyze this bug and suggest a fix?
```

**In Pull Request Comments:**
```markdown
@claude review this PR against our verification requirements
```

## What Claude Will Do

When you mention @claude:
1. **Read context** - Issue/comment body, CLAUDE.md, and `.vision/CONSTRAINTS.md`
2. **Follow kelpie's standards** - TigerStyle, verification-first, DST coverage
3. **Create branches and PRs** - If implementing features
4. **Run verification** - `cargo test`, `cargo clippy`, `cargo fmt`
5. **Comment back** - Progress updates, questions, or completion status

## Best Practices

- **Be specific** - "Implement X following ADR-004" better than "fix this"
- **Reference vision files** - Claude reads CLAUDE.md and CONSTRAINTS.md automatically
- **Expect verification** - Claude will run tests before claiming completion
- **Check progress** - Claude updates the issue/PR with progress comments

## Example Workflows

**Feature Implementation:**
```markdown
@claude implement the bounded liveness testing feature described in Issue #40.
Follow the DST testing approach from CLAUDE.md and create a PR when tests pass.
```

**Bug Fix:**
```markdown
@claude investigate the actor lifecycle bug. Run relevant tests, identify root cause,
and propose a fix with DST coverage.
```

**Code Review:**
```markdown
@claude review this PR for TigerStyle compliance, DST coverage, and verification-first
principles. Check that all assertions follow the 2+ per function guideline.
```
