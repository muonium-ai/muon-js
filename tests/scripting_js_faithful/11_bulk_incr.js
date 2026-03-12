// Faithful copy of Lua test 11_bulk_incr.lua
// Expected: n
redis.call('SET', KEYS[1], 0);
var n = Number(ARGV[1] || 100);
for (var i = 0; i < n; i += 1) {
  redis.call('INCRBY', KEYS[1], 1);
}
return redis.call('GET', KEYS[1]);
