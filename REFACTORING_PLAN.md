# Code Refactoring Plan: Split api.rs

**Current**: api.rs is 7,977 lines  
**Target**: Split into 5-6 focused modules of ~1000-1500 lines each

## Module Structure

### 1. **api.rs** (~1500 lines) - PUBLIC API
Keep only the public API functions that mirror mquickjs.h:
- Context creation (js_new_context, js_free_context)
- Value constructors (js_new_int32, js_new_float64, js_new_string, js_new_object, js_new_array)
- Type checks (js_is_number, js_is_string, js_is_function, etc.)
- Property access (js_get_property_str, js_set_property_str)
- Conversion functions (js_to_number, js_to_string, js_to_int32)
- Evaluation (js_eval, js_parse, js_run)
- C-API aliases (JS_NewContext, JS_Eval, etc.)
- GC ref management
- Helper functions that need to be exported

### 2. **eval.rs** (~1200 lines) - EXPRESSION EVALUATION
Move evaluation logic:
- `eval_expr()` - Main expression evaluator
- `eval_value()` - Base value evaluation
- `eval_program()` - Program/statement sequence evaluation
- `eval_function_body()` - Function body execution
- Expression helpers (split_assignment, split_ternary, split_base_and_tail)
- LValue parsing (parse_lvalue)
- Identifier helpers (is_identifier, parse_identifier)
- Truthiness (is_truthy)

### 3. **parser.rs** (~1500 lines) - STATEMENT PARSING  
Move parsing logic:
- `parse_if_statement()`
- `parse_while_loop()`
- `parse_for_loop()` + for...in + for...of variants
- `parse_do_while_loop()`
- `parse_switch_statement()`
- `parse_try_catch()`
- `parse_function_declaration()`
- `parse_arith_expr()`
- Statement splitting (split_statements, split_top_level)
- String literal helpers (is_simple_string_literal)

### 4. **builtins.rs** (~2500 lines) - BUILT-IN METHODS
Move all built-in implementations:
- String methods (charAt, substring, indexOf, toUpperCase, etc.)
- Array methods (push, pop, slice, join, map, filter, etc.)
- Object methods (keys, values, entries, assign, create)
- Number methods (isNaN, isFinite, isInteger, etc.)
- Math methods (abs, floor, ceil, sqrt, etc.)
- JSON methods (stringify, parse)
- Error constructors
- Global functions (parseInt, parseFloat, isNaN, isFinite)

### 5. **json.rs** (~500 lines) - JSON SUPPORT
Move JSON-specific code:
- `JsonParser` struct and impl
- `parse_json()`
- `json_stringify_value()`
- JSON array parsing
- JSON object parsing  
- JSON value parsing
- Escape/unescape helpers

### 6. **helpers.rs** (~300 lines) - UTILITY FUNCTIONS
Move shared utility functions:
- `number_to_value()`
- `contains_arith_op()`
- `find_matching_brace()`
- `find_matching_paren()`
- `is_ident_start()`
- `is_high_surrogate()`, `is_low_surrogate()`
- Array flattening helpers

## Implementation Steps

1. ✅ Create module files with proper imports
2. ✅ Move code blocks preserving functionality
3. ✅ Update imports in remaining files
4. ✅ Test build after each module
5. ✅ Run full test suite
6. ✅ Commit changes

## Benefits

- **Maintainability**: Easier to find and modify specific functionality
- **Readability**: Each module has a clear purpose
- **Testing**: Can test modules independently
- **Collaboration**: Multiple developers can work on different modules
- **Compilation**: Faster incremental rebuilds
- **Documentation**: Easier to document focused modules

## Compatibility

This refactoring **preserves API compatibility**:
- All `pub fn` functions remain in api.rs or re-exported
- No changes to function signatures
- Internal reorganization only
- Tests should pass identically

## File Sizes (Estimated)

```
src/
├── api.rs         1,500 lines (was 7,977)
├── eval.rs        1,200 lines (new)
├── parser.rs      1,500 lines (new)
├── builtins.rs    2,500 lines (new)
├── json.rs          500 lines (new)
├── helpers.rs       300 lines (new)
├── context.rs        25K (unchanged)
├── lib.rs            30K (update mod declarations)
├── types.rs         3.7K (unchanged)
└── value.rs         3.1K (unchanged)
```

**Total**: Same LOC, better organized
