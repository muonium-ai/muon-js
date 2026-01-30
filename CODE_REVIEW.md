# Code Review: muon-js vs mquickjs

## Executive Summary

After reviewing the mquickjs C source code (~28,000 lines), it's clear that **we've been reinventing the wheel**. The mquickjs codebase has proven implementations that we should port directly to Rust rather than implementing from scratch.

## Critical Architecture Differences

### 1. Value Representation

**mquickjs** (production-proven):
```c
typedef uint32_t JSValue;  // 32-bit on 32-bit systems

enum {
    JS_TAG_INT         = 0, /* 31 bit integer (1 bit tag) */
    JS_TAG_PTR         = 1, /* pointer (2 bits tag) */
    JS_TAG_SPECIAL     = 3, /* special values (2 bits tag) */
    JS_TAG_BOOL        = JS_TAG_SPECIAL | (0 << 2),
    JS_TAG_NULL        = JS_TAG_SPECIAL | (1 << 2),
    JS_TAG_UNDEFINED   = JS_TAG_SPECIAL | (2 << 2),
    JS_TAG_EXCEPTION   = JS_TAG_SPECIAL | (3 << 2),
    // ... more special values
};

#define JS_VALUE_GET_INT(v) ((int)(v) >> 1)
```

**Our implementation** (from scratch):
```rust
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Int32(i32),
    Float64(f64),
    String(Rc<String>),
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<HashMap<String, Value>>>),
    // ...
}
```

**Problem**: We use a Rust enum (8+ bytes minimum) vs mquickjs's tagged 32-bit value. This is **fundamentally incompatible** with the memory-efficient design of mquickjs.

### 2. Memory Management

**mquickjs**:
- Fixed-size memory buffer provided at context creation
- Tracing + compacting GC (objects can move)
- `JS_PushGCRef()` / `JS_PopGCRef()` for GC-safe references
- No malloc/free dependencies

**Our implementation**:
- Uses Rust's standard heap allocator
- `Rc<RefCell<>>` for reference counting
- No GC at all
- Objects never move

**Gap**: We're missing the **entire core innovation** of mquickjs.

### 3. Bytecode VM

**mquickjs**:
- Parses JS → compiles to bytecode → executes on stack-based VM
- Bytecode can be persisted to ROM
- VM operations in `mquickjs_opcode.h`

**Our implementation**:
- Direct AST evaluation (no bytecode)
- No VM
- No bytecode persistence

**Gap**: We don't have a **compiler or VM at all**.

## What Should Be Ported Directly

### HIGH PRIORITY: Core Infrastructure

1. **Value Tagging System** (`mquickjs.h` lines 55-86)
   - Tagged 32/64-bit values
   - NaN-boxing for floats on 64-bit
   - Implement as Rust newtype wrapper

2. **String Implementation** (`mquickjs.c` ~lines 5000-6000)
   - WTF-8 encoding (UTF-8 + unpaired surrogates)
   - String interning (atom table)
   - Single-char string optimization
   - **Port**: The entire `JSString` structure and atom table

3. **Array Implementation** (`mquickjs.c` ~lines 14000-15000)
   - No-holes constraint enforcement
   - Fast paths for contiguous arrays
   - **Port**: `js_array_join()`, `js_array_push()`, `js_array_pop()`, etc.

4. **Object/Property System** (`mquickjs.c` ~lines 7000-9000)
   - Property hash table
   - Prototype chain
   - **Port**: Property lookup and storage

### MEDIUM PRIORITY: Built-in Functions

These are **battle-tested** implementations we should port:

#### String Methods (all in `mquickjs.c`)
- `js_string_substring()` (line 13391) - Better than ours
- `js_string_charAt()` (line 13417) - Handles edge cases
- `js_string_indexOf()` - Has both indexOf and lastIndexOf
- `js_string_split()` - Full implementation
- `js_string_toLowerCase()` / `toUpperCase()` - ASCII handling
- `js_string_trim()` - Trim/trimStart/trimEnd
- `js_string_repeat()` - ES6 repeat

#### Array Methods (all in `mquickjs.c`)
- `js_array_join()` (line 14253) - **CRITICAL: Blocks test 06**
- `js_array_push()` - Returns new length (correct behavior)
- `js_array_pop()` - Returns popped element
- `js_array_shift()` / `unshift()`
- `js_array_slice()`
- `js_array_splice()`
- `js_array_reverse()`
- `js_array_sort()` - With custom comparator
- `js_array_indexOf()` / `lastIndexOf()`
- Array iteration: `forEach`, `map`, `filter`, `every`, `some`

#### Math Methods
- Already implemented in `libm.c` (their own tiny math library)
- Can port if we need no_std support

