# Rust Conventions

Follow these Rust conventions for all code in this project:

## Code Style
- Use `rustfmt` defaults for formatting
- Run `cargo clippy` and address all warnings before committing
- Prefer `?` operator for error propagation over `.unwrap()` in library code
- Use `.expect("meaningful message")` only when failure indicates a bug

## Error Handling
- Use `anyhow::Result` for application code and CLI
- Use `thiserror` for library errors that callers need to match on
- Provide context with `.context()` or `.with_context()`
- Log errors at the appropriate level (error, warn, info)

## Naming
- Use `snake_case` for functions, methods, variables, and modules
- Use `CamelCase` for types, traits, and enum variants
- Prefix private helper functions with underscore only if they shadow public ones
- Use descriptive names; avoid single letters except for trivial loops

## Module Structure
- One module per file, except for small related types
- Put tests in a `#[cfg(test)] mod tests` block at the end of each file
- Use `pub(crate)` for internal visibility when appropriate
- Re-export important types from the crate root

## Documentation
- Add doc comments (`///`) for all public items
- Include examples in doc comments for complex functions
- Use `//` comments sparingly for non-obvious implementation details
- Keep CLAUDE.md updated with key concepts and patterns

## Testing
- Write unit tests for all non-trivial functions
- Use descriptive test names: `test_<function>_<scenario>`
- Group related tests with comment headers
- Use `tempdir()` for tests that need filesystem access
