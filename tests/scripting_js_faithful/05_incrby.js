// Faithful copy of Lua test 05_incrby.lua
// Expected: 15
redis.call('SET', KEYS[0], 10);
return redis.call('INCRBY', KEYS[0], ARGV[0]);
