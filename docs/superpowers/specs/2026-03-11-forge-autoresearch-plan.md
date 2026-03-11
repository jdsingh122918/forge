# forge autoresearch — TDD Implementation Plan

**Design doc:** `docs/superpowers/specs/2026-03-11-forge-autoresearch-design.md`
**Methodology:** GSD (Get Stuff Done) with TDD (Red-Green-Refactor)

---

## Milestone M01: `forge autoresearch` — Autonomous Agent Optimization

**Goal:** Ship `forge autoresearch` command that autonomously improves review specialist prompts via experimentation loop with cross-model evaluation.

---

## Slice S01: Prompt Extraction + Production Loading

**Demo sentence:** "After this, review specialists load prompts from `.forge/autoresearch/prompts/` files, falling back to hardcoded defaults."

### Boundary Map
- **Produces:** `PromptLoader` struct with `load_specialist_prompt(specialist_type, forge_dir) -> PromptConfig`
- **Produces:** `PromptConfig` struct with `focus_areas: Vec<String>`, `body: String`, `specialist_name: String`
- **Produces:** 4 prompt `.md` files extracted from hardcoded templates
- **Consumes:** `SpecialistType` from `src/review/specialists.rs`
- **Integrates:** `build_review_prompt()` in `src/review/dispatcher.rs` gains file-exists fallback

**Note on `mode` field:** The YAML frontmatter `mode` field is informational only (for human reference). Gating behavior is determined by `SpecialistType::default_gating()` in `specialists.rs`, not by the prompt config. The autoresearch loop can mutate prompt content without affecting gating semantics.

### Tasks

#### T01: PromptConfig type + PromptLoader with file-exists fallback

**TDD sequence:**

1. **Red:** Write test `test_load_prompt_returns_hardcoded_default_when_no_file`
   - Create `PromptLoader` with a temp dir that has no prompt files
   - Call `load_specialist_prompt(SecuritySentinel, &temp_dir)`
   - Assert returns `PromptConfig` with default focus areas and role text

2. **Red:** Write test `test_load_prompt_from_file_overrides_default`
   - Write a `security-sentinel.md` file to temp dir with custom focus areas
   - Call `load_specialist_prompt(SecuritySentinel, &temp_dir)`
   - Assert returned config has the custom focus areas, not defaults

3. **Red:** Write test `test_load_prompt_parses_yaml_frontmatter_and_body`
   - Write prompt file with `---\nspecialist: SecuritySentinel\n---\n## Role\nCustom role...`
   - Assert `PromptConfig.specialist_name` is "SecuritySentinel" and body text matches

4. **Red:** Write test `test_load_prompt_handles_malformed_file_gracefully`
   - Write a prompt file with invalid YAML frontmatter
   - Assert falls back to hardcoded default (no panic)

5. **Green:** Implement `PromptConfig` struct and `PromptLoader` to pass all tests

6. **Refactor:** Clean up, ensure error types are clear

**Files:**
- Create: `src/review/prompt_loader.rs`
- Modify: `src/review/mod.rs` (add `pub mod prompt_loader;`)

**Must-haves:**
- Truth: Loading from missing file returns hardcoded defaults
- Truth: Loading from valid file returns file contents
- Truth: Malformed files fall back gracefully
- Artifact: `src/review/prompt_loader.rs` with `PromptConfig`, `PromptLoader`

---

#### T02: Extract hardcoded prompts into config files

**Steps:**

1. Create directory `.forge/autoresearch/prompts/`
2. Extract from `dispatcher.rs` and `specialists.rs`:
   - `security-sentinel.md` — SecuritySentinel focus areas + prompt body
   - `performance-oracle.md` — PerformanceOracle focus areas + prompt body
   - `architecture-strategist.md` — ArchitectureStrategist focus areas + prompt body
   - `simplicity-reviewer.md` — SimplicityReviewer focus areas + prompt body
3. Each file has YAML frontmatter (`specialist`) + markdown body (focus areas, role, instructions, output format)
4. The `mode` field in frontmatter is informational only (e.g., `mode: gating # informational`)

**TDD sequence:**

