# Expression Parser Assessment

**Date**: January 30, 2026  
**Scope**: Document expression parsing limitations and recommendations

## Current Architecture

The muon-js expression parser is a **recursive descent parser** that processes JavaScript expressions by splitting them into components:

1. **eval_expr()** - Main entry point for expression evaluation
2. **eval_value()** - Evaluates base values (literals, identifiers, built-ins)
3. **split_base_and_tail()** - Separates base from method chains/property access
4. **parse_arith_expr()** - Handles arithmetic expressions

### Key Limitation

The parser uses **string splitting and pattern matching** rather than a proper **tokenizer + AST**. This causes issues with:

- Method chaining on complex expressions
- Constructor calls as expressions
- Nested function calls
- Array/object literals in certain contexts

## Documented Issues

### Issue #1: Method Chaining on Constructor Results

**Pattern**: `Constructor().method()`  
**Example**: `Error("msg").toString()`  
**Status**: Fails with Exception  
**Root Cause**: Parser doesn't handle function calls as base values for chaining

**Test Case**:
```javascript
// ❌ Fails
throw Error("test message");
var x = Error("msg").toString();

// ✅ Works (workaround)
var err = Error("test");
throw err;
```

### Issue #2: Complex Expressions in Specific Contexts

**Pattern**: Expressions in `throw`, `for...of`, etc.  
**Example**: `throw x + 1`, `for (var i of [1,2,3])`  
**Status**: Fails with Exception  
**Root Cause**: Context-specific expression evaluation doesn't handle full expression grammar

**Test Cases**:
```javascript
// ❌ Fails
var x = 5;
throw x + 1;                    // Exception

// ❌ Fails
for (var i of [1, 2, 3]) {     // Exception
    console.log(i);
}

// ✅ Works (workaround)
var msg = x + 1;
throw msg;

var arr = [1, 2, 3];
for (var i of arr) {           // Works with pre-declared array
    console.log(i);
}
```

### Issue #3: Property Access on Literals

**Pattern**: `{...}.prop`, `[...][index]`  
**Example**: `{a: 1}.a`, `[1,2,3][0]`  
**Status**: Fails with Exception  
**Root Cause**: Literals not recognized as valid base values for property access

**Test Cases**:
```javascript
// ❌ Fails
var x = {a: 1, b: 2}.a;        // Exception
var y = [10, 20, 30][1];       // Exception

// ✅ Works (workaround)
var obj = {a: 1, b: 2};
var x = obj.a;                 // Works

var arr = [10, 20, 30];
var y = arr[1];                // Works
```

### Issue #4: NaN/Infinity in Function Contexts

**Pattern**: `func(NaN)`, `func(Infinity)`  
**Example**: `Number.isNaN(NaN)`  
**Status**: Returns incorrect result (false instead of true)  
**Root Cause**: NaN/Infinity literals not properly resolved in function argument position

**Test Cases**:
```javascript
// ❌ Incorrect result
Number.isNaN(NaN);             // Returns false (should be true)
Number.isFinite(Infinity);     // Returns true (should be false)

// ✅ Works (workaround)
var nan = 0 / 0;
Number.isNaN(nan);             // Returns true

var inf = 1 / 0;
Number.isFinite(inf);          // Returns false
```

## Impact Analysis

### Affected Patterns

| Pattern | Workaround Available | Priority |
|---------|---------------------|----------|
| Constructor calls in throw | Yes (use variable) | High |
| Method chaining | Yes (break into steps) | High |
| Expressions in throw | Yes (use variable) | Medium |
| Array literals in for...of | Yes (pre-declare) | Medium |
| Property access on literals | Yes (use variable) | Low |
| NaN/Infinity in functions | Yes (compute 0/0, 1/0) | Low |

### Code Coverage Impact

**Estimated**: 15-20% of real-world JavaScript code is affected  
**Most Affected**: Error handling, functional programming patterns, ES6+ features  
**Least Affected**: Procedural code, simple loops, basic operations

### mquickjs Compatibility

**Current Pass Rate**: ~60-70% (estimated)  
**With Parser Fix**: ~90-95% (estimated)  
**Remaining Gaps**: ES6+ features, regex, closures

## Recommended Solutions

### Short-Term (Current Approach) ✅

**Status**: **IMPLEMENTED**  
**Timeline**: Complete

Document limitations and provide workarounds:
- Use intermediate variables to break up complex expressions
- Pre-declare arrays/objects before using in complex contexts
- Compute NaN/Infinity via arithmetic (0/0, 1/0)

