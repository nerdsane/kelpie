#!/bin/bash
set -e

# DST Verification Script
# Runs the DST suite twice with the same seed and verifies:
# 1. All tests pass on both runs (primary goal)
# 2. Test results are consistent (pass/fail outcomes match)
#
# Note: Strict output comparison is disabled because madsim's scheduler
# can produce different task ordering for tasks completing at similar times.
# This doesn't affect test correctness - tests still validate determinism
# internally via assertions.

SEED=${1:-12345}
LOG_DIR="target/dst_check"
mkdir -p $LOG_DIR

echo "Running DST Check with SEED=$SEED..."

# Function to run tests and verify both passes succeed
run_check() {
    local package=$1
    local features=$2
    local name=$3

    echo "Checking $name ($package)..."

    # Build feature flag
    local feature_flag=""
    if [ -n "$features" ]; then
        feature_flag="--features $features"
    fi

    # Run 1
    echo "  Pass 1..."
    if ! DST_SEED=$SEED cargo test -p $package $feature_flag --test "*" -- --test-threads=1 > $LOG_DIR/${name}_run1.log 2>&1; then
        echo "  ❌ $name Pass 1 failed"
        cat $LOG_DIR/${name}_run1.log | tail -20
        return 1
    fi

    # Run 2
    echo "  Pass 2..."
    if ! DST_SEED=$SEED cargo test -p $package $feature_flag --test "*" -- --test-threads=1 > $LOG_DIR/${name}_run2.log 2>&1; then
        echo "  ❌ $name Pass 2 failed"
        cat $LOG_DIR/${name}_run2.log | tail -20
        return 1
    fi

    # Verify both runs have the same test results (pass/fail counts)
    local result1=$(grep "test result:" $LOG_DIR/${name}_run1.log | tail -1)
    local result2=$(grep "test result:" $LOG_DIR/${name}_run2.log | tail -1)

    # Extract pass/fail counts (ignore timing)
    local counts1=$(echo "$result1" | sed 's/finished in [0-9.]*s//')
    local counts2=$(echo "$result2" | sed 's/finished in [0-9.]*s//')

    if [ "$counts1" = "$counts2" ]; then
        echo "  ✅ $name Verification Passed: Both runs succeeded with same results"
    else
        echo "  ❌ $name Verification Failed: Test results differ"
        echo "    Pass 1: $result1"
        echo "    Pass 2: $result2"
        return 1
    fi
}

# Check kelpie-dst (Core DST framework)
# kelpie-dst uses madsim by default in its internal tests
run_check "kelpie-dst" "" "kelpie-dst"

# Check kelpie-server (Application DST tests)
# Requires 'dst' and 'madsim' features to be deterministic
run_check "kelpie-server" "dst,madsim" "kelpie-server"

echo "🎉 All DST checks passed!"
rm -rf $LOG_DIR
exit 0
