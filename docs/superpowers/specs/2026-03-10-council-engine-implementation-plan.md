# LLM Council Engine - TDD Implementation Plan

**Date:** 2026-03-10
**Milestone:** M1 - Council Engine v1 (Single Shippable Release)
**Summary:** Multi-LLM council that runs two workers in parallel, performs anonymized peer review, and synthesizes results via a chairman -- as a drop-in replacement for single-engine `run_iteration` when `[council]` is enabled in `forge.toml`.

---

## Slice S01: Types & Config Foundation

**Demo:** "After this, the user can add a `[council]` section to `forge.toml` and `cargo build` succeeds with the new types parsed."

### Boundary Map

**Produces:**
- `src/council/types.rs` — `WorkerResult`, `ReviewVerdict`, `ReviewResult`, `MergeOutcome`, `CouncilPhaseResult`, `CouncilError`
- `src/council/config.rs` — `CouncilConfig`, `WorkerConfig`
- `src/council/mod.rs` — public re-exports
- Modified `src/forge_config.rs` — `ForgeToml.council: Option<CouncilSection>`
- Modified `src/lib.rs` — `pub mod council;`

**Consumes:**
- `src/phase.rs::Phase`
- `src/forge_config.rs::ForgeToml`, `ForgeConfig`
- `serde::{Serialize, Deserialize}`, `anyhow::Result`

### Tasks

#### Task T01: Council types module

**TDD Approach:** Write tests for type construction, serialization round-trips, and enum variant behavior first.

**Must-haves:**
- Truths:
  - `WorkerResult` holds a worker name, diff text, exit code, and optional session metadata
  - `ReviewVerdict` is an enum: `Approve`, `RequestChanges(String)`, `Abstain`
  - `ReviewResult` holds reviewer name, candidate label, verdict, and a structured comment
  - `MergeOutcome` is an enum: `Clean(String)` (merged diff), `Conflict(Vec<String>)` (conflict paths), `Failure(String)`
  - `CouncilPhaseResult` holds the winning diff, all worker results, all review results, merge attempts count, and a final `MergeOutcome`
  - `CouncilError` is a `thiserror` enum with variants: `WorkerFailed`, `ReviewFailed`, `MergeFailed`, `BudgetExhausted`, `EscalationRequired`
  - All types derive `Debug, Clone` and domain-appropriate `Serialize, Deserialize`
- Artifacts: `src/council/types.rs`
- Key Links: `use serde::{Serialize, Deserialize}; use thiserror::Error;`

**Tests to write first:**
```
test_worker_result_construction
test_worker_result_serialization_roundtrip
test_review_verdict_variants
test_review_result_approve_verdict
test_review_result_request_changes_verdict
test_merge_outcome_clean
test_merge_outcome_conflict
test_merge_outcome_failure
test_council_phase_result_construction
test_council_error_display_messages
test_council_error_is_send_sync
```

**Then implement:**
1. Create `src/council/mod.rs` with `pub mod types;` and re-exports
2. Create `src/council/types.rs` with all structs and enums
3. Add `pub mod council;` to `src/lib.rs`

**Verification:** `cargo test --lib council::types`

---

#### Task T02: Council config parsing

**TDD Approach:** Write tests that parse TOML fragments into `CouncilConfig`, verify defaults, and validate invalid configs.

**Must-haves:**
- Truths:
  - `CouncilConfig` has: `enabled: bool` (default false), `chairman_model: String`, `chairman_reasoning_effort: String` (default "high"), `chairman_retry_budget: u32` (default 3), `anonymize_reviews: bool` (default true), `workers: HashMap<String, WorkerConfig>`
  - `WorkerConfig` has: `cmd: String`, `role: String` (default "worker"), `model: Option<String>`, `reasoning_effort: Option<String>`, `sandbox: Option<String>`
  - A `CouncilConfig::default()` returns `enabled: false` with empty workers
  - Config round-trips through `toml::to_string` / `toml::from_str`
  - Missing `[council]` in `forge.toml` results in `None` (not an error)
- Artifacts: `src/council/config.rs`
- Key Links: `use serde::{Serialize, Deserialize}; use std::collections::HashMap;`

**Tests to write first:**
```
test_council_config_default
test_council_config_enabled_false_by_default
test_council_config_parse_full_toml
test_council_config_parse_minimal_toml
test_worker_config_defaults
test_worker_config_with_all_fields
test_council_config_serialization_roundtrip
test_council_config_missing_workers_is_valid
test_council_config_chairman_retry_budget_default
test_council_config_anonymize_reviews_default_true
```

**Then implement:**
1. Create `src/council/config.rs` with `CouncilConfig`, `WorkerConfig`
2. Add `pub mod config;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::config`

---

#### Task T03: Integrate council config into ForgeToml

**TDD Approach:** Write tests that parse a full `forge.toml` with a `[council]` section and verify it loads through the existing `ForgeConfig` pipeline.

