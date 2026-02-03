# Lua scripting tests (Redis)

These tests validate basic Lua scripting semantics directly against Redis, using examples from the Redis Lua scripting introduction.

## Tests
- 01_hello.lua: basic EVAL return value
- 02_keys_argv.lua: KEYS/ARGV parameterization
- 03_redis_call.lua: redis.call with KEYS/ARGV
- 04_argv_echo.lua: ARGV echo parameterization
- 05_incrby.lua: redis.call with numeric return
- 06_multi_keys.lua: multiple KEYS and array return

## Run
Ensure a Redis server is running, then:

```
REDIS_HOST=127.0.0.1 REDIS_PORT=6379 ./tests/scripting/run_lua_scripting_tests.sh
```

The runner uses `redis-cli` and compares outputs with expected results.
