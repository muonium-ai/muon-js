// Faithful copy of Lua test 08_lrange.lua
// Expected array: b, a (LPUSH inserts left to right)
redis.call('DEL', KEYS[0]);
redis.call('LPUSH', KEYS[0], ARGV[0], ARGV[1]);
return redis.call('LRANGE', KEYS[0], 0, -1);
