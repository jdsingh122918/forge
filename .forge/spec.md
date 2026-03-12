# Implementation Spec: Full Loop Orchestration (T13)

> Generated from: docs/superpowers/specs/autoresearch-tasks/T13-full-loop-orchestration.md
> Generated at: 2026-03-12T02:24:18.031608+00:00

## Goal

Implement loop_runner.rs — the top-level orchestrator that drives the autoresearch experiment loop by iterating over specialists, running experiments via run_single_experiment (T11), tracking budget (T10), logging results (T12), managing git state (T14), and handling keep/discard decisions with consecutive failure counting.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| LoopConfig | Configuration struct holding tag, specialists list, budget cap, max failures, project/prompts dirs, dry_run and resume flags | low | - |
| LoopOutcome and LoopSummary | LoopOutcome enum (Completed, BudgetExhausted, DryRun) and LoopSummary struct with accounting fields (total experiments/keeps/discards/errors, budget spent/remaining) | low | - |
| LoopGitOps trait | Trait abstracting git operations (create_branch, checkout_branch, commit, reset_to_last_keep) for testability with mock implementations | low | - |
| run_loop orchestrator | Async function that initializes state dirs, results log, budget tracker, and git branch; then iterates over specialists running experiments in a loop with budget gate, failure gate, keep/discard/error handling, baseline advancement, and disk persistence after each experiment | high | LoopConfig, LoopOutcome and LoopSummary, LoopGitOps trait |
| resolve_prompt_path helper | Simple helper that joins prompts_dir with specialist name + .md extension | low | - |
| mod.rs wiring | Add pub mod loop_runner to mod.rs and wire cmd_autoresearch to call run_loop | low | run_loop orchestrator |

## Code Patterns

### LoopGitOps trait for testable git abstraction

```
pub trait LoopGitOps: Send + Sync {
    fn create_branch(&self, branch_name: &str) -> Result<()>;
    fn checkout_branch(&self, branch_name: &str) -> Result<()>;
    fn commit(&self, message: &str) -> Result<String>;
    fn reset_to_last_keep(&self) -> Result<()>;
}
```

### Budget gate pattern

```
if !budget.can_afford(ESTIMATED_EXPERIMENT_COST) {
    eprintln!("Budget exhausted");
    budget_exhausted = true;
    break 'outer;
}
```

### Consecutive failure gate pattern

```
if consecutive_failures >= config.max_failures {
    eprintln!("Max consecutive failures reached, moving to next specialist");
    break;
}
```

### Keep/Discard/Error match handling

```
match &exp_result.outcome {
    ExperimentOutcome::Keep => {
        let sha = git.commit(&format!("experiment: {}", exp_result.description))?;
        results.append(ResultRow { status: ExperimentStatus::Keep, .. })?;
        baseline_score = exp_result.score;
        consecutive_failures = 0;
    }
    ExperimentOutcome::Discard => {
        results.append(ResultRow { status: ExperimentStatus::Discard, .. })?;
        git.reset_to_last_keep()?;
        consecutive_failures += 1;
    }
    ExperimentOutcome::Error(msg) => {
        results.append(ResultRow { status: ExperimentStatus::Error, .. })?;
        git.reset_to_last_keep()?;
        consecutive_failures += 1;
    }
}
```

### MockGit test implementation

```
struct MockGit {
    commits: Mutex<Vec<String>>,
    resets: AtomicU32,
    branch_created: Mutex<Option<String>>,
}

impl LoopGitOps for MockGit {
    fn commit(&self, message: &str) -> Result<String> {
        let mut commits = self.commits.lock().unwrap();
        let sha = format!("sha{:04}", commits.len());
        commits.push(message.to_string());
        Ok(sha)
    }
    fn reset_to_last_keep(&self) -> Result<()> {
        self.resets.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
```

### ScriptedExecutor for controllable test scores

```
struct ScriptedExecutor {
    scores: Mutex<Vec<f64>>,
}

impl ScriptedExecutor {
    fn new(scores: Vec<f64>) -> Self {
        Self { scores: Mutex::new(scores) }
    }
}

#[async_trait::async_trait]
impl BenchmarkExecutor for ScriptedExecutor {
    async fn run_and_judge(&self, _specialist: &str, _prompt_path: &Path) -> Result<JudgeVerdict> {
        let score = {
            let mut s = self.scores.lock().unwrap();
            if s.is_empty() { 0.0 } else { s.remove(0) }
        };
        Ok(JudgeVerdict { recall: score, precision: score, actionability: score, composite_score: score, reasoning: format!("score={}", score) })
    }
}
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch::loop_runner passes with 10+ tests green
- [ ] src/cmd/autoresearch/loop_runner.rs exists with LoopConfig, LoopSummary, LoopOutcome, LoopGitOps trait, run_loop(), resolve_prompt_path()
- [ ] run_loop() iterates over specialists, runs experiments, handles keep/discard/error outcomes
- [ ] LoopGitOps trait allows mock git operations in tests
- [ ] Budget exhaustion halts the loop with LoopOutcome::BudgetExhausted
- [ ] max_failures consecutive discards move to the next specialist
- [ ] Successful keep resets the consecutive failure counter and advances baseline score
- [ ] Results (results.tsv) and budget (budget.json) are persisted to disk after each experiment
- [ ] Dry run returns immediately with LoopOutcome::DryRun and zero experiments
- [ ] Missing prompt file skips the specialist instead of crashing
- [ ] mod.rs declares pub mod loop_runner and wires cmd_autoresearch to call run_loop
- [ ] cargo clippy -- -D warnings passes clean

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T13-full-loop-orchestration.md*
