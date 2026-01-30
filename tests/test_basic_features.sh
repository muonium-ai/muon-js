#!/bin/bash
# Simple feature test - tests basic features without assertion framework

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
EVAL_BIN="$PROJECT_ROOT/target/release/examples/eval"
TMP_DIR="$PROJECT_ROOT/tmp/feature_tests"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Basic Feature Tests (mquickjs subset)${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

mkdir -p "$TMP_DIR"

TOTAL=0
PASSED=0

# Test helper
test_code() {
    local name="$1"
    local code="$2"
    local expected="$3"
    
    TOTAL=$((TOTAL + 1))
    
    echo "$code" > "$TMP_DIR/test.js"
    local output
    output=$("$EVAL_BIN" "$TMP_DIR/test.js" 2>&1 || true)
    
    if [ "$output" = "$expected" ]; then
        echo -e "${GREEN}✓${NC} $name"
        PASSED=$((PASSED + 1))
        return 0
    else
        echo -e "${RED}✗${NC} $name (got: $output, expected: $expected)"
        return 1
    fi
}

echo -e "${YELLOW}=== Arithmetic Operators ===${NC}"
test_code "Addition" "1 + 2" "3"
test_code "Subtraction" "5 - 3" "2"
test_code "Multiplication" "2 * 3" "6"
test_code "Division" "10 / 2" "5"
test_code "Modulo" "7 % 3" "1"
test_code "Exponentiation" "2 ** 8" "256"
test_code "Negative" "-5" "-5"
test_code "Unary plus" "+3" "3"
echo ""

echo -e "${YELLOW}=== Comparison Operators ===${NC}"
test_code "Less than" "1 < 2" "true"
test_code "Greater than" "2 > 1" "true"
test_code "Less or equal" "1 <= 1" "true"
test_code "Greater or equal" "2 >= 2" "true"
test_code "Equal" "5 == 5" "true"
test_code "Not equal" "5 != 3" "true"
test_code "Strict equal" "5 === 5" "true"
test_code "Strict not equal" "5 !== 3" "true"
echo ""

echo -e "${YELLOW}=== Logical Operators ===${NC}"
test_code "AND true" "true && true" "true"
test_code "AND false" "true && false" "false"
test_code "OR true" "true || false" "true"
test_code "OR false" "false || false" "false"
test_code "NOT true" "!false" "true"
test_code "NOT false" "!true" "false"
echo ""

echo -e "${YELLOW}=== Variables ===${NC}"
test_code "Variable assignment" "x = 5; x" "5"
test_code "Variable arithmetic" "a = 2; b = 3; a + b" "5"
echo ""

echo -e "${YELLOW}=== Strings ===${NC}"
test_code "String concat" '"hello" + " " + "world"' "hello world"
test_code "String charAt" '"hello".charAt(0)' "h"
test_code "String substring" '"hello".substring(1, 4)' "ell"
test_code "String indexOf" '"hello".indexOf("ll")' "2"
test_code "String length" '"hello".length' "5"
test_code "String toUpperCase" '"hello".toUpperCase()' "HELLO"
test_code "String toLowerCase" '"HELLO".toLowerCase()' "hello"
echo ""

echo -e "${YELLOW}=== Arrays ===${NC}"
test_code "Array literal" "a = [1, 2, 3]; a[1]" "2"
test_code "Array length" "[1, 2, 3].length" "3"
test_code "Array assignment" "a = [1, 2]; a[2] = 3; a[2]" "3"
echo ""

echo -e "${YELLOW}=== Objects ===${NC}"
test_code "Object property" 'o = {x: 5}; o.x' "5"
test_code "Object bracket access" 'o = {x: 5}; o["x"]' "5"
echo ""

echo -e "${YELLOW}=== Functions ===${NC}"
test_code "Function call" "function add(a, b) { return a + b } add(2, 3)" "5"
test_code "Function return" "function five() { return 5 } five()" "5"
echo ""

echo -e "${YELLOW}=== Control Flow ===${NC}"
test_code "If true" "if (true) { 5 } else { 3 }" "5"
test_code "If false" "if (false) { 5 } else { 3 }" "3"
test_code "While loop" "i = 0; sum = 0; while (i < 3) { sum = sum + i; i = i + 1 } sum" "3"
test_code "For loop" "sum = 0; for (i = 0; i < 3; i = i + 1) { sum = sum + i } sum" "3"
test_code "Break" "i = 0; while (true) { if (i == 3) { break } i = i + 1 } i" "3"
test_code "Continue" "i = 0; sum = 0; while (i < 5) { i = i + 1; if (i == 3) { continue } sum = sum + i } sum" "12"
echo ""

echo -e "${YELLOW}=== Math ===${NC}"
test_code "Math.abs" "Math.abs(-5)" "5"
test_code "Math.floor" "Math.floor(3.7)" "3"
test_code "Math.ceil" "Math.ceil(3.2)" "4"
test_code "Math.round" "Math.round(3.5)" "4"
test_code "Math.sqrt" "Math.sqrt(16)" "4"
test_code "Math.max" "Math.max(1, 5, 3)" "5"
test_code "Math.min" "Math.min(1, 5, 3)" "1"
echo ""

# Summary
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

if [ $PASS_RATE -ge 90 ]; then
    echo -e "${GREEN}Excellent! Core features working well. 🎉${NC}"
elif [ $PASS_RATE -ge 70 ]; then
    echo -e "${YELLOW}Good progress on core features.${NC}"
else
    echo -e "${RED}Many core features still need work.${NC}"
fi

rm -rf "$TMP_DIR"

exit 0