**Must-haves:**
- Truths:
  - `ForgeToml` gains `pub council: Option<CouncilConfig>` (serde default `None`)
  - A `forge.toml` without `[council]` still parses correctly (backward compatible)
  - A `forge.toml` with `[council]` populates the field
  - `ForgeConfig` exposes `pub fn council_config(&self) -> Option<&CouncilConfig>`
  - All existing `forge_config` tests continue to pass
- Artifacts: Modified `src/forge_config.rs`, modified `src/council/mod.rs`
- Key Links: `use crate::council::config::CouncilConfig;` in `forge_config.rs`

**Tests to write first (in `src/forge_config.rs` test module):**
```
test_forge_toml_parse_without_council_section
test_forge_toml_parse_with_council_section
test_forge_toml_council_workers_parsed
test_forge_toml_council_defaults_when_partial
test_forge_config_council_config_accessor
```

**Then implement:**
1. Add `use crate::council::config::CouncilConfig;` to `forge_config.rs`
2. Add `pub council: Option<CouncilConfig>` field to `ForgeToml` with `#[serde(default)]`
3. Add `council_config()` method to `ForgeConfig`
4. Re-export `CouncilConfig` from `src/council/mod.rs`

**Verification:** `cargo test --lib forge_config` (all existing + new tests pass)

---

## Slice S02: Worker Trait & Mock Workers

**Demo:** "After this, the user can instantiate a MockWorker that returns deterministic results, enabling all downstream testing without real CLI calls."

### Boundary Map

**Produces:**
- `src/council/worker.rs` — `Worker` trait, `MockWorker` struct
- Extended `src/council/mod.rs` re-exports

**Consumes:**
- `src/council/types.rs::WorkerResult`, `ReviewResult`, `ReviewVerdict`
- `src/phase.rs::Phase`
- `async_trait`
- `std::path::Path`

### Tasks

#### Task T04: Worker trait and MockWorker

**TDD Approach:** Write tests exercising MockWorker's `execute()` and `review()` methods, then implement the trait and mock.

**Must-haves:**
- Truths:
  - `Worker` trait is `async_trait` with `Send + Sync`
  - `Worker::name(&self) -> &str` returns the worker's identifier
  - `Worker::execute(&self, phase: &Phase, prompt: &str, worktree_path: &Path) -> Result<WorkerResult>` produces a diff
  - `Worker::review(&self, phase: &Phase, diff: &str, candidate_label: &str) -> Result<ReviewResult>` reviews a diff
  - `MockWorker` stores configurable responses: `execute_result: WorkerResult`, `review_result: ReviewResult`
  - `MockWorker` tracks call counts via `AtomicU32` (thread-safe for parallel tests)
  - `MockWorker::new(name: &str)` creates a mock with sensible defaults (approve verdict, trivial diff)
  - `MockWorker::with_execute_result(mut self, result: WorkerResult) -> Self` builder pattern
  - `MockWorker::with_review_result(mut self, result: ReviewResult) -> Self` builder pattern
  - `MockWorker::execute_count(&self) -> u32` and `review_count(&self) -> u32` for call tracking
- Artifacts: `src/council/worker.rs`
- Key Links: `use async_trait::async_trait; use crate::council::types::*; use crate::phase::Phase;`

**Tests to write first:**
```
test_mock_worker_name
test_mock_worker_default_execute_returns_ok
test_mock_worker_default_review_returns_approve
test_mock_worker_custom_execute_result
test_mock_worker_custom_review_result
test_mock_worker_execute_count_increments
test_mock_worker_review_count_increments
test_mock_worker_is_send_sync
test_worker_trait_object_safety
```

**Then implement:**
1. Create `src/council/worker.rs` with the `Worker` trait
2. Implement `MockWorker` with `Arc<AtomicU32>` counters
3. Add `pub mod worker;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::worker`

---

## Slice S03: Git Worktree Management (merge.rs)

**Demo:** "After this, the system can create isolated git worktrees, capture diffs, parse git-format patches, apply patches, and detect merge conflicts -- all tested against real temporary git repos."

### Boundary Map

**Produces:**
- `src/council/merge.rs` — `WorktreeManager`, `PatchSet`, `apply_patch`, `parse_diff`, `detect_conflicts`

**Consumes:**
- `git2` crate (already in Cargo.toml)
- `src/council/types.rs::MergeOutcome`
- `tempfile` crate (for test worktrees)
- `std::path::Path`, `std::process::Command` (for `git diff` / `git apply`)

### Tasks

#### Task T05: Worktree creation and cleanup

**TDD Approach:** Write tests that create worktrees in tempdir repos and verify filesystem state.

**Must-haves:**
- Truths:
  - `WorktreeManager::new(repo_path: &Path) -> Result<Self>` opens a git2 Repository
  - `WorktreeManager::create_worktree(name: &str) -> Result<PathBuf>` creates a linked worktree at `<repo>/.forge/council-worktrees/<name>` and returns the path
  - `WorktreeManager::remove_worktree(name: &str) -> Result<()>` prunes the worktree and deletes the directory
  - `WorktreeManager::cleanup_all() -> Result<()>` removes all council worktrees
  - Created worktrees have a copy of HEAD at creation time
  - After removal, the worktree directory no longer exists
