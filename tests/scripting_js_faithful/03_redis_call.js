// Faithful copy of Lua test 03_redis_call.lua
// Expected: value written to KEYS[1]
redis.call('SET', KEYS[1], ARGV[1]);
return redis.call('GET', KEYS[1]);