1. **Red:** Write test `test_extracted_prompt_loads_via_prompt_loader`
   - For each specialist type, load from extracted file via `PromptLoader`
   - Assert `PromptConfig` returned (not a fallback to defaults)
   - Assert focus areas list is non-empty

2. **Red:** Write test `test_extracted_prompt_focus_areas_match_hardcoded`
   - For each specialist, compare loaded focus areas against `SpecialistType::default_focus_areas()`
   - Assert lists match

3. **Green:** Create the 4 prompt files

**Must-haves:**
- Artifact: `.forge/autoresearch/prompts/security-sentinel.md`
- Artifact: `.forge/autoresearch/prompts/performance-oracle.md`
- Artifact: `.forge/autoresearch/prompts/architecture-strategist.md`
- Artifact: `.forge/autoresearch/prompts/simplicity-reviewer.md`
- Truth: Each extracted file loads successfully via `PromptLoader` and returns a `PromptConfig` matching the hardcoded defaults

---

#### T03: Wire PromptLoader into dispatcher.rs

**TDD sequence:**

1. **Red:** Write integration test `test_dispatcher_uses_file_prompt_when_available`
   - Set up a PhaseReviewConfig with SecuritySentinel
   - Place a custom `security-sentinel.md` in the forge dir
   - Call `build_review_prompt()` and assert it uses the file content

2. **Red:** Write integration test `test_dispatcher_falls_back_to_hardcoded`
   - Set up PhaseReviewConfig with SecuritySentinel
   - Do NOT place any prompt file
   - Call `build_review_prompt()` and assert it uses hardcoded content

3. **Green:** Modify `build_review_prompt()` in `dispatcher.rs`:
   - Add `forge_dir: Option<&Path>` parameter
   - If forge_dir provided, try `PromptLoader::load_specialist_prompt()`
   - If file found, use it; otherwise fall back to existing hardcoded logic

4. **Refactor:** Ensure no behavior change when `forge_dir` is `None`

**Files:**
- Modify: `src/review/dispatcher.rs` (add file-exists check in `build_review_prompt`)
- Modify: callers of `build_review_prompt` (pass forge_dir)

**Must-haves:**
- Truth: Existing behavior unchanged when no prompt files exist
- Truth: File-based prompts override hardcoded defaults
- Key Link: `dispatcher.rs` imports and uses `PromptLoader`

---

## Slice S02: Benchmark Suite + Runner

**Demo sentence:** "After this, `forge autoresearch --dry-run` runs one specialist against benchmarks and prints recall/precision scores."

### Boundary Map
- **Produces:** `BenchmarkSuite` struct with `load(forge_dir, specialist) -> Vec<BenchmarkCase>`
- **Produces:** `BenchmarkResult` struct — output of running specialist against one case
- **Produces:** `BenchmarkRunner` struct with `run(prompt_config, benchmark_case, claude_cmd) -> BenchmarkResult`
- **Produces:** `Scorer` struct with `score(results, expected, judge_result?) -> SpecialistScore`
- **Produces:** `FindingMatcher` — matches specialist output against expected findings
- **Consumes:** `PromptConfig` from S01
- **Consumes:** Claude CLI for specialist invocation

### Matching Specification

A `must_find` item is considered detected if the specialist output contains a finding whose:
- `file` matches the expected `location` file (if specified), AND
- `line` is within ±5 lines of expected `location` line range, AND
- `issue` description is semantically related (keyword overlap > 50% of expected description tokens)

A `must_not_flag` item is considered falsely flagged if:
- A finding's `file` and `line` match the `must_not_flag` location (if specified), OR
- A finding's `issue` description has >60% keyword overlap with the `must_not_flag` description

Novel findings (not matching any `must_find` or `must_not_flag`) are classified by the GPT 5.4 judge in S03.

### Tasks

#### T04: Benchmark types + loader

**TDD sequence:**

1. **Red:** Write test `test_load_benchmark_from_directory`
   - Create temp dir with `code.rs`, `context.md`, `expected.json`
   - Call `BenchmarkCase::load(&dir)`
   - Assert fields populated correctly