- Artifacts: `src/council/merge.rs`
- Key Links: `use git2::Repository; use std::path::{Path, PathBuf}; use anyhow::{Context, Result};`

**Tests to write first:**
```
test_worktree_manager_new_valid_repo
test_worktree_manager_new_invalid_path_errors
test_create_worktree_returns_valid_path
test_create_worktree_directory_exists
test_create_worktree_has_files_from_head
test_remove_worktree_cleans_directory
test_remove_worktree_nonexistent_is_noop
test_cleanup_all_removes_everything
test_two_worktrees_are_independent
```

**Then implement:**
1. Create `src/council/merge.rs` with `WorktreeManager`
2. Implement worktree creation using `git2::Repository::worktree` or `std::process::Command` git worktree add
3. Add `pub mod merge;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::merge::tests::test_worktree`

---

#### Task T06: Diff generation and patch parsing

**TDD Approach:** Write tests that make changes in worktrees and verify diff output structure, then write tests for parsing git-format patches.

**Must-haves:**
- Truths:
  - `WorktreeManager::generate_diff(worktree_path: &Path) -> Result<String>` returns unified diff of all changes against the base commit
  - `PatchSet::parse(diff: &str) -> Result<PatchSet>` parses a unified diff into file-level hunks
  - `PatchSet` exposes `files_changed() -> Vec<&str>` listing affected paths
  - `PatchSet` exposes `is_empty() -> bool`
  - Empty worktree (no changes) produces an empty diff
  - Multi-file changes are captured correctly
- Artifacts: `src/council/merge.rs` (extended)
- Key Links: Extends `WorktreeManager` and adds `PatchSet` struct

**Tests to write first:**
```
test_generate_diff_empty_worktree
test_generate_diff_single_file_added
test_generate_diff_single_file_modified
test_generate_diff_multiple_files
test_patch_set_parse_empty
test_patch_set_parse_single_file
test_patch_set_parse_multi_file
test_patch_set_files_changed
test_patch_set_is_empty_true
test_patch_set_is_empty_false
```

**Then implement:**
1. Add `generate_diff()` to `WorktreeManager`
2. Add `PatchSet` struct with `parse()`, `files_changed()`, `is_empty()`

**Verification:** `cargo test --lib council::merge::tests::test_generate_diff && cargo test --lib council::merge::tests::test_patch_set`

---

#### Task T07: Patch application and conflict detection

**TDD Approach:** Write tests that apply clean patches and deliberately conflicting patches, then verify outcomes.

**Must-haves:**
- Truths:
  - `apply_patch(repo_path: &Path, patch: &str) -> Result<MergeOutcome>` applies a unified diff to a working tree
  - Clean apply returns `MergeOutcome::Clean(applied_diff)`
  - Conflicting patches return `MergeOutcome::Conflict(conflicting_paths)`
  - Invalid patch text returns `MergeOutcome::Failure(error_message)`
  - `detect_conflicts(patch_a: &str, patch_b: &str) -> Vec<String>` returns paths where both patches touch the same file regions
- Artifacts: `src/council/merge.rs` (extended)
- Key Links: `use crate::council::types::MergeOutcome;`

**Tests to write first:**
```
test_apply_patch_clean_success
test_apply_patch_conflict_detected
test_apply_patch_invalid_patch_fails
test_apply_patch_empty_patch_noop
test_detect_conflicts_no_overlap
test_detect_conflicts_same_file_different_hunks
test_detect_conflicts_same_file_overlapping_hunks
test_detect_conflicts_empty_patches
test_apply_then_verify_file_contents
```

**Then implement:**
1. Implement `apply_patch()` using `git apply --check` then `git apply`
2. Implement `detect_conflicts()` by parsing file paths and hunk ranges from both patches

**Verification:** `cargo test --lib council::merge::tests::test_apply && cargo test --lib council::merge::tests::test_detect`

---

## Slice S04: Prompt Templates (prompts.rs)

**Demo:** "After this, the system generates deterministic, anonymized review prompts and chairman synthesis prompts with correct label substitution."

### Boundary Map

**Produces:**
- `src/council/prompts.rs` — `review_prompt()`, `chairman_synthesis_prompt()`, `anonymize_label()`, `generate_label_pair()`

**Consumes:**
- `src/phase.rs::Phase`
- `src/council/types.rs::ReviewResult`

### Tasks

#### Task T08: Prompt templates and anonymization

**TDD Approach:** Write tests for label generation, anonymization, and prompt output structure.