**Advantages**:
- No code changes needed
- Clear documentation for users
- Allows incremental progress on other features

**Disadvantages**:
- ~20% of patterns require workarounds
- Not fully mquickjs-compatible

### Medium-Term (Targeted Fixes) 🟡

**Status**: **OPTIONAL**  
**Timeline**: 2-3 weeks  
**Effort**: 30-40 hours

Fix specific high-impact issues:
1. Allow constructor calls as base values
2. Improve expression evaluation in throw statements
3. Fix array literal parsing in for...of loops

**Implementation**:
```rust
// Example: Allow function calls as base values
fn eval_value(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    // ... existing code ...
    
    // Check if src is a function call (ends with parentheses)
    if src.ends_with(')') {
        if let Some(open_paren) = src.rfind('(') {
            let func_name = src[..open_paren].trim();
            let args = &src[open_paren + 1..src.len() - 1];
            // Evaluate function call...
        }
    }
}
```

**Advantages**:
- Fixes ~80% of issues
- Improves mquickjs compatibility to ~80-85%
- Incremental improvement

**Disadvantages**:
- Patches over architectural issue
- May introduce new edge cases
- Still not fully correct

### Long-Term (Parser Redesign) 🔴

**Status**: **RECOMMENDED**  
**Timeline**: Phase 6 (8-12 weeks)  
**Effort**: ~200 hours

Implement proper tokenizer + AST-based parser:

**Phase 6A: Tokenizer (3-4 weeks)**
- Lexical analysis: keywords, operators, literals, identifiers
- Token stream generation
- Position tracking for error messages

**Phase 6B: Parser (4-6 weeks)**
- Build AST from token stream
- Proper operator precedence
- Full expression grammar support
- Error recovery

**Phase 6C: Evaluator (1-2 weeks)**
- Tree-walking interpreter for AST
- Or bytecode compiler + VM (longer path)

**Advantages**:
- ✅ Fixes 100% of expression issues
- ✅ Enables full mquickjs compatibility
- ✅ Foundation for bytecode compiler
- ✅ Better error messages
- ✅ Easier to extend with new features

**Disadvantages**:
- ❌ Large time investment
- ❌ Complete rewrite of parser
- ❌ Risk of introducing new bugs
- ❌ May require architectural changes

## Decision Matrix

| Criterion | Short-Term | Medium-Term | Long-Term |
|-----------|------------|-------------|-----------|
| Time to implement | ✅ Done | 2-3 weeks | 8-12 weeks |
| Code complexity | ✅ Low | 🟡 Medium | 🔴 High |
| Test compatibility | 🟡 60-70% | 🟡 80-85% | ✅ 95%+ |
| Maintainability | ✅ Good | 🟡 Patches | ✅ Excellent |
| Future-proof | ❌ No | ❌ No | ✅ Yes |
| **Recommendation** | **Use now** | **Skip** | **Do in Phase 6** |

## Recommendation

### Current Status: SHORT-TERM APPROACH ✅

**Action**: Continue with documented workarounds  
**Rationale**:
- muon-js is 65% complete overall
- Expression parser is 1 of 6 major components
- Other features (built-in methods, closures, regex) more important
- Phase 6 will fix this properly

### Next Steps

1. ✅ **Complete**: Document all expression parser limitations (this file)
2. 🟡 **Phase 5**: Implement remaining built-in methods (15% left)
   - Array methods (reduce, filter, map, find)
   - String methods (split, replace, match)
   - Object methods (defineProperty, getOwnPropertyNames)
3. 🟡 **Phase 5**: Improve closures and scope handling
4. 🟡 **Phase 5**: Add regex support (30-40 hours)
5. 🔴 **Phase 6**: Full parser redesign (8-12 weeks)

### Phase 6 Trigger Points

Begin Phase 6 when:
- Phase 5 complete (90%+ feature coverage)
- Expression limitations blocking >30% of use cases
- Ready for 2-3 month effort
- Want to achieve 95%+ mquickjs compatibility

## Conclusion

The expression parser limitations are **known, documented, and acceptable** for the current stage of development. They affect ~20% of JavaScript patterns but have clear workarounds.

**Recommendation**: **Defer parser redesign to Phase 6**. Focus on completing Phase 5 (built-in methods, closures, regex) first.

**Timeline**:
- Phase 5: 6-8 weeks (incremental improvements)
- Phase 6: 8-12 weeks (parser redesign)
- **Total to 95% compatibility**: 4-5 months

**Current Status**: On track for incremental mquickjs compatibility.
