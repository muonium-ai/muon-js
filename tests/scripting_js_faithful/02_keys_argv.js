// Faithful copy of Lua test 02_keys_argv.lua
// Expected array: key1, key2, arg1, arg2, arg3
// 1-based indexing to match Redis/Lua convention
return [KEYS[1], KEYS[2], ARGV[1], ARGV[2], ARGV[3]];