**Must-haves:**
- Truths:
  - `generate_label_pair() -> (String, String)` returns two randomized labels (e.g., "Alpha" / "Bravo") from a fixed pool of 10+ label pairs; never returns the same label for both
  - `anonymize_label(worker_name: &str, label_map: &HashMap<String, String>) -> &str` maps worker name to anonymized label
  - `review_prompt(phase: &Phase, diff: &str, candidate_label: &str) -> String` produces a prompt containing: the phase description, the candidate label, the diff, and instructions to output structured review JSON
  - `chairman_synthesis_prompt(phase: &Phase, diffs: &[(&str, &str)], reviews: &[ReviewResult]) -> String` produces a prompt containing: both labeled diffs, all review feedback, and instructions to produce a merged git-format patch
  - Review prompt does NOT contain real worker names when anonymization is on
  - Chairman prompt includes the retry budget context
- Artifacts: `src/council/prompts.rs`
- Key Links: `use crate::phase::Phase; use crate::council::types::ReviewResult; use std::collections::HashMap;`

**Tests to write first:**
```
test_generate_label_pair_returns_two_different_labels
test_generate_label_pair_labels_from_known_pool
test_anonymize_label_maps_correctly
test_anonymize_label_unknown_worker_panics_or_errors
test_review_prompt_contains_phase_name
test_review_prompt_contains_candidate_label
test_review_prompt_contains_diff_content
test_review_prompt_does_not_contain_worker_name
test_review_prompt_requests_structured_json
test_chairman_synthesis_prompt_contains_both_diffs
test_chairman_synthesis_prompt_contains_review_feedback
test_chairman_synthesis_prompt_requests_git_format_patch
test_chairman_synthesis_prompt_mentions_retry_budget
```

**Then implement:**
1. Create `src/council/prompts.rs` with all functions
2. Define label pool as a const array
3. Add `pub mod prompts;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::prompts`

---

## Slice S05: Peer Review Engine (reviewer.rs)

**Demo:** "After this, the system can orchestrate two mock workers reviewing each other's diffs concurrently, with anonymized labels and structured output."

### Boundary Map

**Produces:**
- `src/council/reviewer.rs` — `PeerReviewEngine`, `ReviewRound`

**Consumes:**
- `src/council/worker.rs::Worker` (trait objects)
- `src/council/types.rs::ReviewResult`, `ReviewVerdict`, `WorkerResult`
- `src/council/prompts.rs::review_prompt`, `generate_label_pair`, `anonymize_label`
- `src/council/config.rs::CouncilConfig`
- `src/phase.rs::Phase`
- `tokio` (for concurrent reviews)

### Tasks

#### Task T09: Peer review orchestration

**TDD Approach:** Write tests with MockWorkers verifying review dispatch, anonymization, and concurrent execution.

**Must-haves:**
- Truths:
  - `PeerReviewEngine::new(config: &CouncilConfig) -> Self`
  - `PeerReviewEngine::run_reviews(&self, phase: &Phase, workers: &[Arc<dyn Worker>], results: &[WorkerResult]) -> Result<ReviewRound>` orchestrates peer review
  - Each worker reviews the OTHER worker's diff (not its own)
  - When `anonymize_reviews` is true, worker names are replaced with labels in prompts
  - Reviews run concurrently via `tokio::join!` or `futures::join_all`
  - `ReviewRound` holds: `reviews: Vec<ReviewResult>`, `label_map: HashMap<String, String>`
  - If a worker's review call fails, the error is captured but the other review still completes (partial failure tolerance)
- Artifacts: `src/council/reviewer.rs`
- Key Links: `use crate::council::{worker::Worker, types::*, prompts::*, config::CouncilConfig}; use std::sync::Arc;`

**Tests to write first:**
```
test_peer_review_two_workers_each_reviews_other
test_peer_review_anonymization_enabled
test_peer_review_anonymization_disabled
test_peer_review_concurrent_execution
test_peer_review_label_map_populated
test_peer_review_partial_failure_one_worker_errors
test_peer_review_both_approve
test_peer_review_one_requests_changes
test_peer_review_both_request_changes
test_peer_review_round_structure
```

**Then implement:**
1. Create `src/council/reviewer.rs` with `PeerReviewEngine` and `ReviewRound`
2. Implement review dispatch with label mapping
3. Add `pub mod reviewer;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::reviewer`

---

## Slice S06: Chairman Synthesis (chairman.rs)

**Demo:** "After this, the system can take two worker diffs and their reviews, invoke a chairman LLM to produce a merged patch, and handle the full retry cascade (retry -> winner-takes-all -> escalate)."

### Boundary Map

**Produces:**
- `src/council/chairman.rs` — `Chairman`, `SynthesisResult`, `ChairmanDecision`

**Consumes:**
- `src/council/types.rs::WorkerResult`, `ReviewResult`, `MergeOutcome`, `CouncilError`
- `src/council/merge.rs::apply_patch`, `PatchSet`
- `src/council/prompts.rs::chairman_synthesis_prompt`
- `src/council/config.rs::CouncilConfig`
- `src/council/worker.rs::Worker` (chairman wraps a Worker for LLM calls)
- `src/phase.rs::Phase`

### Tasks

#### Task T10: Chairman synthesis and retry cascade

**TDD Approach:** Write tests for each decision path (clean merge, retry, winner-takes-all, escalation) using MockWorkers.