2. **Red:** Write test `test_load_benchmark_suite_discovers_all_cases`
   - Create 3 benchmark dirs under `benchmarks/security/`
   - Call `BenchmarkSuite::load(&forge_dir, "security")`
   - Assert 3 cases returned, sorted by name

3. **Red:** Write test `test_expected_json_parses_must_find_and_must_not_flag`
   - Parse expected.json with `must_find` (id, severity, description, location) and `must_not_flag` (id, description, optional location)
   - Assert correct deserialization

4. **Green:** Implement types and loader

**Files:**
- Create: `src/cmd/autoresearch/benchmarks.rs`

**Must-haves:**
- Artifact: `benchmarks.rs` with `BenchmarkCase`, `BenchmarkSuite`, `Expected`, `ExpectedFinding`
- Truth: Loader discovers all benchmark directories for a specialist type
- Truth: expected.json deserializes correctly including optional location in must_not_flag

---

#### T05a: Create security benchmark cases (5 cases)

**Source commits:**
1. `d9fcb61^` — TOCTOU race in `create_issue` (position query outside transaction)
2. `0c90928^` — insert + last_insert_rowid without transaction wrapper
3. `7272cee^` — production unwraps that can panic on bad input
4. `d6cbd2c^` — code using vulnerable dependency patterns
5. Synthetic — command injection via unsanitized input in shell exec

**For each case:**
1. Extract "before" code via `git show <commit>^:<file>` (or write synthetic)
2. Write `context.md` describing what the code does
3. Write `expected.json` with `must_find` (known issues) and `must_not_flag` (correct patterns)

**Must-haves:**
- Artifact: 5 dirs under `.forge/autoresearch/benchmarks/security/`
- Truth: Each `expected.json` has at least 1 `must_find` entry

---

#### T05b: Create architecture benchmark cases (5 cases)

**Source commits:**
1. `cf3470a^` — 3,163-line god file (pipeline.rs before split)
2. `e71c83c^` — monolithic db.rs before module split
3. `96c6b63^` — tight coupling to rusqlite before abstraction
4. Synthetic — circular dependency between two modules
5. Synthetic — SOLID violation (god function handling multiple concerns)

**Must-haves:**
- Artifact: 5 dirs under `.forge/autoresearch/benchmarks/architecture/`
- Truth: Each `expected.json` has at least 1 `must_find` entry

---

#### T05c: Create performance benchmark cases (5 cases)

**Source commits:**
1. `06cd7fe^` — synchronous event batching with lock_sync
2. `54b9a63^` — non-shared connection causing DB isolation issues
3. Synthetic — N+1 query pattern in a loop
4. Synthetic — blocking `.unwrap()` in async context
5. Synthetic — unnecessary cloning in hot path

**Must-haves:**
- Artifact: 5 dirs under `.forge/autoresearch/benchmarks/performance/`
- Truth: Each `expected.json` has at least 1 `must_find` entry

---

#### T05d: Create simplicity benchmark cases (5 cases)

**Source commits:**
1. `daa279b^` — over-complex ResolutionMode with 3 unused variants
2. `05b9ffd^` — dead Escalate code path never triggered
3. `9aa7285^` — Strict permission mode mapped to Standard (dead code)
4. `7692201^` — overly verbose let-chain patterns
5. Synthetic — premature abstraction (generic trait for single impl)

**Must-haves:**
- Artifact: 5 dirs under `.forge/autoresearch/benchmarks/simplicity/`
- Truth: Each `expected.json` has at least 1 `must_find` entry

---

#### T06: FindingMatcher + Scorer

**TDD sequence:**

1. **Red:** Write test `test_finding_matcher_detects_must_find_by_location`
   - Specialist finding at line 44, must_find at "line 42-45"
   - Assert: matched (within ±5 line tolerance)

2. **Red:** Write test `test_finding_matcher_detects_must_not_flag_hit`
   - Specialist flagged item matching a must_not_flag description
   - Assert: classified as false positive

3. **Red:** Write test `test_recall_all_found`
   - Specialist found all 3 must_find items → recall = 1.0

4. **Red:** Write test `test_recall_partial`
   - Specialist found 1 of 3 must_find items → recall ≈ 0.333

5. **Red:** Write test `test_precision_no_false_positives`
   - Specialist found 2 items, both in must_find → precision = 1.0

