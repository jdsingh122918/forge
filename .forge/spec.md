# Implementation Spec: Single Experiment Step for Autoresearch Loop

> Generated from: docs/superpowers/specs/autoresearch-tasks/T11-single-experiment.md
> Generated at: 2026-03-12T02:17:37.657038+00:00

## Goal

Implement the run_single_experiment() function — the core unit of the autoresearch loop that proposes a prompt mutation via an LLM, writes it, runs benchmarks, judges outputs, computes cost, and returns a structured ExperimentResult with keep/discard outcome. Uses trait-based abstraction (PromptMutator, BenchmarkExecutor) for testability and to decouple from not-yet-implemented S02/S03 modules.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| Core Types | Data structs: BenchmarkScores, JudgeVerdict, ExperimentOutcome (Keep/Discard/Error), ExperimentResult, MutationProposal, ExperimentConfig — all with serde derives for serialization | low | - |
| PromptMutator Trait | Async trait for proposing prompt mutations. Takes current prompt, history summary, and specialist name; returns MutationProposal with new prompt text, description, and token counts | low | - |
| BenchmarkExecutor Trait | Async trait for running benchmarks and judging outputs. run_and_judge() returns JudgeVerdict; last_run_tokens() returns token usage for cost tracking | low | - |
| run_single_experiment Function | Core async function implementing the 7-step flow: read current prompt, propose mutation, write mutated prompt, placeholder commit SHA, run benchmarks+judge, determine keep/discard outcome, estimate cost. Supports dry_run mode that skips file writes and benchmark execution | medium | Core Types, PromptMutator Trait, BenchmarkExecutor Trait |
| Module Registration | Add pub mod experiment; to src/cmd/autoresearch/mod.rs | low | run_single_experiment Function |

## Code Patterns

### Async trait pattern

```
#[async_trait::async_trait]
pub trait PromptMutator: Send + Sync {
    async fn propose_mutation(
        &self,
        current_prompt: &str,
        history: &str,
        specialist: &str,
    ) -> Result<MutationProposal>;
}
```

### Cost estimation formula

```
let cost = (total_input as f64 / 1_000_000.0) * 3.0
    + (total_output as f64 / 1_000_000.0) * 15.0;
```

### Dry run guard pattern

```
if !config.dry_run {
    std::fs::write(&config.prompt_path, &proposal.new_prompt)
        .with_context(|| format!("Failed to write mutated prompt: {}", config.prompt_path.display()))?;
}
```

### Outcome determination

```
let outcome = if config.dry_run {
    ExperimentOutcome::Keep
} else if verdict.composite_score > config.baseline_score {
    ExperimentOutcome::Keep
} else {
    ExperimentOutcome::Discard
};
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch::experiment passes with 9+ tests green
- [ ] ExperimentResult, ExperimentOutcome, ExperimentConfig, BenchmarkScores, JudgeVerdict, MutationProposal structs exist with serde derives
- [ ] PromptMutator and BenchmarkExecutor are async_trait traits with Send + Sync bounds
- [ ] run_single_experiment() implements full 7-step flow: read prompt, propose mutation, write prompt, placeholder commit, run benchmarks, determine outcome, estimate cost
- [ ] Dry run mode skips file writes and benchmark execution, returns Keep outcome
- [ ] Mutation failure propagates as error with 'Mutation proposal failed' context
- [ ] Benchmark failure propagates as error with 'Benchmark execution failed' context
- [ ] Cost estimation correctly sums mutator + executor tokens using Sonnet pricing ($3/MTok input, $15/MTok output)
- [ ] ExperimentResult roundtrips through serde_json serialization
- [ ] Module registered in src/cmd/autoresearch/mod.rs as pub mod experiment
- [ ] cargo clippy -- -D warnings passes clean

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T11-single-experiment.md*