**Must-haves:**
- Truths:
  - `ChairmanDecision` is an enum: `MergedPatch(String)`, `WinnerTakesAll { winner: String, reason: String }`, `Escalate(String)`
  - `Chairman::new(config: &CouncilConfig, chairman_worker: Arc<dyn Worker>) -> Self`
  - `Chairman::synthesize(&self, phase: &Phase, worker_results: &[WorkerResult], reviews: &[ReviewResult], repo_path: &Path) -> Result<SynthesisResult>`
  - Retry cascade: on `MergeOutcome::Conflict`, chairman retries up to `chairman_retry_budget` times with conflict feedback
  - After retry budget exhaustion, falls back to winner-takes-all: picks the diff with the better review verdict
  - If winner-takes-all also fails (both rejected), returns `CouncilError::EscalationRequired`
  - `SynthesisResult` holds: `decision: ChairmanDecision`, `merged_diff: Option<String>`, `attempts: u32`, `merge_outcome: MergeOutcome`
  - Winner-takes-all selection logic: `Approve` beats `RequestChanges` beats `Abstain`; tie-break goes to first worker alphabetically
- Artifacts: `src/council/chairman.rs`
- Key Links: `use crate::council::{types::*, merge::*, prompts::*, config::CouncilConfig, worker::Worker};`

**Tests to write first:**
```
test_chairman_clean_merge_first_attempt
test_chairman_retry_on_conflict
test_chairman_exhausts_retry_budget
test_chairman_winner_takes_all_after_retries
test_chairman_winner_selection_approve_beats_request_changes
test_chairman_winner_selection_tie_break_alphabetical
test_chairman_escalation_when_both_rejected
test_chairman_synthesis_result_tracks_attempts
test_chairman_synthesis_result_holds_merged_diff
test_chairman_uses_chairman_model_from_config
test_chairman_passes_conflict_feedback_to_retry_prompt
```

**Then implement:**
1. Create `src/council/chairman.rs` with `Chairman`, `ChairmanDecision`, `SynthesisResult`
2. Implement the three-tier cascade
3. Add `pub mod chairman;` to `src/council/mod.rs`

**Verification:** `cargo test --lib council::chairman`

---

## Slice S07: Council Engine (engine.rs + mod.rs)

**Demo:** "After this, the user can call `CouncilEngine::run_phase()` which orchestrates all three stages -- parallel implementation, peer review, chairman synthesis -- and returns a `CouncilPhaseResult` compatible with the existing `PhaseResult` pipeline."

### Boundary Map

**Produces:**
- `src/council/engine.rs` — `CouncilEngine`
- Updated `src/council/mod.rs` — full public API

**Consumes:**
- `src/council/worker.rs::Worker`
- `src/council/reviewer.rs::PeerReviewEngine`
- `src/council/chairman.rs::Chairman`
- `src/council/merge.rs::WorktreeManager`
- `src/council/config.rs::CouncilConfig`
- `src/council/types.rs::*`
- `src/phase.rs::Phase`
- `tokio` (for parallel worker execution)
- `tracing` (for structured logging)

### Tasks

#### Task T11: Three-stage orchestration engine

**TDD Approach:** Write integration tests with MockWorkers verifying the full three-stage flow, then implement.

**Must-haves:**
- Truths:
  - `CouncilEngine::new(config: CouncilConfig, workers: Vec<Arc<dyn Worker>>, chairman_worker: Arc<dyn Worker>, repo_path: PathBuf) -> Self`
  - `CouncilEngine::run_phase(&self, phase: &Phase, prompt: &str) -> Result<CouncilPhaseResult>` executes the full pipeline
  - Stage 1: Creates worktrees, runs all workers in parallel via `tokio::join_all`, collects `WorkerResult`s
  - Stage 2: Runs `PeerReviewEngine::run_reviews()` for cross-review
  - Stage 3: Runs `Chairman::synthesize()` for merge/resolution
  - Worktrees are cleaned up even on error (RAII guard or explicit cleanup in finally block)
  - The engine logs each stage transition via `tracing::info!`
  - If all workers fail in Stage 1, returns `CouncilError::WorkerFailed` without entering review
  - If only one worker succeeds, skips review and uses that worker's output directly
- Artifacts: `src/council/engine.rs`
- Key Links: `use crate::council::{worker::Worker, reviewer::PeerReviewEngine, chairman::Chairman, merge::WorktreeManager, config::CouncilConfig, types::*};`

**Tests to write first:**
```
test_engine_full_flow_both_workers_succeed
test_engine_full_flow_one_worker_fails_uses_other
test_engine_all_workers_fail_returns_error
test_engine_creates_worktrees_for_each_worker
test_engine_cleans_up_worktrees_on_success
test_engine_cleans_up_worktrees_on_error
test_engine_parallel_worker_execution
test_engine_passes_reviews_to_chairman
test_engine_result_contains_all_worker_results
test_engine_result_contains_all_review_results
test_engine_single_worker_skips_review
test_engine_returns_council_phase_result
```

