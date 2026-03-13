# JS scripting tests (muoncache)

These tests validate the muoncache JavaScript scripting engine via `EVAL`.

## Tests
- 01_hello.js: basic EVAL return value
- 02_keys_argv.js: KEYS/ARGV parameterization
- 03_redis_call.js: redis.call with KEYS/ARGV
- 04_argv_echo.js: ARGV echo parameterization
- 05_incrby.js: redis.call with numeric return
- 06_multi_keys.js: multiple KEYS and array return

## Run
Start muoncache and run:

```
MUON_CACHE_HOST=127.0.0.1 MUON_CACHE_PORT=6379 ./tests/scripting_js/run_js_scripting_tests.sh
```
