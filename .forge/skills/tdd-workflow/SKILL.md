# TDD Workflow

Apply Test-Driven Development when implementing new functionality. Use your judgment
to determine when TDD is appropriate — it's most valuable for business logic,
algorithms, and complex state management.

## When to Use TDD

- New functions with defined inputs/outputs
- Bug fixes (write failing test first, then fix)
- Refactoring existing code (ensure tests exist first)
- API endpoints and data transformations

## When TDD May Not Apply

- Configuration files, constants, types/interfaces
- Simple wiring code with no logic
- Exploratory prototyping (but add tests before phase completion)

## The TDD Cycle

1. **Red**: Write a failing test that defines expected behavior
2. **Green**: Write minimal code to make the test pass
3. **Refactor**: Clean up while keeping tests green

## Example

```rust
// 1. RED - Write the test first (this won't compile yet)
#[test]
fn test_parse_phase_number_valid_input() {
    assert_eq!(parse_phase_number("03"), Ok(3));
}

#[test]
fn test_parse_phase_number_invalid_input() {
    assert!(parse_phase_number("abc").is_err());
}

// 2. GREEN - Minimal implementation to pass
fn parse_phase_number(s: &str) -> Result<u32> {
    s.parse().context("invalid phase number")
}

// 3. REFACTOR - Clean up if needed (in this case, it's already clean)
```

## Guidelines

- Test behavior, not implementation details
- One assertion per test when practical
- Name tests descriptively: `test_<function>_<scenario>` (e.g., `test_load_config_missing_file`)
- Run `cargo test` after each change to verify state
- Never emit `<promise>DONE</promise>` with failing tests