**Then implement:**
1. Create `src/council/engine.rs` with `CouncilEngine`
2. Implement the three-stage pipeline with proper error handling
3. Add `pub mod engine;` to `src/council/mod.rs`
4. Update `src/council/mod.rs` to export the full public API:
   ```rust
   pub use config::CouncilConfig;
   pub use engine::CouncilEngine;
   pub use types::*;
   pub use worker::Worker;
   ```

**Verification:** `cargo test --lib council::engine`

---

## Slice S08: Real Workers (ClaudeWorker + CodexWorker)

**Demo:** "After this, the system has production-ready `ClaudeWorker` and `CodexWorker` structs that invoke real CLI tools and parse their output into `WorkerResult` and `ReviewResult`."

### Boundary Map

**Produces:**
- `src/council/worker.rs` (extended) — `ClaudeWorker`, `CodexWorker`

**Consumes:**
- `src/council/worker.rs::Worker` trait
- `src/council/types.rs::WorkerResult`, `ReviewResult`, `ReviewVerdict`
- `src/council/config.rs::WorkerConfig`
- `tokio::process::Command`
- `serde_json` (for parsing structured output)

### Tasks

#### Task T12: ClaudeWorker implementation

**TDD Approach:** Write tests for CLI argument construction and output parsing (mocking the subprocess itself is impractical, so test the argument-building and parsing layers).

**Must-haves:**
- Truths:
  - `ClaudeWorker::new(config: &WorkerConfig) -> Self`
  - `ClaudeWorker` implements `Worker` trait
  - `build_execute_args(phase: &Phase, prompt: &str, worktree_path: &Path) -> Vec<String>` constructs the CLI arguments including `--print`, `--output-format stream-json`, `--dangerously-skip-permissions`, and `--cwd <worktree_path>`
  - `parse_execute_output(raw: &str) -> Result<WorkerResult>` extracts diff text and exit status from Claude's streaming JSON output
  - `build_review_args(phase: &Phase, diff: &str, label: &str) -> Vec<String>` constructs review invocation args
  - `parse_review_output(raw: &str) -> Result<ReviewResult>` parses structured review JSON from Claude output
  - Follows the same streaming JSON parsing pattern as `src/orchestrator/runner.rs`
- Artifacts: `src/council/worker.rs` (extended)
- Key Links: `use crate::council::config::WorkerConfig; use tokio::process::Command;`

**Tests to write first:**
```
test_claude_worker_name
test_claude_worker_build_execute_args_includes_cwd
test_claude_worker_build_execute_args_includes_output_format
test_claude_worker_build_execute_args_includes_skip_permissions
test_claude_worker_parse_execute_output_valid
test_claude_worker_parse_execute_output_empty
test_claude_worker_parse_execute_output_error_exit
test_claude_worker_build_review_args
test_claude_worker_parse_review_output_approve
test_claude_worker_parse_review_output_request_changes
test_claude_worker_parse_review_output_malformed_json
```

**Then implement:**
1. Add `ClaudeWorker` struct to `src/council/worker.rs`
2. Implement `Worker` trait with argument building and output parsing
3. Extract reusable streaming JSON parser from `runner.rs` patterns

**Verification:** `cargo test --lib council::worker::tests::test_claude_worker`

---

#### Task T13: CodexWorker implementation

**TDD Approach:** Write tests for Codex CLI argument construction and output parsing.

**Must-haves:**
- Truths:
  - `CodexWorker::new(config: &WorkerConfig) -> Self`
  - `CodexWorker` implements `Worker` trait
  - `build_execute_args(phase: &Phase, prompt: &str, worktree_path: &Path) -> Vec<String>` includes `--model`, `--reasoning-effort`, `--sandbox` from config, and `-q` for quiet mode
  - `parse_execute_output(raw: &str) -> Result<WorkerResult>` handles Codex's output format
  - `build_review_args(phase: &Phase, diff: &str, label: &str) -> Vec<String>` constructs review invocation
  - `parse_review_output(raw: &str) -> Result<ReviewResult>` parses Codex review output
  - Missing optional config fields (model, reasoning_effort) fall back to sensible defaults
- Artifacts: `src/council/worker.rs` (extended)
- Key Links: `use crate::council::config::WorkerConfig;`

**Tests to write first:**
```
test_codex_worker_name
test_codex_worker_build_execute_args_includes_model
test_codex_worker_build_execute_args_includes_reasoning_effort
test_codex_worker_build_execute_args_includes_sandbox
test_codex_worker_build_execute_args_defaults_without_model
test_codex_worker_parse_execute_output_valid
test_codex_worker_parse_execute_output_empty
test_codex_worker_build_review_args
test_codex_worker_parse_review_output_approve
test_codex_worker_parse_review_output_request_changes
```

**Then implement:**
1. Add `CodexWorker` struct to `src/council/worker.rs`
2. Implement `Worker` trait

**Verification:** `cargo test --lib council::worker::tests::test_codex_worker`

---

#### Task T14: Worker factory function

**TDD Approach:** Write tests for constructing workers from config.

