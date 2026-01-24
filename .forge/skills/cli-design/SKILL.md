# CLI Design

Follow these conventions for CLI commands and user interaction:

## Command Structure
- Use clap with derive macros for argument parsing
- Organize commands as subcommands where logical
- Provide both short and long flags for common options
- Include helpful examples in help text

## User Feedback
- Print progress information during long operations
- Use clear, action-oriented language ("Creating...", "Loading...")
- Show success/failure status clearly
- Provide next steps on completion

## Output Formatting
- Use consistent indentation for structured output
- Add blank lines to separate sections for readability
- Keep line length reasonable for terminal display
- Use colors sparingly (via `console` crate) for emphasis

## Error Messages
- Be specific about what went wrong
- Suggest how to fix the problem when possible
- Include relevant file paths or values
- Exit with non-zero status on error

## Confirmation Prompts
- Require confirmation for destructive operations
- Provide `--force` flag to skip prompts in scripts
- Default to the safer option (usually "no")

## Example Pattern
```rust
fn cmd_example(project_dir: &Path) -> Result<()> {
    println!("Performing operation...");

    // Do work
    let result = do_work(project_dir)?;

    // Report success
    println!("Operation complete: {} items processed", result.count);
    println!();
    println!("Next steps:");
    println!("  Run 'forge next-command' to continue");

    Ok(())
}
```
