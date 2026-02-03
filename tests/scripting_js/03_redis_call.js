// Verify redis.call integration with KEYS/ARGV
// Expected: value written to KEYS[0]
redis.call('SET', KEYS[0], ARGV[0]);
return redis.call('GET', KEYS[0]);
