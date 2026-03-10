# LLM Council Engine — Design Document

**Date:** 2026-03-10
**Status:** Approved
**Inspired by:** Karpathy's llm-council project

---

## Executive Summary

Forge currently uses Claude CLI exclusively as its AI backend. The Council Engine introduces Codex CLI (OpenAI) as a complementary engine, implementing an LLM Council pattern: two independent AI workers implement each phase in isolation, anonymously peer-review each other's output, and a dedicated chairman model synthesizes the best merged result. This produces higher-quality code through adversarial review and diverse reasoning while remaining fully backward-compatible with existing single-engine execution.

---

## Table of Contents

1. [Goals and Non-Goals](#goals-and-non-goals)
2. [Core Concept](#core-concept)
3. [Three-Stage Pipeline](#three-stage-pipeline)
4. [Model Assignments](#model-assignments)
5. [Merge Strategy](#merge-strategy)
6. [Failure Cascade](#failure-cascade)
7. [Configuration](#configuration)
8. [Architecture Flow](#architecture-flow)
9. [Integration with Existing Subsystems](#integration-with-existing-subsystems)
10. [Module Layout](#module-layout)
11. [Worker Trait](#worker-trait)
12. [Audit Trail](#audit-trail)
13. [Environment Variables](#environment-variables)
14. [Implementation Notes](#implementation-notes)

---

## Goals and Non-Goals

### Goals

1. **Higher-quality output** — Two independent implementations with adversarial peer review catch more bugs and produce better designs than a single model.
2. **Model diversity** — Leverage different reasoning strengths of Claude and GPT-5.4 to reduce blind spots.
3. **Impartial synthesis** — A dedicated chairman model merges the best of both implementations without bias toward either worker.
4. **Graceful degradation** — If merging fails, fall back to winner-takes-all, then human escalation.
5. **Zero-impact default** — Council is off by default. Existing `forge run` and `forge swarm` behavior is unchanged until explicitly enabled.
6. **Full auditability** — Every implementation, review, and chairman decision is captured for post-hoc analysis.

### Non-Goals

- Supporting more than two workers in the initial implementation.
- Replacing the existing single-engine execution path.
- Running the council across distributed machines.
- Using council mode for review specialist phases (those already have their own adversarial pattern).

---

## Core Concept

An **LLM Council** where Claude and Codex independently implement each phase, anonymously peer-review each other's work, and a dedicated chairman (GPT-5.4 xhigh via Codex CLI) synthesizes the best merged result. The chairman is a distinct role — it never participates as a worker, ensuring impartiality.

The pattern draws from Karpathy's llm-council: run multiple models on the same task, cross-evaluate, and aggregate. Forge adapts this for code generation by using git worktree isolation and diff-based merging rather than simple text voting.

---

## Three-Stage Pipeline

Each council-enabled phase executes three sequential stages:

### Stage 1: Independent Implementation

Both workers receive the identical phase prompt and execute in **isolated git worktrees**, producing two independent diffs against the same baseline commit.

- Claude works in `worktree-a/`, Codex works in `worktree-b/`.
- Both run in parallel via `tokio::join!`.
- Each worker's output is captured as a unified diff against the baseline.
- Workers are unaware of each other's existence.

### Stage 2: Anonymized Peer Review

Each worker reviews the other's diff under **randomized labels** to prevent model-identity bias.

- Labels are randomized per phase: one diff is presented as "Candidate Alpha," the other as "Candidate Beta." The mapping is shuffled so neither worker can infer which model produced which diff.
- Claude reviews Codex's diff (labeled Alpha or Beta, randomly assigned).
- Codex reviews Claude's diff (labeled with the other label).
- Both reviews run in parallel.
- Reviews produce structured output: numerical scores (correctness, completeness, style, performance) and free-text critiques with specific line references.

### Stage 3: Chairman Synthesis

A dedicated chairman model receives all four artifacts — both diffs and both reviews — and produces a single unified result.

- The chairman is GPT-5.4 with `reasoning_effort: xhigh`, invoked via Codex CLI.
- It receives the diffs in the same anonymized labeling as the reviews.
- It produces a **git-format patch** that can be applied directly to the baseline.
- The chairman never participates as a worker in any stage, ensuring it approaches the synthesis without prior commitment to either implementation.

---

## Model Assignments

| Role | Model | CLI | Notes |
|------|-------|-----|-------|
| Worker A | Claude (default model) | `claude` | Via `claude` CLI with `--print --output-format stream-json` |
| Worker B | GPT-5.4 | `codex` | Via `codex` CLI with `-q --json`, `reasoning_effort: xhigh` |
| Chairman | GPT-5.4 | `codex` | Same binary as Worker B, distinct prompt/role. `reasoning_effort: xhigh` |

The chairman reuses `council.workers.codex.cmd` — no third binary is required. The distinction is purely in the prompt: the chairman receives a synthesis-oriented system prompt rather than an implementation-oriented one.

---

## Merge Strategy

**Diff-based merge** — both engines work on isolated git branches forked from the same baseline commit.

1. After Stage 1, extract `diff_a` and `diff_b` as unified diffs against the baseline.
2. The chairman in Stage 3 receives both diffs plus both peer reviews.
3. The chairman produces a **git-format patch** (`git diff --format` compatible).
4. Forge applies the patch to the baseline using `git apply`.
5. If the patch applies cleanly, the phase succeeds with the merged result.

This avoids three-way merge complexity — the chairman is responsible for resolving conflicts intellectually, not mechanically.

---

## Failure Cascade

A layered fallback strategy handles merge failures:

```
Merge retry (budget: 3)
    │
    │  Chairman patch fails to apply
    ▼
Winner-takes-all (by peer review score)
    │
    │  Both implementations critically flawed
    ▼
Human escalation
```

### Layer 1: Merge Retry

If the chairman's patch fails `git apply`, re-invoke the chairman with the error output appended to context. Retry up to `chairman_retry_budget` times (default: 3). Each retry includes the previous failure reason so the chairman can correct its patch.

### Layer 2: Winner-Takes-All

If all retries are exhausted, select the implementation with the higher aggregate peer review score. Apply that worker's diff directly as the phase result. Log the fallback event in the audit trail.

### Layer 3: Human Escalation

If both implementations received critically low scores (below a configurable threshold) or both failed to produce valid diffs, halt the pipeline and surface the issue for human intervention. This reuses the existing `OnFailure` hook mechanism.

---

## Configuration

### Primary Configuration (forge.toml)

```toml
[council]
enabled = true
chairman_model = "gpt-5.4"
chairman_reasoning_effort = "xhigh"
chairman_retry_budget = 3
anonymize_reviews = true

[council.workers.claude]
cmd = "claude"
role = "worker"
flags = ["--print", "--output-format", "stream-json"]

[council.workers.codex]
cmd = "codex"
role = "worker"
flags = ["-q", "--json"]
model = "gpt-5.4"
reasoning_effort = "xhigh"
sandbox = "workspace-write"
approval_policy = "on-failure"
```

### Field Descriptions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `council.enabled` | bool | `false` | Master switch. When false, Forge uses single-engine execution. |
| `council.chairman_model` | string | `"gpt-5.4"` | Model used for chairman synthesis. |
| `council.chairman_reasoning_effort` | string | `"xhigh"` | Reasoning effort level for the chairman. |
| `council.chairman_retry_budget` | u32 | `3` | Max chairman retry attempts before fallback. |
| `council.anonymize_reviews` | bool | `true` | Randomize candidate labels in peer review. |
| `council.workers.<name>.cmd` | string | required | CLI command to invoke the worker. |
| `council.workers.<name>.role` | string | `"worker"` | Role identifier. |
| `council.workers.<name>.flags` | string[] | `[]` | CLI flags appended to every invocation. |
| `council.workers.<name>.model` | string | optional | Model override (for workers that accept `--model`). |
| `council.workers.<name>.reasoning_effort` | string | optional | Reasoning effort level. |
| `council.workers.<name>.sandbox` | string | optional | Sandbox mode (Codex-specific). |
| `council.workers.<name>.approval_policy` | string | optional | Approval policy (Codex-specific). |

### Phase-Level Overrides

Use existing glob patterns in `[phases.overrides]` to disable council for specific phases or adjust chairman parameters:

```toml
# Disable council for scaffold phases (simple, not worth the cost)
[phases.overrides."*-scaffold"]
council = { enabled = false }

# Increase retry budget for security-critical phases
[phases.overrides."*-security*"]
council = { chairman_reasoning_effort = "xhigh", chairman_retry_budget = 5 }
```

This reuses the existing phase override mechanism in Forge, extending it with a `council` sub-table.

---

## Architecture Flow

```
Phase Scheduler (DAG/Sequential)
        |
        v
  Council Engine
  |-- Stage 1: Claude + Codex in isolated git worktrees (parallel)
  |   |-- worktree A --> diff_a
  |   +-- worktree B --> diff_b
  |-- Stage 2: Anonymized peer review (parallel)
  |   |-- Claude reviews diff_b as "Candidate Alpha"
  |   +-- Codex reviews diff_a as "Candidate Beta"
  +-- Stage 3: Chairman synthesis
      |-- Receives: diff_a, diff_b, review_a, review_b
      |-- Produces: git-format patch
      +-- Failure cascade: retry --> winner-takes-all --> escalate
```

### Detailed Sequence

```
1. Phase scheduler dispatches phase P to the Council Engine
2. Council Engine creates two git worktrees from HEAD
3. [Parallel] Claude executes in worktree-a, Codex executes in worktree-b
4. Extract diff_a (worktree-a vs baseline), diff_b (worktree-b vs baseline)
5. Assign randomized labels: e.g., diff_a = "Candidate Beta", diff_b = "Candidate Alpha"
6. [Parallel] Claude reviews "Candidate Alpha" (diff_b), Codex reviews "Candidate Beta" (diff_a)
7. Chairman receives: diff_a, diff_b, review_of_a, review_of_b (all with anonymized labels)
8. Chairman produces git-format patch
9. Apply patch to baseline
   - Success: phase complete, clean up worktrees
   - Failure: enter failure cascade
10. Emit PhaseResult (same type as single-engine phases)
11. Clean up worktrees
```

---

## Integration with Existing Subsystems

### Drop-in Replacement

The Council Engine returns the same `PhaseResult` type as single-engine phases. From the perspective of the DAG scheduler, swarm executor, and state persistence layer, a council-enabled phase is indistinguishable from a normal phase. The council is an internal implementation detail of phase execution.

### Off by Default

When `council.enabled` is `false` (the default), the Council Engine module is never invoked. Zero code paths change. Zero performance impact.

### Hooks

Existing hooks fire at well-defined points during council execution:

| Hook | When it fires |
|------|---------------|
| `PrePhase` | Before Stage 1 begins |
| `PreIteration` | Fires twice — once per worker at the start of Stage 1 |
| `PostIteration` | After Stage 3 completes (chairman has produced the merged result) |
| `PostPhase` | After the merged patch is applied to the repository |
| `OnFailure` | If the failure cascade reaches human escalation |
| `OnApproval` | If the phase requires approval (existing behavior, unchanged) |

### DAG and Swarm

From the DAG scheduler's perspective, a council-enabled phase occupies **one slot**. Internally, the council uses up to three concurrent processes (two workers + chairman synthesis), but this is transparent to the scheduler. The phase blocks its DAG slot until all three stages complete.

### Signals

Both workers' output is parsed for Forge signals (`<promise>`, `<progress>`, `<blocker>`, `<pivot>`). Signals from both workers are collected and forwarded to the orchestrator. If either worker emits a `<blocker>`, the council engine surfaces it.

### Compaction

Each worker manages its own context window independently. The council engine does not share context between workers — this is by design, as independent reasoning is the core value proposition.

### Audit Trail

Every council execution produces a complete audit record (see [Audit Trail](#audit-trail) section below). This integrates with the existing state persistence layer.

---

## Module Layout

```
src/council/
|-- mod.rs          # Public API: Engine::run_phase()
|-- config.rs       # CouncilConfig parsed from forge.toml
|-- engine.rs       # Three-stage orchestration loop
|-- worker.rs       # Worker trait + ClaudeWorker + CodexWorker
|-- reviewer.rs     # Stage 2: anonymized peer review logic
|-- chairman.rs     # Stage 3: synthesis + merge + retry cascade
|-- merge.rs        # Git patch parsing, apply, conflict detection
|-- prompts.rs      # Prompt templates for review & chairman
+-- types.rs        # WorkerResult, ReviewResult, MergeOutcome, etc.
```

### Module Responsibilities

| Module | Responsibility |
|--------|---------------|
| `mod.rs` | Exports `Engine` with a single public method `run_phase()`. Entry point for the orchestrator. |
| `config.rs` | Deserializes `[council]` from `forge.toml` into `CouncilConfig`. Handles defaults, validation, and phase-level overrides. |
| `engine.rs` | Orchestrates the three stages sequentially. Manages worktree lifecycle. Handles the failure cascade. |
| `worker.rs` | Defines the `Worker` trait and implements `ClaudeWorker` and `CodexWorker`. Each knows how to invoke its CLI, parse output, and extract diffs. |
| `reviewer.rs` | Implements anonymized peer review. Randomizes labels, constructs review prompts, parses structured review output. |
| `chairman.rs` | Invokes the chairman model with both diffs and reviews. Parses the git-format patch output. Implements retry logic. |
| `merge.rs` | Applies git-format patches via `git apply`. Detects conflicts. Computes diff stats. |
| `prompts.rs` | Contains prompt templates for peer review and chairman synthesis. Templates are parameterized with diffs, labels, and review content. |
| `types.rs` | Defines `WorkerResult`, `ReviewResult`, `ReviewScores`, `MergeOutcome`, `CouncilAudit`, and other shared types. |

### Modified Existing Files

| File | Change |
|------|--------|
| `src/orchestrator/runner.rs` | Check `CouncilConfig::enabled` before dispatching a phase. If enabled, delegate to `council::Engine::run_phase()` instead of the single-engine path. |
| `src/forge_config.rs` | Add `council: Option<CouncilConfig>` field. Deserialize the `[council]` table. |
| `src/phase.rs` | Add `council_override: Option<CouncilPhaseOverride>` to phase-level configuration for per-phase council settings. |

---

## Worker Trait

```rust
#[async_trait]
pub trait Worker: Send + Sync {
    /// Human-readable name for logging and audit trail (e.g., "claude", "codex").
    fn name(&self) -> &str;

    /// Execute a phase prompt in the given worktree directory.
    /// Returns the worker's output and the diff against baseline.
    async fn execute(
        &self,
        phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult>;

    /// Review another worker's diff under an anonymized candidate label.
    /// Returns structured scores and critique.
    async fn review(
        &self,
        phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult>;
}
```

### Key Types

```rust
pub struct WorkerResult {
    pub diff: String,              // Unified diff against baseline
    pub diff_stats: DiffStats,     // Files changed, insertions, deletions
    pub signals: Vec<Signal>,      // Parsed promise/progress/blocker/pivot signals
    pub duration: Duration,
    pub token_usage: Option<TokenUsage>,
    pub raw_output: String,        // Full CLI output for audit
}

pub struct ReviewResult {
    pub scores: ReviewScores,
    pub issues: Vec<ReviewIssue>,
    pub summary: String,
    pub candidate_label: String,   // "Alpha" or "Beta"
    pub duration: Duration,
}

pub struct ReviewScores {
    pub correctness: f32,          // 0.0 - 1.0
    pub completeness: f32,
    pub style: f32,
    pub performance: f32,
    pub overall: f32,              // Weighted aggregate
}

pub enum MergeOutcome {
    Merged {
        patch: String,
        stats: DiffStats,
        attempts: u32,
    },
    WinnerTakesAll {
        winner: String,            // Worker name
        diff: String,
        reason: String,
    },
    Escalated {
        reason: String,
    },
}
```

---

## Audit Trail

Every council execution produces a structured audit record persisted alongside the phase result. This enables post-hoc analysis of council effectiveness, cost comparison, and model performance tracking.

### Format

```json
{
  "phase": "05",
  "council": {
    "worker_a": {
      "engine": "claude",
      "diff_stats": {
        "files_changed": 4,
        "insertions": 120,
        "deletions": 30
      },
      "duration_ms": 45000,
      "token_usage": {
        "input": 8500,
        "output": 3200
      }
    },
    "worker_b": {
      "engine": "codex",
      "model": "gpt-5.4",
      "reasoning_effort": "xhigh",
      "diff_stats": {
        "files_changed": 3,
        "insertions": 95,
        "deletions": 22
      },
      "duration_ms": 62000
    },
    "reviews": {
      "claude_reviewed_codex": {
        "scores": {
          "correctness": 0.9,
          "completeness": 0.85,
          "style": 0.8,
          "performance": 0.95,
          "overall": 0.88
        },
        "issues": []
      },
      "codex_reviewed_claude": {
        "scores": {
          "correctness": 0.85,
          "completeness": 0.9,
          "style": 0.75,
          "performance": 0.8,
          "overall": 0.83
        },
        "issues": [
          "Unnecessary allocation in hot loop at src/engine.rs:142"
        ]
      }
    },
    "chairman": {
      "model": "gpt-5.4",
      "reasoning_effort": "xhigh",
      "merge_attempts": 1,
      "resolution": "merged",
      "final_diff_stats": {
        "files_changed": 4,
        "insertions": 130,
        "deletions": 28
      }
    },
    "labels": {
      "alpha": "codex",
      "beta": "claude"
    },
    "total_duration_ms": 125000
  }
}
```

The `labels` field records the randomized assignment so auditors can correlate anonymized reviews with actual workers.

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CLAUDE_CMD` | `claude` | CLI command for the Claude worker (existing variable). |
| `CODEX_CMD` | `codex` | CLI command for the Codex worker. |
| `COUNCIL_CHAIRMAN_MODEL` | `gpt-5.4` | Override the chairman model at runtime. |
| `COUNCIL_ENABLED` | `false` | Override `council.enabled` at runtime without modifying `forge.toml`. |

Environment variables take precedence over `forge.toml` values when set. This allows CI/CD pipelines to toggle council mode without config file changes.

---

## Implementation Notes

### Git Worktree Management

- Create worktrees with `git worktree add` at the start of Stage 1.
- Each worktree gets a unique branch name: `council/<phase-id>/<worker-name>-<timestamp>`.
- Clean up worktrees with `git worktree remove` after Stage 3 completes (or on failure).
- Use the `git2` crate (already a dependency) for worktree operations where possible, falling back to CLI for operations `git2` does not support.

### Cost Considerations

Council mode roughly triples the token cost per phase (two implementations + reviews + chairman). This is why it is off by default and supports per-phase overrides — teams can enable it selectively for high-stakes phases (security, core architecture) while keeping simple phases (scaffolding, docs) on single-engine execution.

### Prompt Design

- **Review prompts** present the diff with the anonymized label and ask for structured JSON output with scores and issues. The prompt explicitly instructs the reviewer not to guess which model produced the diff.
- **Chairman prompts** present both diffs and both reviews, instruct the chairman to produce a git-format patch, and emphasize that the goal is the best possible merged result — not a mechanical union of both diffs.
- Prompt templates live in `src/council/prompts.rs` as const strings with placeholder substitution.

### Concurrency Model

- Stage 1: Two workers run in parallel via `tokio::join!`.
- Stage 2: Two reviews run in parallel via `tokio::join!`.
- Stage 3: Chairman runs sequentially (single invocation, retried on failure).
- The entire council pipeline is `async` and runs within the existing Tokio runtime.

### Testing Strategy

- **Unit tests**: Test label randomization, diff extraction, patch application, score aggregation, and failure cascade logic with mock workers.
- **Integration tests**: Use a test repository with known file contents, run two mock workers that produce predetermined diffs, verify that the chairman prompt is correctly constructed and the patch applies.
- **Worker trait mocking**: The `Worker` trait enables injecting mock implementations for testing without invoking real CLI tools.
