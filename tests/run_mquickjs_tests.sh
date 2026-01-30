#!/bin/bash
# Test muon-js compatibility with official mquickjs test suite

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TESTS_DIR="$SCRIPT_DIR/mquickjs"
EVAL_BIN="$PROJECT_ROOT/target/release/examples/eval"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}mquickjs Compatibility Test Suite${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Build in release mode for accurate testing
echo -e "${YELLOW}Building muon-js in release mode...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --example eval 2>&1 | grep -E "(Compiling|Finished)" || true
echo ""

if [ ! -f "$EVAL_BIN" ]; then
    echo -e "${RED}Error: eval binary not found at $EVAL_BIN${NC}"
    exit 1
fi

# Test files to run
TEST_FILES=(
    "test_language.js"
    "test_builtin.js"
    "test_loop.js"
    "test_closure.js"
    "mandelbrot.js"
    "test_rect.js"
)

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
EXCEPTION_TESTS=0

# Function to run a single test
run_test() {
    local test_file="$1"
    local test_path="$TESTS_DIR/$test_file"
    
    if [ ! -f "$test_path" ]; then
        echo -e "${RED}  ✗ Test file not found: $test_file${NC}"
        return 1
    fi
    
    echo -e "${BLUE}Testing: $test_file${NC}"
    
    # Run the test and capture output
    local output
    local exit_code
    
    output=$("$EVAL_BIN" "$test_path" 2>&1) || exit_code=$?
    
    # Check result
    if [ -z "$exit_code" ] || [ "$exit_code" -eq 0 ]; then
        # Check if output contains "Exception"
        if echo "$output" | grep -q "Exception"; then
            echo -e "${RED}  ✗ EXCEPTION${NC}"
            echo "    Output: $output"
            EXCEPTION_TESTS=$((EXCEPTION_TESTS + 1))
            return 1
        else
            echo -e "${GREEN}  ✓ PASS${NC}"
            if [ -n "$output" ]; then
                echo "    Output: $output"
            fi
            PASSED_TESTS=$((PASSED_TESTS + 1))
            return 0
        fi
    else
        echo -e "${RED}  ✗ FAIL (exit code: $exit_code)${NC}"
        echo "    Output: $output"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        return 1
    fi
}

# Run all tests
echo -e "${YELLOW}Running mquickjs compatibility tests...${NC}"
echo ""

for test_file in "${TEST_FILES[@]}"; do
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    run_test "$test_file"
    echo ""
done

# Summary
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Test Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "Total tests:      $TOTAL_TESTS"
echo -e "${GREEN}Passed:           $PASSED_TESTS${NC}"
echo -e "${RED}Failed:           $FAILED_TESTS${NC}"
echo -e "${RED}Exceptions:       $EXCEPTION_TESTS${NC}"

if [ $TOTAL_TESTS -gt 0 ]; then
    PASS_RATE=$((PASSED_TESTS * 100 / TOTAL_TESTS))
    echo -e "Pass rate:        ${PASS_RATE}%"
fi

echo ""

if [ $PASSED_TESTS -eq $TOTAL_TESTS ]; then
    echo -e "${GREEN}All tests passed! 🎉${NC}"
    exit 0
else
    echo -e "${YELLOW}Some tests failed. See output above for details.${NC}"
    exit 1
fi
