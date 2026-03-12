# SOLID Violation: God Function with 6 Concerns

This is a synthetic benchmark case demonstrating a single function
(`run_pipeline_for_issue`) that handles 6 distinct concerns in ~170 lines,
violating the Single Responsibility Principle:

1. **Input validation** — Checking issue state, project existence, spec presence
2. **Git operations** — Branch creation, slugification, pushing
3. **Phase execution** — Iterating through phases, running forge commands
4. **Budget tracking** — Counting iterations against a maximum budget
5. **Review handling** — Running specialist reviews, attempting fix iterations
6. **PR creation and notification** — GitHub PR creation, Slack webhook posting

The function has 8 parameters including boolean flags (`notify_slack`) and optional
values (`slack_webhook`, `github_repo`) that control branching behavior within the
function body — a classic sign that the function is doing too much.

Each concern should be extracted into its own function or struct method, with the
top-level function serving as a coordinator that delegates to focused helpers.
