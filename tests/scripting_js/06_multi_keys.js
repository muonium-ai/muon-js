// Set two keys and return the second value
// Expected: two
redis.call('SET', KEYS[0], ARGV[0]);
redis.call('SET', KEYS[1], ARGV[1]);
return redis.call('GET', KEYS[1]);
