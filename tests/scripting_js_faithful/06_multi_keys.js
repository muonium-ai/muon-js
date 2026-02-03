// Faithful copy of Lua test 06_multi_keys.lua
// Expected array: one, two
redis.call('SET', KEYS[0], ARGV[0]);
redis.call('SET', KEYS[1], ARGV[1]);
return [redis.call('GET', KEYS[0]), redis.call('GET', KEYS[1])];