**Must-haves:**
- Truths:
  - `create_workers(config: &CouncilConfig) -> Result<Vec<Arc<dyn Worker>>>` creates the correct worker type based on `WorkerConfig.cmd`
  - `cmd = "claude"` produces `ClaudeWorker`
  - `cmd = "codex"` produces `CodexWorker`
  - Unknown `cmd` returns an error
  - `create_chairman_worker(config: &CouncilConfig) -> Result<Arc<dyn Worker>>` creates a worker for the chairman using `chairman_model`
- Artifacts: `src/council/worker.rs` (extended)
- Key Links: `use crate::council::config::CouncilConfig;`

**Tests to write first:**
```
test_create_workers_claude_and_codex
test_create_workers_unknown_cmd_errors
test_create_workers_empty_config
test_create_chairman_worker_uses_chairman_model
test_create_workers_returns_arc_dyn_worker
```

**Then implement:**
1. Add factory functions to `src/council/worker.rs`

**Verification:** `cargo test --lib council::worker::tests::test_create`

---

## Slice S09: Orchestrator Integration

**Demo:** "After this, the user can set `[council] enabled = true` in `forge.toml` and `forge run` uses the council engine instead of the single-worker loop -- with hooks, audit trail, and per-phase overrides all working."

### Boundary Map

**Produces:**
- Modified `src/orchestrator/runner.rs` — council branch in iteration logic
- Modified `src/phase.rs` — optional `council_override: Option<bool>` field on `Phase`
- Modified `src/orchestrator/mod.rs` — re-exports

**Consumes:**
- `src/council::CouncilEngine`, `CouncilConfig`, `CouncilPhaseResult`
- `src/council::worker::create_workers`, `create_chairman_worker`
- `src/config.rs::Config`
- `src/forge_config.rs::ForgeConfig`
- `src/phase.rs::Phase`
- `src/hooks/manager.rs::HookManager`
- `src/audit/mod.rs::PhaseAudit`, `IterationAudit`

### Tasks

#### Task T15: Per-phase council override on Phase struct

**TDD Approach:** Write tests for phase-level council opt-in/opt-out.

**Must-haves:**
- Truths:
  - `Phase` gains `pub council: Option<bool>` field, `#[serde(default, skip_serializing_if = "Option::is_none")]`
  - `None` means "use global config", `Some(true)` forces council on, `Some(false)` forces council off
  - Existing phase JSON without `council` field still deserializes correctly
  - A helper `Phase::use_council(&self, global_enabled: bool) -> bool` resolves the effective setting
- Artifacts: Modified `src/phase.rs`
- Key Links: None new

**Tests to write first (in `src/phase.rs` test module):**
```
test_phase_council_field_default_none
test_phase_council_field_deserialize_true
test_phase_council_field_deserialize_false
test_phase_council_field_absent_in_json
test_phase_use_council_none_global_true
test_phase_use_council_none_global_false
test_phase_use_council_some_true_global_false
test_phase_use_council_some_false_global_true
```

**Then implement:**
1. Add `council: Option<bool>` to `Phase` struct
2. Add `use_council()` method

**Verification:** `cargo test --lib phase`

---

#### Task T16: Orchestrator council dispatch

**TDD Approach:** Write tests verifying that `ClaudeRunner` dispatches to council when config says so.

**Must-haves:**
- Truths:
  - `ClaudeRunner` gains a method `fn should_use_council(&self, phase: &Phase) -> bool` that checks `ForgeConfig.council_config()` + `phase.use_council()`
  - `ClaudeRunner` gains `pub async fn run_council_iteration(&self, phase: &Phase, iteration: u32, ui: Option<Arc<OrchestratorUI>>) -> Result<IterationResult>` that:
    - Creates a `CouncilEngine` from config
    - Calls `engine.run_phase()`
    - Converts `CouncilPhaseResult` to `IterationResult` (maps merged diff to output, sets `promise_found` based on promise tag search in merged output)
  - The existing `run_iteration()` remains unchanged -- council is a parallel path, not a modification
  - Council iteration produces the same `IterationResult` shape so the rest of the orchestrator (state, audit, hooks) works unchanged
  - When council is disabled, behavior is identical to current (zero regression)
- Artifacts: Modified `src/orchestrator/runner.rs`
- Key Links: `use crate::council::{CouncilEngine, CouncilConfig, worker::{create_workers, create_chairman_worker}};`

**Tests to write first (in `src/orchestrator/runner.rs` test module):**
```
test_should_use_council_disabled_globally
test_should_use_council_enabled_globally
test_should_use_council_phase_override_true
test_should_use_council_phase_override_false
test_council_iteration_result_has_promise_found
test_council_iteration_result_has_output
test_council_iteration_result_has_signals
```

**Then implement:**
1. Add `should_use_council()` to `ClaudeRunner`
2. Add `run_council_iteration()` to `ClaudeRunner`
3. Add conversion function `council_result_to_iteration_result(council_result: CouncilPhaseResult, phase: &Phase) -> IterationResult`

**Verification:** `cargo test --lib orchestrator::runner`

