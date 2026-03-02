#!/bin/bash

set -e  # Exit on error

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

TRACE_DIR="traces"
mkdir -p "$TRACE_DIR"

# Function to run a test
run_test() {
    local bin_name=$1
    local module_path=$2
    if [ -z "$3" ]; then
        local core_path=""
    else
        local core_path="-f $module_path"
    fi
    local is_core=${3:-0}
    local trace_file="$TRACE_DIR/${bin_name}.trace"

    echo "Testing: $bin_name with module $module_path"

    # Run the binary to create trace
    echo "  Recording trace..."
    if RUST_LOG=info cargo run --bin "$bin_name" -- -c "$trace_file" -v $core_path; then
        echo -e "${GREEN}  ✓ Recording successful${NC}"
    else
        echo -e "${RED}  ✗ Recording failed${NC}"
        return 1
    fi

    # Replay the trace
    echo "  Replaying trace..."
    if RUST_LOG=info cargo run --bin replay -- -c "$trace_file" -v -f "$module_path"; then
        echo -e "${GREEN}  ✓ Replay successful${NC}"
    else
        echo -e "${RED}  ✗ Replay failed${NC}"
        return 1
    fi

    echo ""
}

# Run a test that is expected to fail (recording should produce a non-zero exit)
run_test_expect_fail() {
    local bin_name=$1
    local module_path=$2
    local trace_file="$TRACE_DIR/${bin_name}.trace"

    echo "Testing (expect fail): $bin_name with module $module_path"

    echo "  Recording trace (expecting failure)..."
    if RUST_LOG=info cargo run --bin "$bin_name" -- -c "$trace_file" -v 2>&1; then
        echo -e "${RED}  ✗ Expected failure but succeeded${NC}"
        return 1
    else
        echo -e "${GREEN}  ✓ Failed as expected${NC}"
    fi

    echo ""
}

# Component model tests
echo "=== Component Model Tests ==="
run_test "multi_return" "test-modules/components/multi_return.wasm"
run_test "record_small" "test-modules/components/record_small.wasm"
run_test "record_large" "test-modules/components/record_large.wasm"
run_test "variant_large" "test-modules/components/variant_large.wasm"
run_test "potpourri" "test-modules/components/potpourri.wasm"
run_test "complex_params" "test-modules/components/complex_params.wasm"
run_test "max_flat" "test-modules/components/max_flat.wasm"
run_test "over_max_flat" "test-modules/components/over_max_flat.wasm"
run_test "resource-1" "test-modules/components/resource-1.wasm"
run_test "resource-2" "test-modules/components/resource-2.wasm"
run_test "resource_drop" "test-modules/components/resource_drop.wasm"
run_test_expect_fail "resource-3" "test-modules/components/resource-3.wasm"

# Core module tests
echo "=== Core Module Tests ==="
run_test "core" "test-modules/core-plain/complex.wat" 1

echo -e "${GREEN}All tests passed!${NC}"
