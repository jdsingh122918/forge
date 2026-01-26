# Context7 Integration

Query Context7 for up-to-date library documentation before implementing code that
uses external dependencies. This ensures you use current APIs and best practices.

## When to Query Context7

- Before first use of an unfamiliar or rapidly-evolving library
- When implementing patterns you haven't used recently
- When errors suggest API changes or deprecations
- When the spec mentions specific library features

## When NOT to Query

- Standard library features (Rust `std`, Python built-ins)
- Well-known stable APIs you're confident about (`serde`, `clap` basics)
- Internal project modules

## How to Query

Use the MCP tools in sequence:

1. **Resolve the library ID first:**
   ```
   Tool: mcp__plugin_context7_context7__resolve-library-id
   Parameters:
     libraryName: "tokio"
     query: "async file operations"
   ```

2. **Query documentation with the resolved ID:**
   ```
   Tool: mcp__plugin_context7_context7__query-docs
   Parameters:
     libraryId: "/tokio-rs/tokio" (from step 1)
     query: "how to read file asynchronously"
   ```

## Guidelines

- Query once per library per phase — avoid redundant queries for the same library
- Be specific in queries: "jwt token validation" not just "auth"
- If the library isn't found, proceed with general knowledge but note the gap
- Limit to 3 Context7 queries per phase to avoid excessive API calls

## Handling Missing Libraries

If Context7 doesn't have documentation for a library:
1. Check the library's README or docs.rs
2. Proceed with general knowledge
3. Add extra test coverage for that integration