6. **Red:** Write test `test_precision_with_must_not_flag_hit`
   - Specialist flagged a must_not_flag item → precision < 1.0

7. **Red:** Write test `test_composite_score_weighted`
   - recall=0.8, precision=0.9, actionability=0.7
   - Assert score = 0.4*0.8 + 0.3*0.9 + 0.3*0.7 = 0.80

8. **Red:** Write test `test_edge_case_no_findings_no_must_find`
   - 0 findings, 0 must_find → recall=1.0 (vacuous truth), precision=1.0

9. **Red:** Write test `test_edge_case_no_findings_with_must_find`
   - 0 findings, 3 must_find → recall=0.0, precision=1.0

10. **Green:** Implement `FindingMatcher` and `Scorer`

**Files:**
- Create: `src/cmd/autoresearch/scorer.rs`

**Must-haves:**
- Artifact: `scorer.rs` with `FindingMatcher`, `Scorer`, `SpecialistScore`
- Truth: Composite formula is `0.4 * recall + 0.3 * precision + 0.3 * actionability`
- Truth: Matching uses location tolerance (±5 lines) and keyword overlap

---

#### T06b: BenchmarkRunner — invoke specialist via Claude CLI

**TDD sequence:**

1. **Red:** Write test `test_runner_builds_correct_claude_command`
   - Provide PromptConfig + BenchmarkCase
   - Assert command args include `-p`, `--print`, `--output-format json`

2. **Red:** Write test `test_runner_parses_specialist_json_output`
   - Provide sample specialist JSON response (verdict, findings array)
   - Assert `BenchmarkResult` populated correctly

3. **Red:** Write test `test_runner_handles_unparseable_output`
   - Provide garbage output from Claude
   - Assert `BenchmarkResult` has error status, empty findings

4. **Green:** Implement `BenchmarkRunner::run()`

**Files:**
- Create: `src/cmd/autoresearch/runner.rs`

**Must-haves:**
- Artifact: `runner.rs` with `BenchmarkRunner`, `BenchmarkResult`
- Truth: Specialist JSON output parsed into structured findings
- Truth: Unparseable output returns error result (no panic)

---

## Slice S03: Cross-Model Judge (GPT 5.4)

**Demo sentence:** "After this, specialist outputs are scored by GPT 5.4 via Codex CLI with actionability ratings and novel finding classification."

### Boundary Map
- **Produces:** `Judge` struct with `evaluate(specialist_output, expected, code) -> JudgeResult`
- **Produces:** `JudgeResult` with `classifications: Vec<Classification>`, `actionability_scores: Vec<ActionabilityScore>`
- **Consumes:** `BenchmarkResult` from S02
- **Consumes:** Codex CLI (`codex --model gpt-5.4-xhigh`)

### Tasks

#### T07: Judge prompt template + Codex CLI invocation

**TDD sequence:**

1. **Red:** Write test `test_judge_prompt_contains_all_required_sections`
   - Build judge prompt with sample code, expected.json, specialist output
   - Assert prompt contains "Code Under Review", "Ground Truth", "Specialist Output", "Tasks", "Output"

2. **Red:** Write test `test_parse_judge_response_valid_json`
   - Parse a sample GPT 5.4 response with classifications and actionability scores
   - Assert deserialization succeeds, fields populated

3. **Red:** Write test `test_parse_judge_response_malformed_falls_back`
   - Parse garbage response
   - Assert fallback: actionability=0.5, unknowns=true_positive

4. **Red:** Write test `test_judge_invocation_builds_correct_codex_command`
   - Mock the command builder
   - Assert args include `--model gpt-5.4-xhigh --quiet --json -p`

5. **Green:** Implement `Judge` struct, prompt builder, response parser

6. **Refactor:** Add retry logic (1 retry on failure)

**Files:**
- Create: `src/cmd/autoresearch/judge.rs`

**Must-haves:**
- Artifact: `judge.rs` with `Judge`, `JudgeResult`, `Classification`, `ActionabilityScore`
- Truth: Malformed responses trigger fallback scoring
- Truth: Codex CLI invoked with correct model and flags
- Truth: Retry once on failure, then fall back

