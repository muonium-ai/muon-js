-- Set two keys and return both values
-- Expected array: one, two
redis.call('SET', KEYS[1], ARGV[1])
redis.call('SET', KEYS[2], ARGV[2])
return { redis.call('GET', KEYS[1]), redis.call('GET', KEYS[2]) }
