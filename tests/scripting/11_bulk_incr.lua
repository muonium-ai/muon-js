-- Increment a counter in a tight loop.
redis.call('SET', KEYS[1], 0);
local n = tonumber(ARGV[1]) or 100;
for i = 1, n do
  redis.call('INCRBY', KEYS[1], 1);
end
return redis.call('GET', KEYS[1]);
