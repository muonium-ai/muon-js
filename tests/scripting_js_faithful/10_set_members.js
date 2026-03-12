// Faithful copy of Lua test 10_set_members.lua
// Expected: n
redis.call('DEL', KEYS[1]);
var n = Number(ARGV[1] || 100);
for (var i = 1; i <= n; i += 1) {
  redis.call('SADD', KEYS[1], 'm' + i);
}
var members = redis.call('SMEMBERS', KEYS[1]);
return members.length;
