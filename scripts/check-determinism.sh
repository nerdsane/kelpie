#!/bin/bash
# DST Determinism Enforcement Script
#
# This script scans Kelpie source code for direct usage of non-deterministic
# I/O operations that should instead use injected providers (TimeProvider, RngProvider).
#
# Usage:
#   ./scripts/check-determinism.sh [OPTIONS]
#
# Options:
#   --strict       Exit with code 1 on violations (default)
#   --warn-only    Report violations but exit with code 0
#   --help         Show this help message
#
# Exit codes:
#   0 - No violations found (or --warn-only mode)
#   1 - Violations found (--strict mode only)
#
# See: https://github.com/rita-aga/kelpie/issues/23

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Parse arguments
STRICT_MODE=true
for arg in "$@"; do
    case $arg in
        --warn-only)
            STRICT_MODE=false
            ;;
        --strict)
            STRICT_MODE=true
            ;;
        --help|-h)
            sed -n '2,18p' "$0" | sed 's/^# //' | sed 's/^#//'
            exit 0
            ;;
    esac
done

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Change to project root
cd "$PROJECT_ROOT"

echo "=== DST Determinism Check ==="
if [ "$STRICT_MODE" = false ]; then
    echo -e "${YELLOW}Mode: warn-only (violations won't fail CI)${NC}"
fi
echo ""

# Forbidden patterns that bypass DST
# These must use TimeProvider or RngProvider instead
FORBIDDEN_PATTERNS=(
    "tokio::time::sleep"
    "std::thread::sleep"
    "rand::random"
    "thread_rng"
    "SystemTime::now"
    "Instant::now"
)

# Files/paths that are ALLOWED to use these patterns
# These are production implementations or external integrations
EXCEPTION_PATTERNS=(
    # Production time/rng provider implementations (exact files)
    "kelpie-core/src/io.rs"
    "kelpie-core/src/runtime.rs"
    # DST framework itself (needs real time for comparison, seed generation)
    "kelpie-dst/"
    # VM backends interact with real VMs
    "kelpie-vm/"
    "kelpie-sandbox/"
    # CLI tools run in production, not DST
    "kelpie-cli/"
    "kelpie-tools/"
    # Cluster coordination uses real time for heartbeats/gossip
    "kelpie-cluster/"
)

# Check if a file matches any exception pattern
is_exception() {
    local file="$1"

    # Normalize path (remove double slashes)
    file=$(echo "$file" | sed 's|//|/|g')

    for pattern in "${EXCEPTION_PATTERNS[@]}"; do
        if [[ "$file" == *"$pattern"* ]]; then
            return 0
        fi
    done

    # Allow test files
    if [[ "$file" == *"_test.rs" ]] || [[ "$file" == *"/tests/"* ]]; then
        return 0
    fi

    return 1
}

# Check if a line is a comment or documentation
is_comment_or_doc() {
    local line="$1"
    # Check if line is primarily a comment (starts with // or //! or /// or /* after whitespace)
    if echo "$line" | grep -qE '^\s*(//|/\*|\*|#\[)'; then
        return 0
    fi
    return 1
}

# Check if line is inside a #[cfg(test)] block by looking at surrounding context
# Note: This is a heuristic - we check if there's a #[cfg(test)] before this line
is_in_test_module() {
    local file="$1"
    local line_num="$2"

    # Check if there's a #[cfg(test)] before this line (within last 100 lines)
    # This is a simple heuristic that works for most Rust code
    local test_marker_count
    test_marker_count=$(head -n "$line_num" "$file" 2>/dev/null | tail -n 100 | grep -c '#\[cfg(test)\]' 2>/dev/null || true)

    # Handle empty output
    if [ -z "$test_marker_count" ]; then
        test_marker_count=0
    fi

    # Check if count > 0
    if [ "$test_marker_count" -gt 0 ] 2>/dev/null; then
        return 0
    fi
    return 1
}

violations_found=0
violation_count=0

echo "Scanning for non-deterministic patterns..."
echo ""

for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
    echo -n "Checking: $pattern ... "

    # Find all matches in src/ directories only
    matches=$(grep -rn "$pattern" crates/*/src/ --include="*.rs" 2>/dev/null || true)

    if [ -z "$matches" ]; then
        echo -e "${GREEN}OK${NC}"
        continue
    fi

    # Filter out exceptions and comments
    pattern_violations=""
    while IFS= read -r match; do
        if [ -z "$match" ]; then
            continue
        fi

        file=$(echo "$match" | cut -d: -f1)
        line_num=$(echo "$match" | cut -d: -f2)
        line_content=$(echo "$match" | cut -d: -f3-)

        # Skip if file is an exception
        if is_exception "$file"; then
            continue
        fi

        # Skip if line is a comment/doc
        if is_comment_or_doc "$line_content"; then
            continue
        fi

        # Skip if inside #[cfg(test)] block
        if is_in_test_module "$file" "$line_num"; then
            continue
        fi

        pattern_violations="$pattern_violations$match"$'\n'
        ((violation_count++)) || true
    done <<< "$matches"

    if [ -z "$(echo "$pattern_violations" | tr -d '[:space:]')" ]; then
        echo -e "${GREEN}OK${NC}"
    else
        echo -e "${RED}VIOLATION${NC}"
        violations_found=1
        echo "$pattern_violations" | while IFS= read -r line; do
            if [ -n "$line" ]; then
                echo -e "  ${YELLOW}→${NC} $line"
            fi
        done
    fi
done

echo ""
echo "=== Summary ==="

if [ $violations_found -eq 0 ]; then
    echo -e "${GREEN}✓ No violations found${NC}"
    echo ""
    echo "All time/random operations use injected providers (TimeProvider, RngProvider)."
    exit 0
else
    echo -e "${RED}✗ Found $violation_count violation(s)${NC}"
    echo ""
    echo "These patterns bypass DST determinism. Use injected providers instead:"
    echo ""
    echo -e "  ${BLUE}Instead of:                     Use:${NC}"
    echo "  ─────────────────────────────────────────────────────"
    echo "  tokio::time::sleep(dur)    →    time_provider.sleep_ms(ms)"
    echo "  std::thread::sleep(dur)    →    time_provider.sleep_ms(ms)"
    echo "  SystemTime::now()          →    time_provider.now_ms()"
    echo "  Instant::now()             →    time_provider.monotonic_ms()"
    echo "  rand::random()             →    rng_provider.next_u64()"
    echo "  thread_rng()               →    rng_provider"
    echo ""
    echo -e "${BLUE}Allowed exceptions (production code):${NC}"
    for exception in "${EXCEPTION_PATTERNS[@]}"; do
        echo "  - $exception"
    done
    echo "  - Test files (*_test.rs, tests/*.rs, #[cfg(test)] blocks)"
    echo ""
    echo "See: crates/kelpie-core/src/io.rs for TimeProvider/RngProvider traits"

    if [ "$STRICT_MODE" = true ]; then
        exit 1
    else
        echo ""
        echo -e "${YELLOW}Note: Running in --warn-only mode, not failing CI${NC}"
        exit 0
    fi
fi
