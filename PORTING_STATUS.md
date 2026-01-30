# Porting Status: mquickjs → muon-js

## Overview

**Current Progress: ~55% core features ported**

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
- ✅ `charCodeAt(index)`
- ✅ `substring(start)` / `substring(start, end)`
- ✅ `slice(start, end)`
- ✅ `indexOf(substring)`
- ✅ `lastIndexOf(substring)`
- ✅ `split(separator)`
- ✅ `concat(...strings)`
- ✅ `toUpperCase()`
- ✅ `toLowerCase()`
- ✅ `trim()` / `trimStart()` / `trimEnd()`
- ✅ `length` property
- ✅ `startsWith(prefix)` / `endsWith(suffix)`
- ✅ `includes(substring)` / `repeat(count)`
- ✅ `padStart(length, fill)` / `padEnd(length, fill)`
- ✅ `replace(search, replacement)` / `replaceAll(search, replacement)`
- ✅ `match(regex)` / `matchAll(regex)` / `search(regex)`

#### Array Methods
- ✅ `push(element)` / `pop()` / `shift()` / `unshift()`
- ✅ `slice(start, end)` / `splice(start, deleteCount, ...items)`
- ✅ `concat(...arrays)` / `join(separator)`
- ✅ `reverse()` / `sort()` (numeric)
- ✅ `indexOf(element)` / `lastIndexOf(element)` / `includes(element)`
- ✅ `forEach(callback)` / `map(callback)` / `filter(callback)` / `reduce(callback, initial)`
- ✅ `find(callback)` / `findIndex(callback)` / `some(callback)` / `every(callback)`
- ✅ `flat()` / `flatMap()` (partial)
- ✅ `Array.isArray()` / `Array.from()` / `Array.of()`
- ✅ `length` property
- ✅ Array indexing: `arr[i]`
- ✅ `for` iteration over arrays

#### Object Methods
- ✅ Object creation: `{key: value}`
- ✅ Property access: `obj.key` and `obj[key]`
- ✅ Property assignment
- ✅ `Object.keys(obj)` / `Object.values(obj)` / `Object.entries(obj)`
- ✅ `Object.assign(target, ...sources)`
- ✅ `Object.hasOwnProperty(key)`
- ✅ `Object.defineProperty()` (simplified)
- ✅ `Object.getOwnPropertyDescriptor()` (simplified)
- ✅ `Object.create(proto)` (simplified)
- ✅ `Object.freeze(obj)` (stub)
- ✅ `Object.seal(obj)` (stub)
- ✅ `Object.isSealed(obj)` / `Object.isFrozen(obj)` (stub)
- ✅ `Object.getPrototypeOf(obj)` (improved)

#### Math Object
- ✅ `Math.abs(x)` / `Math.floor(x)` / `Math.ceil(x)` / `Math.round(x)`
- ✅ `Math.sqrt(x)` / `Math.pow(x, y)`
- ✅ `Math.max(a, b, ...)` / `Math.min(a, b, ...)`
- ✅ `Math.sin/cos/tan/asin/acos/atan/atan2`
- ✅ `Math.exp/log/log2/log10`
- ✅ `Math.fround(x)` / `Math.imul(a, b)` / `Math.clz32(x)`
- ✅ `Math.trunc(x)` / `Math.random()`
- ✅ `Math.PI`, `Math.E` constants

---

## ❌ What's Missing

### Core Engine Architecture
- ⚠️ **Tracing GC scaffolding** (mark-only, no sweep/compaction yet)
- ✅ **JSGCRef system** (`JS_PushGCRef` / `JS_PopGCRef`) (stubbed, no GC integration yet)
- ⚠️ **Bytecode compiler & VM** (layout scaffolding only; no codegen or execution yet)
- ⚠️ **Bytecode persistence** (`JS_PrepareBytecode` / `JS_LoadBytecode` / `JS_RelocateBytecode`): header checks + basic relocation; no compiler or VM yet
- ✅ **Atom table** (string interning + refcounts; no GC integration yet)
- ✅ **Memory buffer allocation** (fixed-size buffer for embedded systems)
- ⚠️ **Standard library ROM plumbing** (stdlib metadata stored; no ROM generation yet)