---

#### T08: Wire judge into scorer for actionability + novel finding classification

**TDD sequence:**

1. **Red:** Write test `test_scorer_with_judge_computes_full_composite`
   - Provide specialist output + judge result (actionability scores + classifications)
   - Assert composite score uses judge's actionability ratings
   - Assert novel findings classified by judge affect precision

2. **Red:** Write test `test_scorer_without_judge_uses_neutral_actionability`
   - Provide specialist output without judge result
   - Assert actionability defaults to 0.5, novel findings default to true_positive

3. **Green:** Modify `Scorer::score()` to accept optional `JudgeResult`

**Files:**
- Modify: `src/cmd/autoresearch/scorer.rs`

**Must-haves:**
- Truth: Full composite score computed with all 3 dimensions
- Truth: Without judge, actionability=0.5 and novel findings assumed true positive
- Key Link: `scorer.rs` imports and uses `JudgeResult` from `judge.rs`

---

## Slice S04: Experiment Loop + CLI Command

**Demo sentence:** "After this, `forge autoresearch --budget 25 --tag mar11` runs the full autonomous optimization loop with git tracking and budget enforcement."

### Boundary Map
- **Produces:** `forge autoresearch` CLI command registered in `main.rs`
- **Produces:** `ExperimentLoop` struct orchestrating the full cycle
- **Produces:** `GitOps` for branch/commit/reset
- **Produces:** `BudgetTracker` for cost enforcement
- **Produces:** `ResultsLog` for TSV output
- **Consumes:** `PromptLoader` from S01, `BenchmarkSuite` + `Scorer` + `BenchmarkRunner` from S02, `Judge` from S03
- **Consumes:** Claude CLI for prompt mutation proposals
- **Consumes:** git2 for branch/commit/reset
- **Consumes:** Audit logger for tracking

### Tasks

#### T09: CLI registration + argument parsing

**TDD sequence:**

1. **Red:** Write test `test_autoresearch_args_parse_defaults`
   - Parse `["autoresearch"]` with clap
   - Assert budget=25, max_failures=5, specialist=None, dry_run=false

2. **Red:** Write test `test_autoresearch_args_parse_all_flags`
   - Parse with `--budget 10 --tag test --specialist security --dry-run --max-failures 3`
   - Assert all fields set correctly

3. **Red:** Write test `test_autoresearch_args_parse_results_flag`
   - Parse with `--results mar11`
   - Assert results_tag is Some("mar11")

4. **Green:** Add `Autoresearch` variant to `Commands` enum in `main.rs`

**Files:**
- Modify: `src/main.rs` (add Commands variant + match arm)
- Create: `src/cmd/autoresearch/mod.rs` (entry point)
- Modify: `src/cmd/mod.rs` (add module)

**Must-haves:**
- Artifact: `src/cmd/autoresearch/mod.rs`
- Truth: `forge autoresearch --help` shows all flags including `--results`
- Key Link: main.rs dispatches to `cmd::autoresearch::cmd_autoresearch`

---

#### T10: Budget tracker

**TDD sequence:**

1. **Red:** Write test `test_budget_tracker_starts_at_zero`
   - Create `BudgetTracker::new(25.0)`
   - Assert `remaining()` == 25.0, `spent()` == 0.0

2. **Red:** Write test `test_budget_tracker_records_spend`
   - Record $0.50 spend
   - Assert `remaining()` == 24.5

3. **Red:** Write test `test_budget_tracker_halts_when_exceeded`
   - Record $25.10 spend
   - Assert `can_continue()` == false

4. **Red:** Write test `test_budget_tracker_estimates_cost_from_tokens`
   - Estimate cost for 5000 input tokens + 2000 output tokens for Claude
   - Assert reasonable cost estimate (~$0.03-0.10 range per call)

5. **Red:** Write test `test_budget_tracker_restores_from_results_log`
   - Provide results.tsv with 5 experiments at $0.50 each
   - Assert `spent()` == 2.50 after restore

6. **Green:** Implement `BudgetTracker`

**Files:**
- Create: `src/cmd/autoresearch/budget.rs`

