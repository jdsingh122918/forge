# Testing Strategy

Apply this testing strategy for all new code:

## Test Levels
1. **Unit Tests** - Test individual functions in isolation
   - Located in `mod tests` at the bottom of each file
   - Mock external dependencies
   - Fast, deterministic, no I/O

2. **Integration Tests** - Test modules working together
   - Located in `tests/` directory
   - Use real dependencies where practical
   - May involve filesystem, database, or network

3. **End-to-End Tests** - Test complete user workflows
   - Test CLI commands with actual execution
   - Verify output matches expectations

## Test Naming Convention
```rust
#[test]
fn test_<function_name>_<scenario>() {
    // Example: test_parse_config_with_missing_file()
}
```

## Test Structure (AAA Pattern)
```rust
#[test]
fn test_example() {
    // Arrange - Set up test data and conditions
    let input = create_test_input();

    // Act - Execute the code under test
    let result = function_under_test(input);

    // Assert - Verify the results
    assert_eq!(result, expected_value);
}
```

## What to Test
- Happy path (normal operation)
- Edge cases (empty input, boundaries, limits)
- Error conditions (invalid input, failures)
- Regression tests for fixed bugs

## Test Helpers
- Create helper functions for common test setup
- Use `tempdir()` for filesystem tests
- Use builders for complex test data

## Running Tests
```bash
cargo test                    # Run all tests
cargo test <test_name>        # Run specific test
cargo test -- --nocapture     # Show println! output
```
