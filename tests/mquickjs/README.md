# mquickjs Compatibility Tests

This directory contains copies of the official mquickjs test suite from the upstream repository.

## Test Files

- **test_language.js** (355 lines) - Language features: operators, comparisons, type conversion
- **test_builtin.js** (875 lines) - Built-in objects: String, Array, Math, Number, JSON, RegExp
- **test_loop.js** (395 lines) - Control flow: while, for, for-in, switch, try/catch
- **test_closure.js** (106 lines) - Closure semantics and scope
- **mandelbrot.js** (39 lines) - Mandelbrot set computation (simple program test)
- **test_rect.js** (68 lines) - Rectangle class test
- **microbench.js** (1137 lines) - Performance benchmarks

## Running Tests

### Basic Test Suite (file-level)
```bash
./tests/run_mquickjs_tests.sh
```

This runs each test file as a whole and reports pass/fail status.

### Detailed Compatibility Check (function-level)
```bash
./tests/check_mquickjs_compatibility.sh
```

This extracts individual test functions and runs them separately, providing detailed compatibility statistics.

## Current Limitations

Many tests will fail because muon-js does not yet support:

- ❌ `throw` / `try` / `catch` / `finally` (error handling)
- ❌ `Error()` constructor
- ❌ `typeof` operator
- ❌ `switch` statements
- ❌ Bitwise operators (`&`, `|`, `^`, `~`, `<<`, `>>`, `>>>`)
- ❌ `for...in` loops
- ❌ `do...while` loops
- ❌ Constructors with `new` keyword
- ❌ `this` keyword
- ❌ Regular expressions
- ❌ JSON.parse / JSON.stringify
- ❌ Typed arrays
- ❌ Many built-in methods

## Test Structure

The mquickjs tests use a common pattern:

```javascript
function assert(actual, expected, message) {
    // Custom assertion logic
    if (actual !== expected) {
        throw_error("assertion failed: ...");
    }
}

function test_something() {
    // Test code
    assert(1 + 1, 2);
}

// Call all test functions
test_something();
// ... more test calls
```

Since we don't support `throw`/`Error`, tests that call `assert` with failing conditions will hit our missing features.

## Expected Pass Rate

Based on current muon-js features:

- **test_language.js**: ~10-20% (basic operators work, but missing bitwise ops, typeof, etc.)
- **test_builtin.js**: ~5-10% (missing most String/Array methods, JSON, RegExp, etc.)
- **test_loop.js**: ~20-30% (have while/for, but missing do-while, for-in, switch, try/catch)
- **test_closure.js**: ~0% (closures not implemented)
- **mandelbrot.js**: Likely fails (complex expressions)
- **test_rect.js**: ~0% (uses `new` constructor)

**Overall**: ~10-15% compatibility with full mquickjs test suite.

## Updating Tests

To update tests from upstream:

```bash
cd vendor/mquickjs
git pull
cd ../..
cp -r vendor/mquickjs/tests/* tests/mquickjs/
```