**Must-haves:**
- Artifact: `budget.rs` with `BudgetTracker`
- Truth: Loop halts when budget exceeded
- Truth: Budget can be restored from results.tsv for resume

---

#### T11: Experiment — single iteration logic

**TDD sequence:**

1. **Red:** Write test `test_single_experiment_returns_keep_on_improvement`
   - Mock: Claude returns prompt mutation, specialist scores 0.85 (above baseline 0.70)
   - Assert: `ExperimentOutcome::Keep` with score 0.85

2. **Red:** Write test `test_single_experiment_returns_discard_on_regression`
   - Mock: specialist scores 0.65 (below baseline 0.70)
   - Assert: `ExperimentOutcome::Discard` with score 0.65

3. **Red:** Write test `test_single_experiment_returns_discard_on_crash`
   - Mock: specialist returns unparseable output
   - Assert: `ExperimentOutcome::Discard` with score 0.0

4. **Red:** Write test `test_experiment_records_budget_spent`
   - Run experiment with mock Claude + Codex
   - Assert: `BudgetTracker` spend increased

5. **Green:** Implement `Experiment::run()` — returns `ExperimentOutcome` (does NOT handle git; caller does)

**Files:**
- Create: `src/cmd/autoresearch/experiment.rs`

**Must-haves:**
- Truth: Improvement returns `Keep` with new score
- Truth: Regression returns `Discard` with old score as baseline
- Truth: Crash returns `Discard` with score 0.0
- Truth: Budget updated per experiment (experiment does not manage git)

---

#### T12: Results TSV logging + display

**TDD sequence:**

1. **Red:** Write test `test_results_tsv_header_written_on_create`
   - Create new results file
   - Assert header: `commit\tscore\trecall\tprecision\tactionability\tstatus\tdescription`

2. **Red:** Write test `test_results_tsv_append_experiment`
   - Append experiment result
   - Assert row format correct, tab-separated

3. **Red:** Write test `test_results_tsv_parse_existing`
   - Load results with 3 entries
   - Assert parsed correctly, can determine best score and last kept commit

4. **Red:** Write test `test_results_display_summary`
   - Load results with 5 entries (3 keep, 2 discard)
   - Assert display shows best score, improvement over baseline, experiment count

5. **Green:** Implement `ResultsLog` with `new()`, `append()`, `load()`, `display_summary()`

**Files:**
- Create: `src/cmd/autoresearch/results.rs`

**Must-haves:**
- Artifact: `results.rs` with `ResultsLog`, `ResultEntry`
- Truth: TSV format is tab-separated, 7 columns
- Truth: `--results <tag>` displays formatted summary

---

#### T13: Full loop orchestration with convergence, budget, resume, and audit

**TDD sequence:**

1. **Red:** Write test `test_loop_stops_after_max_consecutive_failures`
   - Mock: 5 consecutive non-improvements for security specialist
   - Assert: loop moves to next specialist (performance)

2. **Red:** Write test `test_loop_stops_on_budget_exceeded`
   - Mock: budget tracker says can_continue=false after 3rd experiment
   - Assert: loop halts, prints summary with experiments run and budget spent

3. **Red:** Write test `test_loop_cycles_through_specialists`
   - Mock: security converges after 3 experiments, performance after 4
   - Assert: both specialists optimized, separate results logged for each

4. **Red:** Write test `test_loop_resume_restores_budget`
   - Provide results.tsv with 5 existing entries totaling $2.50 spent
   - Assert: budget tracker starts at $2.50 spent, not $0

5. **Red:** Write test `test_loop_resume_picks_correct_specialist`
   - Provide results showing security fully converged, performance in progress
   - Assert: loop resumes at performance specialist, not security

6. **Red:** Write test `test_loop_resume_uses_correct_git_branch`
   - Provide existing autoresearch branch with commits
   - Assert: loop checks out the branch and continues from HEAD

7. **Red:** Write test `test_loop_keep_commits_and_discard_resets`
   - Run 3 experiments: keep, discard, keep
   - Assert: git history has 2 kept commits, discard was reset

8. **Red:** Write test `test_loop_emits_audit_log_per_experiment`
   - Run 2 experiments
   - Assert: audit log has 2 entries with experiment details

