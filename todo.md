# Failing tests (latest run)

Date: 2026-02-01

## Summary
- `make test`: 43 passed, 21 failed.
- `make test-integration`: 1 passed, 9 failed.

---

## Unit tests failing (`make test`)

1. `tests::array_length_rules`
2. `tests::array_no_holes`
3. `tests::array_sort_and_flatmap`
4. `tests::bracket_call_sets_this`
5. `tests::eval_semicolon_sequence`
6. `tests::array_from_of_and_object_descriptors`
7. `tests::comma_operator_and_eval`
8. `tests::eval_string_concat`
9. `tests::eval_basic_literals`
10. `tests::arrow_default_and_rest_params`
11. `tests::number_formatting_methods`
12. `tests::method_call_sets_this`
13. `tests::nested_closures_shadow_and_mutate`
14. `tests::numeric_property_names_on_arrays`
15. `tests::object_and_number_extras`
16. `tests::object_get_prototype_of_improvements`
17. `tests::regex_string_methods`
18. `tests::regexp_methods_test_exec`
19. `tests::register_stdlib_minimal`
20. `tests::string_locale_stubs_and_normalize`
21. `tests::throw_sets_exception`

### Failure analysis (unit tests)

- **Array invariants and edge cases**
  - `array_length_rules`, `array_no_holes`, `numeric_property_names_on_arrays`, `array_from_of_and_object_descriptors`.
  - Signals: missing/incorrect array length validation, sparse array behavior, numeric index property rules, and Array.from/Object descriptor semantics.

- **Array methods**
  - `array_sort_and_flatmap`.
  - Signals: `Array.prototype.sort` and/or `flatMap` are stubbed or return `undefined`.

- **Method call `this` binding**
  - `bracket_call_sets_this`, `method_call_sets_this`.
  - Signals: method calls are not setting `this` to the receiver for dot/bracket invocation.

- **Eval + parser sequencing / comma operator**
  - `eval_basic_literals`, `eval_string_concat`, `eval_semicolon_sequence`, `comma_operator_and_eval`.
  - Signals: eval returns incorrect expression results; statement splitting or comma operator evaluation is off.

- **Function parameters / closures**
  - `arrow_default_and_rest_params`, `nested_closures_shadow_and_mutate`.
  - Signals: parsing or scope handling for default/rest params and nested capture/shadow behavior.

- **Number/Object builtins**
  - `number_formatting_methods`, `object_and_number_extras`, `object_get_prototype_of_improvements`.
  - Signals: missing Number formatting methods and Object prototype helpers or incorrect return values.

- **RegExp and string localization**
  - `regex_string_methods`, `regexp_methods_test_exec`, `string_locale_stubs_and_normalize`.
  - Signals: RegExp integration and string locale/normalize stubs not matching expected behavior.

- **Exception message shaping**
  - `throw_sets_exception`.
  - Signals: thrown error message is wrapped as `[object Object]` instead of the actual message.

- **Stdlib minimal registration**
  - `register_stdlib_minimal`.
  - Signals: minimal stdlib registration failing or returning error values.

---

## Integration tests failing (`make test-integration`)

Failed (9/10):
1. `tests/integration/01_fibonacci.js`
2. `tests/integration/02_array_processing.js`
3. `tests/integration/03_string_manipulation.js`
4. `tests/integration/04_factorial.js`
5. `tests/integration/05_number_formatting.js`
6. `tests/integration/06_array_deduplication.js`
7. `tests/integration/07_palindrome_check.js`
8. `tests/integration/09_text_statistics.js`
9. `tests/integration/10_nested_data.js`

### Failure analysis (integration tests)

All failing integration scripts error with:
`ReferenceError: not defined` at line 1 or the first assignment.

These scripts use implicit global assignments (e.g., `result = ...`, `data = ...`) without `var/let/const`. If the evaluator runs in strict mode (or strict-like semantics), those assignments should throw `ReferenceError`. That matches the observed errors.

Likely fixes:
- Update integration scripts to declare variables (`let`/`var`) explicitly.
- Or run integration tests in non-strict mode (if that is a supported compatibility target).

---

## Next actions

1. Fix `this` binding on method calls and bracket calls.
2. Correct array length validation and numeric index handling.
3. Repair eval/comma operator result handling.
4. Implement or align missing array methods (sort/flatMap) and number formatting.
5. Decide on integration test strictness; adjust scripts or evaluator accordingly.

---

## Mini-redis persistence + shutdown

- [x] Default mini-redis port to 6379.
- [x] Add release targets for persistence and background run.
- [x] Add stop task to send SIGINT and wait for graceful shutdown.
- [x] On shutdown, dump DB keys/types and snapshot to persistence store.
- [x] Ensure persisted file loads on startup via `--persist`.
