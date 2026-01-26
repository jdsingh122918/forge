# DRY Principles

Eliminate duplication to improve maintainability. Every piece of knowledge should
have a single, authoritative representation in the codebase.

## Before Writing Code

- Search for existing functions/utilities that solve the problem
- Check if similar patterns exist elsewhere that should be unified
- Consider if this logic belongs in a shared module

## Duplication Red Flags

- Copy-pasting code blocks (extract to function)
- Similar functions with minor variations (parameterize or use generics)
- Repeated validation logic (create validators/middleware)
- Magic strings/numbers appearing multiple times (define constants)
- Similar error handling patterns (create error utilities)

## When Duplication is Acceptable

- Test code: clarity over DRY (some repetition aids readability)
- Early prototyping: extract patterns after they stabilize
- Fewer than 3 occurrences: wait until you see 3 instances before abstracting

## Example Refactoring

```rust
// Before: Duplicated validation
fn create_user(name: &str) -> Result<User> {
    if name.is_empty() { bail!("name required"); }
    // ...
}
fn update_user(name: &str) -> Result<User> {
    if name.is_empty() { bail!("name required"); }
    // ...
}

// After: Extracted validator
fn validate_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "name required");
    Ok(())
}
fn create_user(name: &str) -> Result<User> {
    validate_name(name)?;
    // ...
}
```

## Refactoring for DRY

When you identify duplication:
1. Verify both instances truly represent the same concept
2. Extract to a well-named function/constant/type
3. Update all call sites
4. Run tests to confirm behavior unchanged

## Warning

Premature abstraction is worse than duplication. Only abstract when:
- The pattern appears 3+ times, OR
- The duplication causes active maintenance burden
