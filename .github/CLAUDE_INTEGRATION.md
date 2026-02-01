# Claude Code GitHub Integration

This repository is configured to work with Claude Code via @claude mentions in issues and PRs.

## Setup Status

✅ **Workflow configured** - `.github/workflows/claude.yml`
✅ **Documentation updated** - `CLAUDE.md` includes usage guide
⚠️  **API key needed** - Must add `ANTHROPIC_API_KEY` to repository secrets

## Required: Add API Key

Before using @claude mentions, you must add your Anthropic API key:

1. Go to repository Settings → Secrets and variables → Actions
2. Click "New repository secret"
3. Name: `ANTHROPIC_API_KEY`
4. Value: Your Anthropic API key (starts with `sk-ant-...`)
5. Click "Add secret"

## How to Use

### In Issues

Open an issue or comment with:
```markdown
@claude please implement bounded liveness testing for actor leases (Issue #40).
Follow the DST approach from CLAUDE.md and create a PR when tests pass.
```

### In Pull Requests

Add a comment:
```markdown
@claude review this PR for TigerStyle compliance and verification-first principles.
Check that all functions have 2+ assertions.
```

## What Happens

1. **Workflow triggers** - GitHub Actions runs `.github/workflows/claude.yml`
2. **Claude analyzes** - Reads issue/comment, CLAUDE.md, vision files
3. **Claude works** - Creates branches, writes code, runs tests
4. **Claude reports** - Comments back with progress/results
5. **Creates PR** - If implementing a feature, opens a pull request

## Testing the Integration

After adding the API key, test with this issue:

**Title:** Test @claude integration
**Body:**
```markdown
@claude please analyze the current test coverage in kelpie-dst and report:
1. How many DST tests exist
2. Which fault types are covered
3. Any gaps in coverage
```

Claude should respond within a few minutes with an analysis comment.

## Verification Standards

Claude follows kelpie's strict verification requirements:
- ✅ All tests must pass (`cargo test`)
- ✅ No clippy warnings (`cargo clippy`)
- ✅ Code formatted (`cargo fmt`)
- ✅ DST coverage for critical paths
- ✅ TigerStyle compliance (2+ assertions per function)
- ✅ No stubs or TODOs in production code

## Example Workflows

### Feature Implementation
```markdown
@claude implement the actor teleportation feature from docs/adr/006.
Create a plan in .progress/ first, then implement with DST coverage.
Open a PR when all tests pass.
```

### Bug Investigation
```markdown
@claude investigate why actor state persistence is failing in test_actor_recovery.
Run the test with different DST seeds and identify the root cause.
```

### Code Review
```markdown
@claude review this PR against CONSTRAINTS.md requirements.
Check for:
- Big-endian naming violations
- Missing assertions (need 2+ per function)
- Implicit truncation (u64 → u32)
- Unwrap() in production code
```

## Troubleshooting

**@claude doesn't respond:**
- Check that `ANTHROPIC_API_KEY` is added to repository secrets
- View workflow run: Actions tab → "Claude Code" workflow
- Check workflow logs for errors

**Workflow fails:**
- Verify API key is valid and not expired
- Check that all required permissions are granted
- Review workflow logs in Actions tab

**Claude makes wrong changes:**
- Be more specific in your request
- Reference specific ADRs or vision documents
- Ask Claude to create a plan first before implementing

## Cost Management

Each @claude invocation uses Anthropic API credits:
- Simple analysis: ~$0.01-0.05
- Feature implementation: ~$0.10-1.00
- Large refactoring: ~$1.00-5.00

Monitor usage in your Anthropic Console.

## Security

- API key is stored securely in GitHub Secrets (encrypted)
- Claude can only access public repository content
- All changes go through PRs (review before merging)
- Claude cannot merge PRs or modify repository settings

## Support

- [Claude Code Documentation](https://docs.anthropic.com/claude-code)
- [GitHub Actions Docs](https://docs.github.com/actions)
- [Kelpie CLAUDE.md](../CLAUDE.md) - Full development guide
