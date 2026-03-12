// Verify KEYS/ARGV parameterization
// Expected: key1|key2|arg1|arg2|arg3
// 1-based indexing to match Redis/Lua convention
return KEYS[1] + "|" + KEYS[2] + "|" + ARGV[1] + "|" + ARGV[2] + "|" + ARGV[3];
