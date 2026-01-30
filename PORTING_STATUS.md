# Porting Status: mquickjs → muon-js

## Overview

**Current Progress: ~15% core features ported**

Muon-js is a native Rust port of [MicroQuickJS](https://github.com/bellard/mquickjs), a tiny JavaScript engine designed for embedded systems. This document tracks what has been ported and what remains.

### mquickjs Reference
- **Upstream**: https://github.com/bellard/mquickjs (submodule at `vendor/mquickjs`)
- **Official tests**: 45 test functions across ~3,000 lines
  - `test_language.js` (355 lines) - operators, control flow
  - `test_builtin.js` (875 lines) - String, Array, Math, Number, JSON, RegExp
  - `test_loop.js` (395 lines) - loops, switch, try/catch
  - `test_closure.js` (106 lines) - closure semantics
  - Plus: mandelbrot, microbench, test_rect

---

## ✅ What's Been Ported

### Core Engine (Partial)
- ✅ **Context creation** (`JS_NewContext` equivalent)
- ✅ **Value representation** (32-bit tagged values for int32/ptr)
- ✅ **Basic evaluation** (`JS_Eval` equivalent)
- ✅ **Marker-based method dispatch** (strings/arrays/objects)

### JavaScript Language Features
- ✅ **Variables**: `var`, assignment, scoping
- ✅ **Data types**: Numbers (int32/float), strings, booleans, undefined, null, arrays, objects
- ✅ **Operators**: 
  - Arithmetic: `+`, `-`, `*`, `/`, `%`, `**` (exponentiation)
  - Comparison: `<`, `>`, `<=`, `>=`, `==`, `!=`, `===`, `!==`
  - Logical: `&&`, `||`, `!`
  - Unary: `++`, `--`, `-`, `+`
- ✅ **Control flow**:
  - `if`/`else` statements
  - `while` loops
  - `for` loops
  - `break`/`continue`
  - Early `return` from functions
- ✅ **Functions**: Declaration, invocation, parameters, return values
- ✅ **Comments**: `//` single-line comments

### Built-in Objects & Methods

#### String Methods
- ✅ `charAt(index)`
- ✅ `substring(start)` / `substring(start, end)`
- ✅ `indexOf(substring)`
- ✅ `split(separator)`
- ✅ `toUpperCase()`
- ✅ `toLowerCase()`
- ✅ `trim()`
- ✅ `length` property

#### Array Methods
- ✅ `push(element)`
- ✅ `pop()`
- ✅ `length` property
- ✅ Array indexing: `arr[i]`
- ✅ `for` iteration over arrays

#### Object Methods
- ✅ Object creation: `{key: value}`
- ✅ Property access: `obj.key` and `obj[key]`
- ✅ Property assignment

#### Math Object
- ✅ `Math.abs(x)`
- ✅ `Math.sqrt(x)`
- ✅ `Math.floor(x)`
- ✅ `Math.ceil(x)`
- ✅ `Math.round(x)`
- ✅ `Math.max(a, b, ...)`
- ✅ `Math.min(a, b, ...)`

---

## ❌ What's Missing

### Core Engine Architecture
- ❌ **Tracing & compacting GC** (mquickjs's key innovation)
- ❌ **JSGCRef system** (`JS_PushGCRef` / `JS_PopGCRef`)
- ❌ **Bytecode compiler & VM** (stack-based bytecode execution)
- ❌ **Bytecode persistence** (`JS_SaveBytecode` / `JS_LoadBytecode` / `JS_RelocateBytecode`)
- ❌ **Atom table** (string interning for property keys)
- ❌ **Memory buffer allocation** (fixed-size buffer for embedded systems)
- ❌ **Standard library ROM generation** (`mquickjs_build.c` tool)

### JavaScript Language Features
- ❌ **Strict mode enforcement** (mquickjs is always strict)
- ❌ **Typed arrays** (Int8Array, Uint8Array, Float32Array, etc.)
- ❌ **Regular expressions** (limited in mquickjs: `/pattern/flags`)
- ❌ **Error handling**: `try`/`catch`/`finally`/`throw`
- ❌ **Object constructors**: `new Constructor()`
- ❌ **Prototypes & inheritance**: `prototype`, `__proto__`
- ❌ **`this` keyword** (not functional yet)
- ❌ **Arrow functions**: `() => {}`
- ❌ **Template literals**: `` `string ${expr}` ``
- ❌ **Destructuring**: `[a, b] = arr`, `{x, y} = obj`
- ❌ **Spread operator**: `...arr`
- ❌ **Rest parameters**: `function(...args)`
- ❌ **Default parameters**: `function(a = 1)`
- ❌ **`for...in` loops** (object property iteration)
- ❌ **`for...of` loops** (array iteration - mquickjs supports this)
- ❌ **Block-scoped variables**: `let`, `const`
- ❌ **Switch statements**
- ❌ **Ternary operator**: `condition ? a : b`
- ❌ **Comma operator**
- ❌ **Bitwise operators**: `&`, `|`, `^`, `~`, `<<`, `>>`, `>>>`

### Built-in Objects & Methods

#### String Methods (Missing)
- ❌ `charCodeAt(index)`
- ❌ `codePointAt(index)` (ES6)
- ❌ `slice(start, end)`
- ❌ `substr(start, length)` (deprecated)
- ❌ `concat(...strings)`
- ❌ `replace(search, replacement)`
- ❌ `replaceAll(search, replacement)` (ES2021)
- ❌ `match(regex)`
- ❌ `search(regex)`
- ❌ `startsWith(prefix)`
- ❌ `endsWith(suffix)`
- ❌ `includes(substring)`
- ❌ `repeat(count)` (added in latest mquickjs)
- ❌ `padStart(length, fill)`
- ❌ `padEnd(length, fill)`
- ❌ `trimStart()` / `trimEnd()`
- ❌ **Method chaining** (e.g., `str.charAt(0).toUpperCase()`)

#### Array Methods (Missing)
- ❌ `shift()` / `unshift()`
- ❌ `slice(start, end)`
- ❌ `splice(start, deleteCount, ...items)`
- ❌ `concat(...arrays)`
- ❌ `join(separator)` ⚠️ *Critical: Blocks integration test 06*
- ❌ `reverse()`
- ❌ `sort(compareFn)`
- ❌ `indexOf(element)`
- ❌ `lastIndexOf(element)`
- ❌ `includes(element)`
- ❌ `forEach(callback)`
- ❌ `map(callback)`
- ❌ `filter(callback)`
- ❌ `reduce(callback, initial)`
- ❌ `find(callback)`
- ❌ `findIndex(callback)`
- ❌ `some(callback)`
- ❌ `every(callback)`
- ❌ **Method chaining** (e.g., `arr.filter().map()`)

#### Object Methods (Missing)
- ❌ `Object.keys(obj)`
- ❌ `Object.values(obj)`
- ❌ `Object.entries(obj)`
- ❌ `Object.assign(target, ...sources)`
- ❌ `Object.hasOwnProperty(key)` (mquickjs supports this)
- ❌ `Object.defineProperty()` (limited in mquickjs)
- ❌ `Object.create(proto)`
- ❌ `Object.freeze(obj)`
- ❌ `Object.seal(obj)`

#### Math Methods (Missing)
- ❌ `Math.pow(x, y)` (use `**` operator)
- ❌ `Math.sin/cos/tan/asin/acos/atan/atan2`
- ❌ `Math.exp/log/log2/log10` (log2/log10 in mquickjs)
- ❌ `Math.random()`
- ❌ `Math.PI`, `Math.E`, other constants
- ❌ `Math.trunc(x)` (mquickjs supports)
- ❌ `Math.fround(x)` (mquickjs supports)
- ❌ `Math.imul(a, b)` (mquickjs supports)
- ❌ `Math.clz32(x)` (mquickjs supports)

#### Number Methods (Missing)
- ❌ `Number.parseInt(string)`
- ❌ `Number.parseFloat(string)`
- ❌ `Number.isNaN(value)`
- ❌ `Number.isFinite(value)`
- ❌ `Number.isInteger(value)`
- ❌ `toFixed(digits)`
- ❌ `toPrecision(digits)`
- ❌ `toExponential(digits)`
- ❌ `toString(radix)`

#### Date Object (Missing)
- ❌ **Entire Date API** (mquickjs only supports `Date.now()`)

#### JSON Object (Missing)
- ❌ `JSON.parse(string)`
- ❌ `JSON.stringify(value)`

#### Global Functions (Missing)
- ❌ `parseInt(string)`
- ❌ `parseFloat(string)`
- ❌ `isNaN(value)`
- ❌ `isFinite(value)`
- ❌ `eval(code)` (only indirect eval in mquickjs)
- ❌ `console.log()` ⚠️ *Useful for debugging*
- ❌ `setTimeout()` / `setInterval()` (not in mquickjs)
- ❌ `globalThis` property (mquickjs supports)

---

## 🚧 Known Limitations in Current Implementation

### Parser & Expression Handling
1. ⚠️ **No method chaining**: `str.charAt(0).toUpperCase()` fails
2. ⚠️ **Complex expressions**: Nested calls like `func(obj.method())` may fail
3. ⚠️ **Operator precedence**: Limited precedence handling in some contexts

### Arrays
4. ⚠️ **No holes enforcement**: mquickjs forbids `arr[10] = 1` if `arr.length < 10`

### Objects
5. ⚠️ **No property descriptors**: All properties are writable/enumerable/configurable

### Strings
6. ⚠️ **UTF-8 handling**: Not using WTF-8 like mquickjs
7. ⚠️ **Case conversion**: ASCII-only (same limitation as mquickjs)

---

## 🎯 Integration Test Status: 8/10 Passing

| Test | Status | Blocker |
|------|--------|---------|
| 01_fibonacci.js | ✅ PASS | - |
| 02_array_processing.js | ✅ PASS | - |
| 03_string_manipulation.js | ❌ FAIL | Method chaining (`.charAt(0).toUpperCase()`) |
| 04_factorial.js | ✅ PASS | - |
| 05_number_formatting.js | ✅ PASS | - |
| 06_array_deduplication.js | ❌ FAIL | `Array.join()` not implemented |
| 07_palindrome_check.js | ✅ PASS | - |
| 08_prime_checker.js | ✅ PASS | - |
| 09_text_statistics.js | ✅ PASS | - |
| 10_nested_data.js | ✅ PASS | - |

**Success rate improved from 5/10 → 8/10** after fixing `<=` and `>=` operators.

---

## 📋 Priority Next Steps

### High Priority (Core Compatibility)
1. **Implement `Array.join(separator)`** → Unblocks test 06
2. **Implement method chaining** → Unblocks test 03
3. **Port mquickjs GC architecture** (compacting GC + JSGCRef)
4. **Port bytecode compiler & VM** (critical for size/speed)
5. **Add proper error handling** (`try`/`catch`/`throw`)

### Medium Priority (Essential Features)
6. **Object.keys/values/entries**
7. **Array methods**: `map`, `filter`, `reduce`, `forEach`
8. **String methods**: `repeat`, `includes`, `startsWith`, `endsWith`
9. **JSON.parse / JSON.stringify**
10. **Regular expressions** (basic support)

### Low Priority (Nice to Have)
11. **Typed arrays** (Int8Array, Uint8Array, etc.)
12. **Math functions** (trig, random)
13. **`for...of` loops**
14. **Arrow functions**
15. **Template literals**

---

## 📊 Architecture Comparison

| Feature | mquickjs | muon-js | Notes |
|---------|----------|---------|-------|
| **Memory Model** | Fixed buffer | Dynamic (Rust heap) | ⚠️ Needs rewrite |
| **GC** | Tracing + compacting | None | ⚠️ Critical missing piece |
| **Value Size** | 32-bit (1 word) | 64-bit (Rust enum) | ⚠️ Need NaN-boxing |
| **String Storage** | WTF-8 | UTF-8 (Rust String) | Close enough |
| **Execution** | Bytecode VM | Direct AST eval | ⚠️ Need bytecode |
| **Parser** | Non-recursive | Recursive | ⚠️ Stack usage issue |
| **Stdlib** | ROM (generated) | Hardcoded Rust | ⚠️ Need ROM generation |
| **C API** | ~20 functions | Partial (5 functions) | ⚠️ Need full API |

---

## 🔍 What Makes mquickjs Special (Not Yet Ported)

1. **10 kB RAM usage** - We're not measuring/optimizing for this yet
2. **ROM-resident bytecode** - No bytecode support at all
3. **Compacting GC** - No GC implemented
4. **No malloc dependency** - We use Rust's heap allocator
5. **WTF-8 strings** - We use standard Rust UTF-8 strings
6. **Stricter mode** - We don't enforce array hole restrictions
7. **Property hash tables** - We use Rust HashMap (dynamic allocation)

---

## 📝 Conclusion

**We have a working JavaScript interpreter with basic features**, but we're still far from mquickjs's core innovations:
- No bytecode compiler/VM
- No GC at all
- No embedded systems optimizations
- No ROM-resident code

**The good news**: The JavaScript feature set is growing steadily (8/10 integration tests passing), and the Rust implementation is clean and maintainable.

**The challenge**: Porting the low-level memory architecture (GC, bytecode, fixed buffers) requires significant architectural changes.

---

## 📚 References

- **mquickjs upstream**: https://github.com/bellard/mquickjs (submodule at `vendor/mquickjs/`)
- **Commit history**: See git log for incremental feature additions
- **Integration tests**: `tests/integration/*.js` (10 tests, 8 passing)
- **mquickjs official tests**: `vendor/mquickjs/tests/` (45 test functions, ~3k lines)

### Next: Run mquickjs Tests Against muon-js

To measure true compatibility, we should:
1. Run `vendor/mquickjs/tests/test_language.js` → Test operators, control flow
2. Run `vendor/mquickjs/tests/test_builtin.js` → Test String/Array/Math methods
3. Run `vendor/mquickjs/tests/test_loop.js` → Test while/for/switch/try-catch

Expected pass rate: **~20-30%** (we have basic features but missing try/catch, switch, bitwise ops, etc.)
