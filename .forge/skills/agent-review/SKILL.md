# Agent Review

Before claiming phase completion, spawn an independent review agent to verify
your implementation. This catches issues you may have missed.

## When to Trigger Review

- After all implementation for the phase is complete
- After all tests pass
- Before emitting `<promise>DONE</promise>`

## How to Spawn Review Agent

Use the Task tool with these parameters:

- **subagent_type**: `"general-purpose"`
- **description**: `"Review phase N implementation"` (replace N with phase number)
- **prompt**: Include the phase context and checklist below

Example prompt to pass to the review agent:

```text
Review the code changes for Phase {N} - {phase name}.

Run this command to see what changed:
git diff HEAD~{iterations} --stat
git diff HEAD~{iterations}

Check for:
1. DRY violations - duplicated code that should be extracted
2. Security issues - injection risks, hardcoded secrets, missing validation
3. Test coverage - are edge cases covered, are tests meaningful
4. Code clarity - naming, structure, comments where needed

Report issues as a numbered list with file:line references.
If no issues found, respond with: REVIEW PASSED
```

Replace `{N}` with the phase number, `{phase name}` with the phase name, and
`{iterations}` with the number of iterations completed in this phase.

## Handling Review Results

- **REVIEW PASSED**: Proceed to emit `<promise>DONE</promise>`
- **Issues found**: Address each issue, re-run tests, then request another review
- **Max 2 review cycles**: If issues persist after 2 reviews, emit `<blocker>review-failed</blocker>` and describe the unresolved issues

## Review Scope

The reviewer checks only the current phase's changes, not the entire codebase.
Use `git diff HEAD~N` where N is the iteration count to see phase changes.
