// Use redis.call to modify a key and return integer result
// Expected: 15
redis.call('SET', KEYS[0], 10);
return redis.call('INCRBY', KEYS[0], ARGV[0]);
