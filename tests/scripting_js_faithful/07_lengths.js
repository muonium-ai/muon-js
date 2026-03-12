// Faithful copy of Lua test 07_lengths.lua
// Expected: "2|3"
// 1-based indexing: array length is n+1, subtract 1 to match Lua #KEYS/#ARGV
return String(KEYS.length - 1) + "|" + String(ARGV.length - 1);
