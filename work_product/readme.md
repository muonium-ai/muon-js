# Work product: debug JavaScript snippets

This folder contains the JavaScript snippets that were written to reproduce specific mquickjs compatibility issues and to lock in the fixes. They are intentionally minimal and targeted so they can be re-run quickly during regressions.

## Files and purpose

- debug_func_newline.js
  - Reproduces the parse error for named function expressions when the opening `{` is on the next line. This validated the line-continuation normalization change.

- test_rect.js
  - Exercises the Rectangle/FilledRectangle test path and verifies that `Function.call` respects overrides. This was used to debug the `Rectangle.call` behavior in the test_rect scenario.

- test_for_in_debug.js
  - Minimal `for (i in obj)` iteration with assertion output, used to confirm correct lvalue assignment and property enumeration order.

- debug_closure3.js
  - Tests recursive named function expressions (`fib1`) and closure behavior. This was used to reproduce the closure3 failure.

- closure3_test.js
  - Condensed version of the closure3 reproduction for quick manual runs and output checking.

These files are kept as lightweight regression probes for the specific fixes made during the mquickjs compatibility push in early 2026.
