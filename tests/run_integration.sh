#!/bin/bash

# Integration test runner for MuonJS
# Tests complex real-world JavaScript scenarios

PASS=0
FAIL=0
TOTAL=0

echo "Running MuonJS Integration Tests"
echo "=================================="
echo ""

for test_file in tests/integration/*.js; do
    if [ -f "$test_file" ]; then
        TOTAL=$((TOTAL + 1))
        test_name=$(basename "$test_file" .js)
        
        echo -n "Testing $test_name ... "
        
        # Run the test and capture output
        output=$(cargo run --release --example eval -- "$test_file" 2>&1)
        exit_code=$?
        
        # Check if it ran without exception
        if [ $exit_code -eq 0 ] && [[ ! "$output" =~ "Exception" ]]; then
            echo "✓ PASS (result: $output)"
            PASS=$((PASS + 1))
        else
            echo "✗ FAIL"
            echo "  Output: $output"
            FAIL=$((FAIL + 1))
        fi
        echo ""
    fi
done

echo "=================================="
echo "Results: $PASS/$TOTAL passed, $FAIL/$TOTAL failed"
echo ""

if [ $FAIL -eq 0 ]; then
    exit 0
else
    exit 1
fi
