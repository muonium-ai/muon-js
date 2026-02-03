#!/bin/bash
# Run faithful JS scripting tests against mini-redis (EVAL executes JS).
# These mirror the Lua tests exactly and may fail until JS array returns work.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MINI_REDIS_HOST="${MINI_REDIS_HOST:-127.0.0.1}"
MINI_REDIS_PORT="${MINI_REDIS_PORT:-6379}"

PASS=0
FAIL=0
TOTAL=0

run_test() {
    local name="$1"
    local file="$2"
    shift 2
    local expected="$1"
    shift 1
    local output

    TOTAL=$((TOTAL + 1))
    output=$(redis-cli -h "$MINI_REDIS_HOST" -p "$MINI_REDIS_PORT" --raw EVAL "$(cat "$file")" "$@")
    if [ "$output" = "$expected" ]; then
        echo "✓ $name"
        PASS=$((PASS + 1))
    else
        echo "✗ $name"
        echo "  Expected: $expected"
        echo "  Got: $output"
        FAIL=$((FAIL + 1))
    fi
}

echo "Running mini-redis JS scripting tests (faithful)"

echo "Test 1: Hello scripting"
run_test "hello" "$SCRIPT_DIR/01_hello.js" "Hello, scripting!" 0

echo "Test 2: KEYS/ARGV mapping"
run_test "keys_argv" "$SCRIPT_DIR/02_keys_argv.js" $'key1\nkey2\narg1\narg2\narg3' 2 key1 key2 arg1 arg2 arg3

echo "Test 3: redis.call SET/GET"
run_test "redis_call" "$SCRIPT_DIR/03_redis_call.js" "bar" 1 test:js:key bar

echo "Test 4: ARGV echo"
run_test "argv_echo" "$SCRIPT_DIR/04_argv_echo.js" "Hello" 0 Hello

echo "Test 5: INCRBY"
run_test "incrby" "$SCRIPT_DIR/05_incrby.js" "15" 1 test:js:counter 5

echo "Test 6: Multiple KEYS"
run_test "multi_keys" "$SCRIPT_DIR/06_multi_keys.js" $'one\ntwo' 2 test:js:k1 test:js:k2 one two

echo "Test 7: KEYS/ARGV lengths"
run_test "lengths" "$SCRIPT_DIR/07_lengths.js" "2|3" 2 key1 key2 arg1 arg2 arg3

echo "Test 8: LRANGE"
run_test "lrange" "$SCRIPT_DIR/08_lrange.js" $'b\na' 1 test:js:list a b

echo "Test 9: Hash sum"
run_test "hash_sum" "$SCRIPT_DIR/09_hash_sum.js" "5050" 1 test:js:hash 100

echo "Test 10: Set members"
run_test "set_members" "$SCRIPT_DIR/10_set_members.js" "100" 1 test:js:set 100

echo "Test 11: Bulk INCRBY"
run_test "bulk_incr" "$SCRIPT_DIR/11_bulk_incr.js" "100" 1 test:js:bulk 100

echo ""
echo "Results: $PASS/$TOTAL passed, $FAIL failed"
if [ $FAIL -ne 0 ]; then
    exit 1
fi
