# Porting Action Plan: From Clean-Room to Proper Port

## Current Status

**What we built from scratch:**
- ✅ Basic JS interpreter (51/51 core features working)
- ✅ 8/10 integration tests passing
- ✅ Direct AST evaluation (no bytecode)
- ✅ Rust-idiomatic Value enum
- ⚠️ NOT compatible with mquickjs architecture

**What mquickjs has (not ported):**
- ❌ Tagged 32/64-bit value representation
- ❌ Bytecode compiler + stack-based VM
- ❌ Tracing + compacting GC
- ❌ Fixed-size memory buffer (embedded-friendly)
- ❌ ROM-resident bytecode
- ❌ WTF-8 string implementation
- ❌ Proven built-in method implementations

## Strategic Decision: Hybrid Approach

**Option A: Full Architectural Rewrite** (4-6 months)
- Port tagged value system
- Port GC architecture
- Port bytecode VM
- Result: True mquickjs port, but **all current code discarded**

**Option B: Incremental Port** (Recommended)
- Keep current architecture for now
- Port proven built-in method implementations
- Add missing features incrementally
- **Later**: Refactor to tagged values + bytecode VM
- Result: **Working system today, better system tomorrow**

**DECISION: Option B - Incremental Port**

## Phase 1: Quick Wins (This Week)

### Goal: 10/10 integration tests passing

**Task 1: Port `Array.join()` from mquickjs** (2-3 hours)
- File: `vendor/mquickjs/mquickjs.c:14253`
- Blocks: Integration test 06
- Port strategy:
  1. Read mquickjs implementation
  2. Translate C → Rust (adapt to our Value enum)
  3. Add to `src/api.rs` array methods
  4. Test with integration test 06

**Task 2: Fix method chaining** (4-6 hours)
- Blocks: Integration test 03
- Problem: `str.charAt(0).toUpperCase()` fails
- Solution: Fix expression parser to handle chained calls
- Files: `src/api.rs` (parse_call function)

**Task 3: Port `Object.keys()`** (2-3 hours)
- File: `vendor/mquickjs/mquickjs.c:13837`
- Frequently requested feature
- Enables better testing

**Estimated time: 8-12 hours**
**Result: 10/10 integration tests, Object.keys() available**

---

## Phase 2: Built-in Methods (Next 2 Weeks)

### String Methods from mquickjs

Port these proven implementations:

1. **`js_string_repeat()`** (1 hour)
   - ES6 feature, tested in mquickjs
   
2. **`js_string_indexOf()` improvements** (1 hour)
   - Add `lastIndexOf` support
   
3. **`js_string_split()` improvements** (2 hours)
   - Better edge case handling
   
4. **`js_string_trim()` family** (1 hour)
   - `trimStart()`, `trimEnd()`

### Array Methods from mquickjs

5. **`js_array_push()` / `pop()` fixes** (2 hours)
   - Correct return values
   - Proper length updates
   
6. **`js_array_slice()`** (2 hours)
   - Full implementation
   
7. **`js_array_splice()`** (3 hours)
   - Complex but essential
   
8. **`js_array_reverse()`** (1 hour)
   - In-place reversal
   
9. **`js_array_sort()`** (4 hours)
   - With custom comparator
   
10. **Array iteration methods** (8 hours)
    - `forEach()`, `map()`, `filter()`, `reduce()`
    - `every()`, `some()`, `find()`, `findIndex()`

### Object Methods from mquickjs

11. **`Object.values()`** (1 hour)
12. **`Object.entries()`** (1 hour)
13. **`Object.assign()`** (2 hours)

**Estimated time: 30-35 hours**
**Result: Comprehensive built-in method library**

---

## Phase 3: Error Handling (Week 3-4)

### Goal: Unlock mquickjs test suite

**Task 1: Add Exception handling to Value** (4 hours)
- Add `Exception(String)` variant to `Value` enum
- Thread exceptions through all operations

**Task 2: Implement `throw` statement** (6 hours)
- Parse `throw expr;`
- Propagate exceptions up call stack
- Add to control flow enum

**Task 3: Implement `try/catch/finally`** (12 hours)
- Parse try/catch/finally blocks
- Implement exception catching
- Implement finally block execution

**Task 4: Port Error constructors** (4 hours)
- `Error()`, `TypeError()`, `ReferenceError()`, etc.
- From `mquickjs.c` error handling

**Task 5: Port `typeof` operator** (2 hours)
- Required by mquickjs tests

**Estimated time: 28 hours**
**Result: mquickjs test suite unlocked, ~20-30% passing**

---

## Phase 4: Control Flow Completeness (Week 5)

**Task 1: `switch` statements** (8 hours)
- Parse switch/case/default
- Fall-through behavior

**Task 2: `do...while` loops** (2 hours)
- Simple addition

**Task 3: `for...in` loops** (6 hours)
- Iterate over object properties

**Task 4: `for...of` loops** (4 hours)
- Array iteration

**Task 5: Bitwise operators** (4 hours)
- `&`, `|`, `^`, `~`, `<<`, `>>`, `>>>`

**Estimated time: 24 hours**
**Result: mquickjs test suite ~40-50% passing**

---

## Phase 5: Advanced Features (Month 2)

### JSON Support (Week 6)
- Port `JSON.parse()` from mquickjs
- Port `JSON.stringify()` from mquickjs
- Time: 12-16 hours

