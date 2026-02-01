#!/bin/bash
# Install Kelpie Git Hooks
#
# This script installs git hooks that enforce hard controls and auto-indexing.
# Run this after cloning the repository.

set -e

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "🔧 Installing Kelpie Git Hooks..."
echo ""

# Check if git directory exists
if [ ! -d "$REPO_ROOT/.git" ]; then
    echo "❌ Error: .git directory not found"
    echo "Are you in a git repository?"
    exit 1
fi

# Install pre-commit hook
PRECOMMIT_SOURCE="$REPO_ROOT/hooks/pre-commit"
PRECOMMIT_DEST="$REPO_ROOT/.git/hooks/pre-commit"

if [ ! -f "$PRECOMMIT_SOURCE" ]; then
    echo "❌ Error: pre-commit hook source not found at $PRECOMMIT_SOURCE"
    exit 1
fi

if [ -f "$PRECOMMIT_DEST" ]; then
    echo "⚠️  Existing pre-commit hook found, backing up..."
    cp "$PRECOMMIT_DEST" "$PRECOMMIT_DEST.backup.$(date +%s)"
fi

cp "$PRECOMMIT_SOURCE" "$PRECOMMIT_DEST"
chmod +x "$PRECOMMIT_DEST"
echo "✅ Pre-commit hook installed"

# Install post-commit hook
POSTCOMMIT_SOURCE="$REPO_ROOT/hooks/post-commit"
POSTCOMMIT_DEST="$REPO_ROOT/.git/hooks/post-commit"

if [ ! -f "$POSTCOMMIT_SOURCE" ]; then
    echo "❌ Error: post-commit hook source not found at $POSTCOMMIT_SOURCE"
    exit 1
fi

if [ -f "$POSTCOMMIT_DEST" ]; then
    echo "⚠️  Existing post-commit hook found, backing up..."
    cp "$POSTCOMMIT_DEST" "$POSTCOMMIT_DEST.backup.$(date +%s)"
fi

cp "$POSTCOMMIT_SOURCE" "$POSTCOMMIT_DEST"
chmod +x "$POSTCOMMIT_DEST"
echo "✅ Post-commit hook installed"

echo ""
echo "Hooks installed:"
echo ""
echo "1. Pre-commit (enforces working code):"
echo "   - Constraint checks (from .kelpie-index/constraints/)"
echo "   - cargo fmt --check"
echo "   - cargo clippy (with warnings as errors)"
echo "   - cargo test (full suite)"
echo ""
echo "2. Post-commit (keeps indexes fresh):"
echo "   - Detects changed .rs and Cargo.toml files"
echo "   - Runs incremental index rebuild"
echo "   - Prevents stale index issues"
echo ""
echo "To test the hooks:"
echo "  git commit --allow-empty -m 'test hooks'"
echo ""
