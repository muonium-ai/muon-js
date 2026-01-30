#!/bin/bash
# Detailed mquickjs compatibility test runner
# This script attempts to run individual test functions from the mquickjs test suite

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TESTS_DIR="$SCRIPT_DIR/mquickjs"
EVAL_BIN="$PROJECT_ROOT/target/release/examples/eval"
TMP_DIR="$PROJECT_ROOT/tmp/mquickjs_tests"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}mquickjs Detailed Compatibility Report${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# Build in release mode
echo -e "${YELLOW}Building muon-js...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --example eval 2>&1 | grep -E "(Compiling|Finished)" || true
echo ""

if [ ! -f "$EVAL_BIN" ]; then
    echo -e "${RED}Error: eval binary not found${NC}"
    exit 1
fi

# Create temp directory for test files
mkdir -p "$TMP_DIR"

# Function to extract and run individual test functions
extract_and_run_test() {
    local test_file="$1"
    local test_name="$2"
    local test_path="$TESTS_DIR/$test_file"
    
    if [ ! -f "$test_path" ]; then
        echo -e "${RED}  ✗ File not found: $test_file${NC}"
        return 1
    fi
    
    # Create a test runner that calls the specific test function
    local tmp_test="$TMP_DIR/${test_name}.js"
    
    # Copy helper functions and the test function, then call it
    {
        # Extract everything before the test functions
        sed -n '1,/^function test_/p' "$test_path" | head -n -1
        # Extract the specific test function
        sed -n "/^function ${test_name}(/,/^}/p" "$test_path"
        # Call the test function
        echo ""
        echo "${test_name}();"
    } > "$tmp_test"
    
    # Run the test
    local output
    local exit_code=0
    
    output=$("$EVAL_BIN" "$tmp_test" 2>&1) || exit_code=$?
    
    if [ $exit_code -eq 0 ] && ! echo "$output" | grep -q "Exception"; then
        echo -e "${GREEN}  ✓ ${test_name}${NC}"
        return 0
    else
        echo -e "${RED}  ✗ ${test_name}${NC}"
        if [ -n "$output" ]; then
            echo "      ${output}" | head -n 1
        fi
        return 1
    fi
}

# Simple file test (no function extraction)
simple_test() {
    local test_file="$1"
    local test_path="$TESTS_DIR/$test_file"
    
    echo -e "${CYAN}Testing: $test_file${NC}"
    
    if [ ! -f "$test_path" ]; then
        echo -e "${RED}  ✗ File not found${NC}"
        return 1
    fi
    
    local output
    local exit_code=0
    
    output=$("$EVAL_BIN" "$test_path" 2>&1) || exit_code=$?
    
    if [ $exit_code -eq 0 ] && ! echo "$output" | grep -q "Exception"; then
        echo -e "${GREEN}  ✓ PASS${NC}"
        return 0
    else
        echo -e "${RED}  ✗ FAIL${NC}"
        echo "    ${output}" | head -n 3
        return 1
    fi
}

TOTAL=0
PASSED=0

# Test language features
echo -e "${CYAN}=== test_language.js ===${NC}"
for test in test_op1 test_cvt test_eq test_op2 test_string_cmp test_var test_side_effect \
            test_member_get_nothrow test_call_nothrow; do
    TOTAL=$((TOTAL + 1))
    if extract_and_run_test "test_language.js" "$test" 2>/dev/null; then
        PASSED=$((PASSED + 1))
    fi
done
echo ""

# Test builtin features
echo -e "${CYAN}=== test_builtin.js ===${NC}"
for test in test_function test_enum test_array test_array_ext test_string test_string2 \
            test_math test_number test_global_eval test_typed_array test_json test_regexp; do
    TOTAL=$((TOTAL + 1))
    if extract_and_run_test "test_builtin.js" "$test" 2>/dev/null; then
        PASSED=$((PASSED + 1))
    fi
done
echo ""

# Test loops and control flow
echo -e "${CYAN}=== test_loop.js ===${NC}"
for test in test_while test_while_break test_do_while test_for test_for_in test_for_break \
            test_switch1 test_switch2 test_try_catch1 test_try_catch2 test_try_catch3 \
            test_try_catch4 test_try_catch5 test_try_catch6 test_try_catch7 test_try_catch8; do
    TOTAL=$((TOTAL + 1))
    if extract_and_run_test "test_loop.js" "$test" 2>/dev/null; then
        PASSED=$((PASSED + 1))
    fi
done
echo ""

# Test closures
echo -e "${CYAN}=== test_closure.js ===${NC}"
for test in test_closure1 test_closure2 test_closure3; do
    TOTAL=$((TOTAL + 1))
    if extract_and_run_test "test_closure.js" "$test" 2>/dev/null; then
        PASSED=$((PASSED + 1))
    fi
done
echo ""

# Test simple programs
echo -e "${CYAN}=== Simple Programs ===${NC}"
for file in mandelbrot.js test_rect.js; do
    TOTAL=$((TOTAL + 1))
    if simple_test "$file"; then
        PASSED=$((PASSED + 1))
    fi
done
echo ""

# Summary
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Compatibility Summary${NC}"
echo -e "${BLUE}========================================${NC}"
FAILED=$((TOTAL - PASSED))
PASS_RATE=0
if [ $TOTAL -gt 0 ]; then
    PASS_RATE=$((PASSED * 100 / TOTAL))
fi

echo -e "Total tests:      $TOTAL"
echo -e "${GREEN}Passed:           $PASSED (${PASS_RATE}%)${NC}"
echo -e "${RED}Failed:           $FAILED${NC}"
echo ""

if [ $PASS_RATE -ge 80 ]; then
    echo -e "${GREEN}Excellent compatibility! 🎉${NC}"
elif [ $PASS_RATE -ge 50 ]; then
    echo -e "${YELLOW}Good progress, but more work needed.${NC}"
elif [ $PASS_RATE -ge 20 ]; then
    echo -e "${YELLOW}Basic compatibility established.${NC}"
else
    echo -e "${RED}Significant work needed for mquickjs compatibility.${NC}"
fi

# Cleanup
rm -rf "$TMP_DIR"

exit 0
