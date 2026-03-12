# Arbiter Resolution Mode

The `ResolutionMode` enum controls how the system handles gating review failures. It was designed with three modes:
- **Manual**: Pause for human input (never used — no UI exists for this)
- **Auto**: Auto-fix with retry limit (the actual behavior in all deployments)
- **Arbiter**: LLM-based decision making (used, but its `escalate_on` and `auto_proceed_on` fields are never configured)

In practice, the system always uses auto-fix with optional LLM support. The Manual mode is the Default variant but is immediately overridden by forge.toml configuration. The Arbiter's escalation and auto-proceed category lists are empty in every known deployment.

This was simplified to a single `ResolutionMode` struct with `max_attempts: u32`, `model: Option<String>`, and `confidence_threshold: f64` — removing 456 lines and adding 254 lines. The deprecated enum variants are accepted during deserialization and mapped to the unified struct.
