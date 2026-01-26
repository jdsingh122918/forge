# Secure Coding

Follow these secure coding practices for all code in this project.

## Input Validation

- Validate all external input (user input, API responses, file contents)
- Use allowlists over denylists when validating
- Sanitize data before use in SQL, shell commands, or HTML output
- Validate early, at system boundaries (CLI args, file reads, network responses)

## Rust-Specific Security

- Minimize `unsafe` blocks; document safety invariants when used
- Use established crypto libraries (`ring`, `rustls`) — never roll your own
- Validate and canonicalize file paths to prevent directory traversal
- Handle symlinks carefully; use `canonicalize()` before path operations
- Prefer `OsStr`/`OsString` for paths from external sources

## Command Execution

- Never interpolate untrusted input into shell commands
- Use `Command::new().arg()` instead of shell string concatenation
- Validate command arguments against an allowlist when possible

## OWASP Awareness

- **Injection**: Use parameterized queries, avoid string interpolation
- **Broken Auth**: Never hardcode credentials, use secure session management
- **Sensitive Data**: Encrypt at rest and in transit, minimize data collection
- **Broken Access Control**: Verify permissions on every request, deny by default
- **Logging**: Log security events, never log secrets or PII

## Secrets Management

- Never commit secrets to version control
- Use environment variables or secret managers for credentials
- Add sensitive file patterns to `.gitignore` (`.env`, `*.key`, `credentials.*`)

## Error Handling

- Never expose stack traces or internal details to users
- Log detailed errors server-side only
- Return generic error messages to external callers
- Use `.context()` to add information without leaking internals
