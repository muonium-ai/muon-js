# JS scripting tests (mini-redis)

These tests validate the mini-redis JavaScript scripting engine via `EVAL`.

## Tests
- 01_hello.js: basic EVAL return value
- 02_keys_argv.js: KEYS/ARGV parameterization
- 03_redis_call.js: redis.call with KEYS/ARGV
- 04_argv_echo.js: ARGV echo parameterization
- 05_incrby.js: redis.call with numeric return
- 06_multi_keys.js: multiple KEYS and array return

## Run
Start mini-redis and run:

```
MINI_REDIS_HOST=127.0.0.1 MINI_REDIS_PORT=6379 ./tests/scripting_js/run_js_scripting_tests.sh
```
