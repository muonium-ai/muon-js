-- List write + LRANGE return
-- Expected array: b, a (LPUSH inserts left to right)
redis.call('DEL', KEYS[1])
redis.call('LPUSH', KEYS[1], ARGV[1], ARGV[2])
return redis.call('LRANGE', KEYS[1], 0, -1)
