// Faithful copy of Lua test 05_incrby.lua
// Expected: 15
redis.call('SET', KEYS[1], 10);
return redis.call('INCRBY', KEYS[1], ARGV[1]);
