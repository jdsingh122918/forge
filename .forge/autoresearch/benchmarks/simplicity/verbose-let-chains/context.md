# Stream JSON Parser — Verbose Conditional Patterns

This module parses streaming JSON events from Claude CLI subprocess output. It matches against various JSON message types (content_block_delta, content_block_start, assistant messages) to extract text, thinking, and tool events.

The parsing logic uses deeply nested `if`/`if let` chains with 3-4 levels of indentation. Rust's let-chain syntax (`if condition && let Some(x) = expr { ... }`) can flatten these into single-level conditions, reducing visual complexity and indentation without changing behavior.

This is a readability/simplicity issue, not a bug. The deeply nested style makes it harder to see the logical flow and increases the chance of subtle logic errors when modifying the conditions.
