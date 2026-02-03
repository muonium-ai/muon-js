// Faithful copy of Lua test 10_set_members.lua
// Expected: n
redis.call('DEL', KEYS[0]);
var n = Number(ARGV[0] || 100);
for (var i = 1; i <= n; i += 1) {
  redis.call('SADD', KEYS[0], 'm' + i);
}
var members = redis.call('SMEMBERS', KEYS[0]);
return members.length;
