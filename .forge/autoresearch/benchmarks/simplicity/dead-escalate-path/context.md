# Arbiter Verdict System

The arbiter evaluates review failures and decides how to proceed. It produces one of three verdicts:
- **PROCEED**: Continue despite review findings
- **FIX**: Spawn a fix agent and retry
- **ESCALATE**: Require human decision

However, the pipeline is fully autonomous — there is no human-in-the-loop mechanism. No UI, notification system, or webhook exists to deliver an Escalate verdict to a human. In practice, Escalate is treated as a pipeline failure.

The `requires_human()` method returns true for Escalate, but no code ever checks this to pause execution. The `escalation_summary` field is set but never displayed or acted upon.

This was simplified by replacing Escalate with FailPhase (explicit terminal failure), removing the pretense of human escalation.