9. **Green:** Implement `ExperimentLoop::run()` composing all components

**Files:**
- Create: `src/cmd/autoresearch/loop_runner.rs`

**Must-haves:**
- Truth: Convergence stop after N consecutive failures (configurable, default 5)
- Truth: Budget stop when ceiling reached
- Truth: Resume restores budget spent, picks correct specialist, uses correct branch
- Truth: Keep advances branch, discard resets to last kept commit
- Truth: Each experiment emits an audit log entry
- Truth: Summary printed on completion (best scores, improvements, budget remaining)

---

#### T14: Git integration — branch, commit, keep/discard

**TDD sequence:**

1. **Red:** Write test `test_create_autoresearch_branch`
   - Create branch `autoresearch/test-tag`
   - Assert branch exists and is checked out

2. **Red:** Write test `test_commit_experiment`
   - Modify a prompt file, commit with "experiment: description"
   - Assert commit exists with correct message

3. **Red:** Write test `test_discard_resets_to_previous_kept`
   - Make 2 commits, discard second
   - Assert HEAD is at first commit

4. **Red:** Write test `test_discard_restores_working_directory`
   - Modify prompt file, commit, then discard
   - Assert prompt file content matches pre-experiment state

5. **Red:** Write test `test_checkout_existing_branch_for_resume`
   - Create branch, make commits, switch away, then checkout for resume
   - Assert: on correct branch at correct HEAD

6. **Green:** Implement `GitOps` wrapping forge's `GitTracker`

**Files:**
- Create: `src/cmd/autoresearch/git_ops.rs`

**Must-haves:**
- Truth: Branch created with `autoresearch/<tag>` naming
- Truth: Keep advances branch, discard resets to last kept commit
- Truth: Discard restores prompt file to pre-experiment state
- Truth: Resume checks out existing branch at correct HEAD

---

## Task Dependency Graph

```
S01: Prompt Extraction
  T01 (PromptConfig + Loader)
   └── T02 (Extract prompt files)
        └── T03 (Wire into dispatcher)

S02: Benchmark Suite + Runner
  T04 (Benchmark types + loader) ──┐
  T05a (Security benchmarks)       │
  T05b (Architecture benchmarks)   ├── T06 (FindingMatcher + Scorer)
  T05c (Performance benchmarks)    │    └── T06b (BenchmarkRunner)
  T05d (Simplicity benchmarks) ────┘

S03: Cross-Model Judge
  T07 (Judge + Codex CLI)
   └── T08 (Wire judge into scorer) ← depends on T06

S04: Experiment Loop + CLI
  T09 (CLI registration)
  T10 (Budget tracker)
  T11 (Single experiment) ← depends on T06b, T08
  T12 (Results TSV + display)
  T14 (Git integration)
   └── T13 (Full loop) ← depends on T09, T10, T11, T12, T14, T03
```

## Execution Order

**Wave 1 (parallel):** T01, T04, T07, T09, T10
**Wave 2 (parallel):** T02, T05a, T05b, T05c, T05d, T12, T14
**Wave 3 (parallel):** T03, T06, T06b, T08
**Wave 4 (parallel):** T11
**Wave 5 (sequential):** T13

## Estimated Token Budget

| Phase | Claude calls | Codex calls | Est. cost |
|-------|-------------|-------------|-----------|
| Development (T01-T14) | ~150 | 0 | ~$10-15 |
| Benchmark creation (T05a-d) | ~30 | 0 | ~$2-3 |
| Dry-run validation | ~20 | ~20 | ~$2-3 |
| **Total pre-loop** | | | **~$14-21** |
| **Remaining for experiments** | | | **~$4-11** |

At ~$0.50/experiment, that's 8-22 experiments for actual optimization. If development comes in under budget, more experiments are possible.

**Per-experiment cost breakdown:**
- 5 Claude specialist calls: ~2K input + ~1K output each ≈ $0.15-0.25
- 5 Codex judge calls: ~1.5K input + ~500 output each ≈ $0.10-0.20
- 1 Claude mutation proposal call: ~3K input + ~1K output ≈ $0.05-0.10
- **Total per experiment: ~$0.30-0.55**
