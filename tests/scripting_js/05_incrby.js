// Use redis.call to modify a key and return integer result
// Expected: 15
redis.call('SET', KEYS[1], 10);
return redis.call('INCRBY', KEYS[1], ARGV[1]);
