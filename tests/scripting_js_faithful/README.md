# JS scripting tests (faithful Lua copies)

These tests mirror the Lua scripts exactly, including array return values. They are expected to fail until JS array return conversion is implemented in mini-redis.

## Tests
- 01_hello.js
- 02_keys_argv.js
- 03_redis_call.js
- 04_argv_echo.js
- 05_incrby.js
- 06_multi_keys.js
- 07_lengths.js
- 08_lrange.js

## Run
Start mini-redis and run:

```
MINI_REDIS_HOST=127.0.0.1 MINI_REDIS_PORT=6379 ./tests/scripting_js_faithful/run_js_scripting_tests.sh
```