### Regular Expressions (Week 7-8)
- Port regex engine from mquickjs
- Limited but correct implementation
- Time: 30-40 hours

### Closures (Week 9)
- Proper closure semantics
- Lexical scope capture
- Time: 20-25 hours

---

## Phase 6: Architectural Refactoring (Month 3-4)

This is the **big refactor** to match mquickjs architecture:

### Tagged Value System
- Replace `Value` enum with tagged 32/64-bit representation
- NaN-boxing for floats on 64-bit
- Massive code changes
- Time: 2-3 weeks

### Bytecode Compiler + VM
- Port mquickjs bytecode opcodes
- Port compiler (parser → bytecode)
- Port VM execution loop
- Time: 3-4 weeks

### Garbage Collector
- Port tracing GC
- Port compacting GC
- `JS_PushGCRef` / `JS_PopGCRef` system
- Time: 2-3 weeks

### Fixed-Size Memory Buffer
- Remove Rust heap allocator
- Use provided memory buffer
- Time: 1-2 weeks

**Estimated time: 8-12 weeks**
**Result: True mquickjs port, embedded-friendly, bytecode support**

---

## Milestones

### ✅ Milestone 0: Basic Interpreter (DONE)
- 51/51 core features
- 8/10 integration tests

### 🎯 Milestone 1: Feature Complete (Week 4)
- 10/10 integration tests
- ~30-40% mquickjs test suite passing
- All common built-in methods

### 🎯 Milestone 2: Test Suite Passing (Week 8-10)
- 60-70% mquickjs test suite passing
- Error handling complete
- JSON, closures, regex basics

### 🎯 Milestone 3: Architecture Port (Month 4)
- Tagged values
- Bytecode VM
- GC implemented
- 80-90% mquickjs test suite passing

### 🎯 Milestone 4: Production Ready (Month 5-6)
- 95%+ mquickjs test suite passing
- Embedded-friendly (10 kB RAM)
- Bytecode persistence
- Full mquickjs API compatibility

---

## What to Port First: Prioritized List

### This Sprint (Next 7 Days)
1. ✅ **`Array.join()`** - 2-3 hours - Blocks test
2. ✅ **Method chaining fix** - 4-6 hours - Blocks test
3. ✅ **`Object.keys()`** - 2-3 hours - High value

### Next Sprint (Days 8-14)
4. **String methods** - 6-8 hours - Complete string support
5. **Array slice/splice** - 5-6 hours - Essential operations
6. **Array iteration** - 8-10 hours - `map()`, `filter()`, etc.

### Sprint 3 (Days 15-21)
7. **Error handling basics** - 8-10 hours - Foundation
8. **`throw` statement** - 6-8 hours - Basic exceptions
9. **`try/catch`** - 12-15 hours - Full error handling

### Sprint 4 (Days 22-28)
10. **`typeof` operator** - 2 hours
11. **`switch` statements** - 8 hours
12. **Bitwise operators** - 4 hours
13. **`for...in` loops** - 6 hours

---

## Testing Strategy

After each port:

1. ✅ Run core feature tests (`./tests/test_basic_features.sh`)
2. ✅ Run integration tests (`make test-integration`)
3. ✅ Run mquickjs compatibility (`make test-mquickjs-detailed`)
4. ✅ Track pass rate improvements

**Current baseline:**
- Core: 51/51 (100%)
- Integration: 8/10 (80%)
- mquickjs: 0/45 (0%)

**Target after Phase 3:**
- Core: 51/51 (100%)
- Integration: 10/10 (100%)
- mquickjs: 12-15/45 (~30%)

**Target after Phase 5:**
- Core: 51/51 (100%)
- Integration: 10/10 (100%)
- mquickjs: 25-30/45 (~60%)

---

## Code Reuse vs Rewrite

### Port Directly (High Confidence)
These mquickjs implementations are proven and should be ported with minimal changes:
- ✅ All string methods
- ✅ All array methods
- ✅ Object.keys/values/entries
- ✅ Math functions (if needed)
- ✅ Error constructors
- ✅ JSON parse/stringify

### Adapt for Rust (Medium Confidence)
These need translation but logic is sound:
- ⚠️ Parser (recursive → non-recursive in mquickjs)
- ⚠️ Property lookup
- ⚠️ Type conversions

### Rethink for Rust (Low Confidence)
These may benefit from Rust-specific approaches:
- 🤔 Memory allocation (use Rust allocator first, port later)
- 🤔 Error propagation (use `Result<>` not setjmp/longjmp)
- 🤔 String storage (use Rust String initially)

---

## Documentation Updates Needed

1. Update PLAN.md with hybrid approach decision
2. Update PORTING_STATUS.md with new timelines
3. Add "Ported from mquickjs" comments to each ported function
4. Create ARCHITECTURE.md explaining current vs target design

---

## Next Immediate Action

**Start with Task 1: Port `Array.join()`**

Location: `vendor/mquickjs/mquickjs.c:14253-14310`

Steps:
1. Read the C implementation
2. Understand the algorithm
3. Translate to Rust (adapt to our Value enum)
4. Add to `src/api.rs` in array section
5. Test with: `echo 'a = [1,2,3]; a.join(",")' | cargo run`
6. Run integration test 06

**Estimated time: 2-3 hours**
**Impact: Unblocks integration test 06**

Ready to start?
