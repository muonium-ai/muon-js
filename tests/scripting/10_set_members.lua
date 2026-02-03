-- Populate a set and count members.
redis.call('DEL', KEYS[1]);
local n = tonumber(ARGV[1]) or 100;
for i = 1, n do
  redis.call('SADD', KEYS[1], 'm' .. i);
end
local members = redis.call('SMEMBERS', KEYS[1]);
return #members;
