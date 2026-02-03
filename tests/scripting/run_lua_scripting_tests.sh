#!/bin/bash
# Run Lua scripting tests against a running Redis server.
# Usage: REDIS_HOST=127.0.0.1 REDIS_PORT=6379 ./tests/scripting/run_lua_scripting_tests.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REDIS_HOST="${REDIS_HOST:-127.0.0.1}"
REDIS_PORT="${REDIS_PORT:-6379}"

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
    output=$(redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" --raw EVAL "$(cat "$file")" "$@")
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

echo "Running Redis Lua scripting tests"

echo "Test 1: Hello scripting"
run_test "hello" "$SCRIPT_DIR/01_hello.lua" "Hello, scripting!" 0

echo "Test 2: KEYS/ARGV mapping"
run_test "keys_argv" "$SCRIPT_DIR/02_keys_argv.lua" $'key1\nkey2\narg1\narg2\narg3' 2 key1 key2 arg1 arg2 arg3

echo "Test 3: redis.call SET/GET"
run_test "redis_call" "$SCRIPT_DIR/03_redis_call.lua" "bar" 1 test:lua:key bar

echo "Test 4: ARGV echo"
run_test "argv_echo" "$SCRIPT_DIR/04_argv_echo.lua" "Hello" 0 Hello

echo "Test 5: INCRBY"
run_test "incrby" "$SCRIPT_DIR/05_incrby.lua" "15" 1 test:lua:counter 5

echo "Test 6: Multiple KEYS"
run_test "multi_keys" "$SCRIPT_DIR/06_multi_keys.lua" $'one\ntwo' 2 test:lua:k1 test:lua:k2 one two

echo ""
echo "Results: $PASS/$TOTAL passed, $FAIL failed"
if [ $FAIL -ne 0 ]; then
    exit 1
fi