#### Object Methods
- `js_object_keys()` (line ~15000)
- `js_object_hasOwnProperty()`
- `js_object_defineProperty()` - Limited but correct

### LOW PRIORITY: Advanced Features

1. **Error Handling** (`mquickjs.c` ~lines 11000-12000)
   - `throw` statement implementation
   - `try` / `catch` / `finally` bytecode
   - Error object construction
   - **Note**: Requires bytecode VM

2. **Regular Expressions** (`mquickjs.c` ~lines 15500-17000)
   - Limited but correct regex engine
   - Unicode support (partial)

3. **JSON** (in stdlib)
   - `JSON.parse()` / `JSON.stringify()`

## Recommended Porting Strategy

### Phase 1: Value System (1-2 weeks)
1. Port the tagged value representation
2. Implement NaN-boxing for 64-bit
3. Replace our `Value` enum with tagged values
4. Update all code to use new value system

### Phase 2: String System (1 week)
1. Port WTF-8 string implementation
2. Port atom table (string interning)
3. Port all string methods from mquickjs.c

### Phase 3: Array System (3-4 days)
1. Port array structure (no-holes enforcement)
2. Port all array methods (especially `join()`)
3. Add array iteration methods

### Phase 4: Object System (1 week)
1. Port property hash table
2. Port prototype chain
3. Port Object.keys/values/entries

### Phase 5: Error Handling (2-3 days)
1. Add Exception value type
2. Implement throw/try/catch parsing
3. Port error constructors

### Phase 6: Bytecode VM (3-4 weeks)
1. Port bytecode opcodes
2. Port compiler (parser → bytecode)
3. Port VM execution loop
4. Port bytecode persistence

## Specific Functions to Port Immediately

### Critical Blockers (Next Sprint)

1. **`js_array_join()`** - Blocks integration test 06
   ```c
   // vendor/mquickjs/mquickjs.c:14253
   JSValue js_array_join(JSContext *ctx, JSValue *this_val,
                         int argc, JSValue *argv)
   ```
   
2. **String method chaining support** - Blocks integration test 03
   - Requires fixing our expression parser to handle `a.b().c()`

### High-Value Ports (This Month)

3. **`js_object_keys()`** - Frequently needed
4. **`js_string_repeat()`** - ES6 feature
5. **Array iteration methods** - `map()`, `filter()`, `forEach()`
6. **`JSON.parse()` / `JSON.stringify()`** - Critical for real apps

## Files to Focus On

1. **`vendor/mquickjs/mquickjs.h`** (383 lines)
   - Public API and types
   - Value tagging macros
   - **Action**: Port all type definitions

2. **`vendor/mquickjs/mquickjs_priv.h`** (269 lines)
   - Internal structures
   - Built-in function signatures
   - **Action**: Port JSObject, JSString structures

3. **`vendor/mquickjs/mquickjs.c`** (18,325 lines)
   - Core implementation
   - All built-in methods
   - **Action**: Port built-in functions incrementally

4. **`vendor/mquickjs/mquickjs_opcode.h`**
   - Bytecode opcodes
   - **Action**: Port for VM phase

## What NOT to Port

1. **C-specific code**:
   - Platform detection (`#ifdef ARM`, etc.)
   - Manual memory management (use Rust's allocator initially)
   - `setjmp`/`longjmp` for error handling (use Rust `Result`)

2. **Low-level optimizations** (port later):
   - Hand-optimized assembly
   - CPU-specific SIMD
   - Platform-specific float handling

3. **Features we don't need yet**:
   - Proxy objects
   - Reflect API
   - Advanced regexp features

## Compatibility Target

After porting the above, we should aim for:

- **mquickjs test suite**: 70-80% passing
- **Integration tests**: 9-10/10 passing
- **Core features**: 100% (already at 51/51)

## Immediate Next Steps

1. **Port `js_array_join()`** → Unblocks integration test 06
2. **Fix method chaining** → Unblocks integration test 03
3. **Port Object.keys()** → Enables better testing
4. **Port error handling basics** → Unlocks mquickjs test suite
5. **Begin value tagging refactor** → Foundation for future work

## Conclusion

We've built a working JS interpreter from scratch (impressive!), but **mquickjs has 28,000 lines of battle-tested C code** that solves all the problems we're facing:

- ✅ Memory-efficient value representation
- ✅ Proven built-in method implementations
- ✅ Complete bytecode VM
- ✅ GC that works in 10 kB RAM
- ✅ Comprehensive test suite

**Recommendation**: Port proven mquickjs code instead of reinventing. Our time is better spent on Rust-specific improvements (safety, ergonomics, better APIs) rather than reimplementing JavaScript semantics that mquickjs already got right.