### JavaScript Language Features
- ❌ **Strict mode enforcement** (mquickjs is always strict)
- ❌ **Typed arrays** (Int8Array, Uint8Array, Float32Array, etc.)
- ✅ **Regular expressions** (subset; see Regex section)
- ✅ **Error handling**: `try`/`catch`/`finally`/`throw`
- ❌ **Object constructors**: `new Constructor()`
- ❌ **Prototypes & inheritance**: `prototype`, `__proto__`
- ✅ **`this` keyword** (method calls and basic function calls)
- ❌ **Arrow functions**: `() => {}`
- ❌ **Template literals**: `` `string ${expr}` ``
- ❌ **Destructuring**: `[a, b] = arr`, `{x, y} = obj`
- ❌ **Spread operator**: `...arr`
- ❌ **Rest parameters**: `function(...args)`
- ❌ **Default parameters**: `function(a = 1)`
- ✅ **`for...in` loops** (object property iteration)
- ✅ **`for...of` loops** (array iteration - mquickjs supports this)
- ❌ **Block-scoped variables**: `let`, `const`
- ✅ **Switch statements**
- ✅ **Ternary operator**: `condition ? a : b`
- ❌ **Comma operator**
- ✅ **Comma operator**
- ✅ **Bitwise operators**: `&`, `|`, `^`, `~`, `<<`, `>>`, `>>>`

### Built-in Objects & Methods

#### String Methods (Missing)
- ❌ **Method chaining** (e.g., `str.charAt(0).toUpperCase()`)

#### Array Methods (Missing)
- ❌ **Method chaining** (e.g., `arr.filter().map()`)

#### Object Methods (Missing)
- None

#### Number Methods (Missing)
- None

#### Date Object (Partial)
- ✅ `Date.now()`
- ❌ Remaining Date API (not in mquickjs scope)

#### JSON Object (Missing)
- ❌ Full compatibility audit for `JSON.parse` / `JSON.stringify`

#### Global Functions (Missing)
- ✅ `eval(code)` (basic)
- ✅ `console.log()` (basic)
- ❌ `setTimeout()` / `setInterval()` (not in mquickjs)

---

## 🚧 Known Limitations in Current Implementation

### Parser & Expression Handling
1. ⚠️ **Method chaining gaps**: some chained expressions still fail in complex contexts
2. ⚠️ **Complex expressions**: Nested calls like `func(obj.method())` may fail
3. ⚠️ **Operator precedence**: Limited precedence handling in some contexts

### Arrays
4. ✅ **No holes enforcement**: mquickjs forbids `arr[10] = 1` if `arr.length < 10`

### Objects
5. ⚠️ **Property descriptors simplified**: defineProperty/getOwnPropertyDescriptor are value-only

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

Run with: `make test-integration` or `./tests/run_integration.sh`

---

## 🧪 Test Infrastructure

### 1. Core Feature Tests (51 tests)
Tests basic JavaScript features in isolation without mquickjs test framework dependencies.

```bash
./tests/test_basic_features.sh
```

**Status: 51/51 passing (100%)** ✅

Categories tested:
- Arithmetic operators (8 tests)
- Comparison operators (8 tests)
- Logical operators (6 tests)
- Variables (2 tests)
- Strings (7 tests)
- Arrays (3 tests)
- Objects (2 tests)
- Functions (2 tests)
- Control flow (6 tests)
- Math (7 tests)

### 2. Integration Tests (10 tests)
Real-world JavaScript programs testing complete features.

```bash
make test-integration
# or
./tests/run_integration.sh
```

**Status: 8/10 passing (80%)** ✅

### 3. mquickjs Compatibility Tests (45 test functions)
Official test suite from upstream mquickjs repository.

```bash
make test-mquickjs-detailed
# or
./tests/check_mquickjs_compatibility.sh
```

**Status: 0/45 passing (0%)** ❌  
**Blocker**: All mquickjs tests use `throw`/`try`/`catch`/`Error()` which we don't support yet.

Test files:
- `test_language.js` - 9 test functions (operators, type conversion)
- `test_builtin.js` - 12 test functions (String, Array, Math, JSON, RegExp)
- `test_loop.js` - 16 test functions (while, for, for-in, switch, try/catch)
- `test_closure.js` - 3 test functions (closure semantics)
- `mandelbrot.js`, `test_rect.js` - Simple programs

### 4. Rust Unit Tests
```bash
cargo test
# or
make test
```

Tests internal Rust implementation details.

---

## 🎯 Integration Test Status: 8/10 Passing (MOVED ABOVE)

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
