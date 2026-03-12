// Faithful copy of Lua test 09_hash_sum.lua
// Expected: sum of 1..n
redis.call('DEL', KEYS[1]);
var n = Number(ARGV[1] || 100);
for (var i = 1; i <= n; i += 1) {
  redis.call('HSET', KEYS[1], 'f' + i, i);
}
var sum = 0;
for (var j = 1; j <= n; j += 1) {
  var v = redis.call('HGET', KEYS[1], 'f' + j);
  sum += Number(v);
}
return sum;
