-- Populate a hash and sum the values.
redis.call('DEL', KEYS[1]);
local n = tonumber(ARGV[1]) or 100;
for i = 1, n do
  redis.call('HSET', KEYS[1], 'f' .. i, i);
end
local sum = 0;
for i = 1, n do
  local v = redis.call('HGET', KEYS[1], 'f' .. i);
  sum = sum + tonumber(v);
end
return sum;