---

#### Task T17: Audit trail enrichment

**TDD Approach:** Write tests verifying council-specific data appears in the audit trail.

**Must-haves:**
- Truths:
  - `IterationAudit` (in `src/audit/mod.rs`) gains `pub council_data: Option<CouncilAuditData>` field, `#[serde(default, skip_serializing_if = "Option::is_none")]`
  - `CouncilAuditData` holds: `workers_used: Vec<String>`, `review_verdicts: Vec<(String, String)>` (reviewer, verdict), `merge_attempts: u32`, `chairman_decision: String`, `winning_worker: Option<String>`
  - Existing audit JSON without `council_data` still deserializes (backward compat)
  - When council is not used, `council_data` is `None`
- Artifacts: Modified `src/audit/mod.rs`, new `src/council/types.rs::CouncilAuditData`
- Key Links: `use crate::council::types::CouncilAuditData;`

**Tests to write first:**
```
test_iteration_audit_council_data_default_none
test_iteration_audit_council_data_serialization
test_iteration_audit_backward_compat_no_council_field
test_council_audit_data_construction
test_council_audit_data_from_council_phase_result
```

**Then implement:**
1. Add `CouncilAuditData` to `src/council/types.rs`
2. Add `council_data` field to `IterationAudit`
3. Add conversion `CouncilPhaseResult::to_audit_data() -> CouncilAuditData`

**Verification:** `cargo test --lib audit && cargo test --lib council::types::tests::test_council_audit`

---

## Dependency Graph

```
S01 (Types & Config)
 |
 +---> S02 (Worker Trait & Mock)
 |      |
 |      +---> S05 (Peer Review) --+
 |      |                         |
 |      +---> S08 (Real Workers)  |
 |             |                  |
 +---> S03 (Git Worktree/Merge)   |
 |      |                        |
 +---> S04 (Prompts) --+---------+
                       |         |
                       v         v
                  S06 (Chairman)
                       |
                       v
                  S07 (Engine)
                       |
                       v
                  S09 (Orchestrator Integration)
```

## Summary of Files

### New Files (8)
| File | Slice | Purpose |
|------|-------|---------|
| `src/council/mod.rs` | S01 | Module root, public re-exports |
| `src/council/types.rs` | S01 | All domain types and error enum |
| `src/council/config.rs` | S01 | `CouncilConfig`, `WorkerConfig` |
| `src/council/worker.rs` | S02, S08 | `Worker` trait, `MockWorker`, `ClaudeWorker`, `CodexWorker`, factory |
| `src/council/merge.rs` | S03 | `WorktreeManager`, `PatchSet`, `apply_patch`, `detect_conflicts` |
| `src/council/prompts.rs` | S04 | Prompt templates and anonymization |
| `src/council/reviewer.rs` | S05 | `PeerReviewEngine`, `ReviewRound` |
| `src/council/chairman.rs` | S06 | `Chairman`, `ChairmanDecision`, `SynthesisResult` |
| `src/council/engine.rs` | S07 | `CouncilEngine` three-stage orchestrator |

### Modified Files (4)
| File | Slice | Change |
|------|-------|--------|
| `src/lib.rs` | S01 | Add `pub mod council;` |
| `src/forge_config.rs` | S01 | Add `council: Option<CouncilConfig>` to `ForgeToml`, accessor to `ForgeConfig` |
| `src/phase.rs` | S09 | Add `council: Option<bool>` field, `use_council()` method |
| `src/orchestrator/runner.rs` | S09 | Add `should_use_council()`, `run_council_iteration()` |
| `src/audit/mod.rs` | S09 | Add `council_data: Option<CouncilAuditData>` to `IterationAudit` |

## Test Count Summary

| Slice | Tests |
|-------|-------|
| S01: Types & Config | 26 |
| S02: Worker Trait & Mock | 9 |
| S03: Git Worktree/Merge | 28 |
| S04: Prompts | 13 |
| S05: Peer Review | 10 |
| S06: Chairman | 11 |
| S07: Engine | 12 |
| S08: Real Workers | 26 |
| S09: Orchestrator Integration | 20 |
| **Total** | **155** |

## Execution Order

Tasks are numbered T01-T17. Execute in order, verifying each with `cargo test` before proceeding:

1. **T01** - types.rs
2. **T02** - config.rs
3. **T03** - ForgeToml integration
4. **T04** - Worker trait + MockWorker
5. **T05** - Worktree creation/cleanup
6. **T06** - Diff generation + patch parsing
7. **T07** - Patch application + conflict detection
8. **T08** - Prompt templates
9. **T09** - Peer review engine
10. **T10** - Chairman synthesis
11. **T11** - Council engine
12. **T12** - ClaudeWorker
13. **T13** - CodexWorker
14. **T14** - Worker factory
15. **T15** - Phase council override
16. **T16** - Orchestrator dispatch
17. **T17** - Audit trail enrichment

At each step: write listed tests (they fail) -> implement (they pass) -> `cargo test` (no regressions) -> commit.
