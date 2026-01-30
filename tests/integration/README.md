# Integration Tests

Complex, real-world JavaScript scenarios to test MuonJS capabilities.

## Purpose
These tests are **not modified** to pass - they represent natural JavaScript code patterns. The goal is to track which realistic scenarios work and which features are still missing.

## Running Tests
```bash
./tests/run_integration.sh
```

## Test Results (as of 2026-01-30)

**Overall: 5/10 passing (50%)**

### ✓ Passing Tests

1. **02_array_processing** - Array iteration, arithmetic, conditionals (result: 2809)
2. **05_number_formatting** - Math operations, string concatenation with variables
3. **06_array_deduplication** - Nested loops, array operations, uniqueness logic
4. **09_text_statistics** - String/array processing, word counting, averaging
5. **10_nested_data** - 2D arrays, nested loops, matrix operations

### ✗ Failing Tests

1. **01_fibonacci** - Function with early return (return in if statement)
2. **03_string_manipulation** - String concatenation with + operator for building strings
3. **04_factorial** - Function with early return (return in if statement)
4. **07_palindrome_check** - String comparison, negative loop indexing, string building
5. **08_prime_checker** - Function with multiple early returns, complex conditionals

## Missing Features Identified

### Critical Issues
- **Early returns in conditionals**: Functions can't return from within if statements
- **String concatenation operator**: `+` doesn't work for combining strings dynamically
- **Negative indexing**: Can't use negative values in loops effectively

### Working Features
- ✓ Functions with simple returns
- ✓ While loops
- ✓ Array operations (push, length, indexing)
- ✓ String methods (split, charAt, substring, toUpperCase)
- ✓ Math operations
- ✓ Nested loops
- ✓ Conditionals (if/else)
- ✓ Variable assignments in loops
- ✓ 2D arrays

## Test Descriptions

### 01_fibonacci
Calculates the 10th Fibonacci number using iteration.
- Uses early return for base case
- Iterative algorithm with variable swapping

### 02_array_processing ✓
Processes an array to find sum and maximum value.
- Demonstrates array iteration
- Multiple while loops
- Arithmetic operations

### 03_string_manipulation
Capitalizes a name by splitting, processing, and rejoining.
- String splitting and array access
- String concatenation with +
- Character-by-character manipulation

### 04_factorial
Calculates factorials for two numbers and adds them.
- Recursive-style base case handling
- Function calls with different arguments
- Arithmetic on results

### 05_number_formatting ✓
Formats a price with tax as a currency string.
- Floating point arithmetic
- Math.floor for rounding
- String concatenation

### 06_array_deduplication ✓
Removes duplicates from an array.
- Nested loop pattern
- Boolean flags
- Array building logic

### 07_palindrome_check
Checks if a string is a palindrome by reversing it.
- Reverse iteration (i--)
- String building character by character
- String equality comparison

### 08_prime_checker
Finds all prime numbers up to 20.
- Function with multiple return statements
- Complex conditional logic
- Array building in loop

### 09_text_statistics ✓
Calculates word count and average word length.
- String splitting
- Multiple iterations
- Average calculation

### 10_nested_data ✓
Sums all elements in a 2D array (matrix).
- Nested arrays
- Double loop pattern
- Accumulator pattern

## Notes
- Tests are kept in their original form to track engine progress
- Each test represents common JavaScript patterns
- Passing tests validate multiple features working together
