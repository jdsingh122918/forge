# LLM Council Engine — Task Execution Prompts

Each prompt below is a self-contained instruction set for executing one task. Copy-paste into a Claude Code session (or use with `forge phase`). Tasks must be executed in order T01→T17.

**TDD discipline:** Write ALL listed tests first (they must fail/not compile). Then implement until all tests pass. Run `cargo test` to verify no regressions. Commit.

---

## Task T01: Council types module

**Slice:** S01 — Types & Config Foundation
**Files to create:** `src/council/mod.rs`, `src/council/types.rs`
**Files to modify:** `src/lib.rs`

### Prompt

```
You are implementing the LLM Council Engine for Forge. This is Task T01: creating the foundational types.

CONTEXT:
- Forge is a Rust (Edition 2024) CLI tool at /Users/jdsingh/Projects/AI/forge
- Read the design spec: docs/superpowers/specs/2026-03-10-council-engine-design.md
- Read the implementation plan: docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md
- All public functions return Result<T> with .context() for error enrichment
- Uses thiserror for error types, serde for serialization

STRICT TDD — WRITE TESTS FIRST:

1. Create src/council/mod.rs with `pub mod types;`
2. Add `pub mod council;` to src/lib.rs
3. Create src/council/types.rs with ONLY the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_result_construction() { /* WorkerResult can be constructed with all fields */ }

    #[test]
    fn test_worker_result_serialization_roundtrip() { /* serde_json round-trip preserves all fields */ }

    #[test]
    fn test_review_verdict_variants() { /* All ReviewVerdict variants exist: Approve, RequestChanges(String), Abstain */ }

    #[test]
    fn test_review_result_approve_verdict() { /* ReviewResult with Approve verdict */ }

    #[test]
    fn test_review_result_request_changes_verdict() { /* ReviewResult with RequestChanges carries message */ }

    #[test]
    fn test_merge_outcome_clean() { /* MergeOutcome::Clean holds merged diff string */ }

    #[test]
    fn test_merge_outcome_conflict() { /* MergeOutcome::Conflict holds Vec<String> of paths */ }

    #[test]
    fn test_merge_outcome_failure() { /* MergeOutcome::Failure holds error message */ }

    #[test]
    fn test_council_phase_result_construction() { /* CouncilPhaseResult with all fields */ }

    #[test]
    fn test_council_error_display_messages() { /* Each CouncilError variant has a meaningful Display */ }

    #[test]
    fn test_council_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CouncilError>();
    }
}
```

4. Run `cargo test --lib council::types` — tests should FAIL (types don't exist yet)

5. NOW implement the types to make all tests pass:

- `WorkerResult` — worker name (String), diff text (String), exit_code (i32), duration (Duration), token_usage (Option<TokenUsage>), raw_output (String), signals (Vec<String>)
- `ReviewVerdict` — enum: Approve, RequestChanges(String), Abstain
- `ReviewResult` — reviewer_name (String), candidate_label (String), verdict (ReviewVerdict), scores (ReviewScores), issues (Vec<String>), summary (String), duration (Duration)
- `ReviewScores` — correctness (f32), completeness (f32), style (f32), performance (f32), overall (f32)
- `MergeOutcome` — enum: Clean(String), Conflict(Vec<String>), Failure(String)
- `CouncilPhaseResult` — winning_diff (String), worker_results (Vec<WorkerResult>), review_results (Vec<ReviewResult>), merge_outcome (MergeOutcome), merge_attempts (u32)
- `CouncilError` — thiserror enum: WorkerFailed, ReviewFailed, MergeFailed, BudgetExhausted, EscalationRequired
- All types derive Debug, Clone, Serialize, Deserialize where appropriate

6. Run `cargo test --lib council::types` — ALL tests must pass
7. Run `cargo test` — no regressions
8. Commit: `git commit -m "feat(council): add foundational types module (T01)"`
```

---

## Task T02: Council config parsing

**Slice:** S01 — Types & Config Foundation
**Files to create:** `src/council/config.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T02: Council config parsing.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (Configuration section)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T02)
- T01 is complete: src/council/types.rs exists with all types
- Forge uses toml crate for config, serde for deserialization
- Follow existing patterns in src/forge_config.rs for config structs

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod config;` to src/council/mod.rs
2. Create src/council/config.rs with ONLY the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_council_config_default() { /* Default has enabled=false, empty workers */ }

    #[test]
    fn test_council_config_enabled_false_by_default() { /* CouncilConfig::default().enabled == false */ }

    #[test]
    fn test_council_config_parse_full_toml() {
        /* Parse this TOML fragment:
        [council]
        enabled = true
        chairman_model = "gpt-5.4"
        chairman_reasoning_effort = "xhigh"
        chairman_retry_budget = 5
        anonymize_reviews = true

        [council.workers.claude]
        cmd = "claude"
        role = "worker"
        flags = ["--print", "--output-format", "stream-json"]

        [council.workers.codex]
        cmd = "codex"
        role = "worker"
        model = "gpt-5.4"
        reasoning_effort = "xhigh"
        sandbox = "workspace-write"
        */
    }

    #[test]
    fn test_council_config_parse_minimal_toml() { /* Only [council] enabled = true, no workers */ }

    #[test]
    fn test_worker_config_defaults() { /* role defaults to "worker", flags defaults to empty vec */ }

    #[test]
    fn test_worker_config_with_all_fields() { /* All optional fields populated */ }

    #[test]
    fn test_council_config_serialization_roundtrip() { /* toml::to_string then toml::from_str */ }

    #[test]
    fn test_council_config_missing_workers_is_valid() { /* No [council.workers] section is fine */ }

    #[test]
    fn test_council_config_chairman_retry_budget_default() { /* Default is 3 */ }

    #[test]
    fn test_council_config_anonymize_reviews_default_true() { /* Default is true */ }
}
```

3. Run `cargo test --lib council::config` — tests FAIL

4. Implement:
- `CouncilConfig` with: enabled (bool, default false), chairman_model (String, default "gpt-5.4"), chairman_reasoning_effort (String, default "xhigh"), chairman_retry_budget (u32, default 3), anonymize_reviews (bool, default true), workers (HashMap<String, WorkerConfig>, default empty)
- `WorkerConfig` with: cmd (String), role (String, default "worker"), flags (Vec<String>, default empty), model (Option<String>), reasoning_effort (Option<String>), sandbox (Option<String>), approval_policy (Option<String>)
- Both derive Serialize, Deserialize, Debug, Clone with serde defaults

5. Run `cargo test --lib council::config` — ALL pass
6. Run `cargo test` — no regressions
7. Commit: `git commit -m "feat(council): add config parsing (T02)"`
```

---

## Task T03: Integrate council config into ForgeToml

**Slice:** S01 — Types & Config Foundation
**Files to modify:** `src/forge_config.rs`, `src/council/mod.rs`

### Prompt

```
You are implementing Task T03: Integrating council config into ForgeToml.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T03)
- T01-T02 complete: src/council/types.rs and src/council/config.rs exist
- src/forge_config.rs has ForgeToml struct (~line 591) with existing optional sections
- ForgeConfig wraps ForgeToml with accessor methods
- Pattern to follow: look at how other optional config sections (e.g., autonomy, decomposition) are integrated

STRICT TDD — WRITE TESTS FIRST:

1. Read src/forge_config.rs to understand the ForgeToml struct and existing test patterns
2. Add these tests to the existing #[cfg(test)] module in forge_config.rs:

```rust
#[test]
fn test_forge_toml_parse_without_council_section() {
    // A forge.toml without [council] should parse with council == None
}

#[test]
fn test_forge_toml_parse_with_council_section() {
    // A forge.toml with [council] enabled = true should populate the field
}

#[test]
fn test_forge_toml_council_workers_parsed() {
    // [council.workers.claude] and [council.workers.codex] both parse correctly
}

#[test]
fn test_forge_toml_council_defaults_when_partial() {
    // Only [council] enabled = true, rest uses defaults
}

#[test]
fn test_forge_config_council_config_accessor() {
    // ForgeConfig.council_config() returns Some(&CouncilConfig) when present
}
```

3. Run `cargo test --lib forge_config` — new tests FAIL

4. Implement:
- Add `use crate::council::config::CouncilConfig;` to forge_config.rs
- Add `#[serde(default)] pub council: Option<CouncilConfig>` to ForgeToml
- Add `pub fn council_config(&self) -> Option<&CouncilConfig>` to ForgeConfig
- Re-export CouncilConfig from src/council/mod.rs

5. Run `cargo test --lib forge_config` — ALL tests pass (existing + new)
6. Run `cargo test` — no regressions
7. Commit: `git commit -m "feat(council): integrate config into ForgeToml (T03)"`
```

---

## Task T04: Worker trait and MockWorker

**Slice:** S02 — Worker Trait & Mock Workers
**Files to create:** `src/council/worker.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T04: Worker trait and MockWorker.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (Worker Trait section)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T04)
- T01-T03 complete: types, config, and ForgeToml integration exist
- MockWorker is CRITICAL — it enables TDD for all downstream slices (S05-S07) without real CLI calls
- Uses async_trait crate (already in Cargo.toml)
- Worker must be Send + Sync for use with tokio::join!

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod worker;` to src/council/mod.rs
2. Create src/council/worker.rs with test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use std::path::Path;

    fn test_phase() -> Phase {
        // Create a minimal Phase for testing — read src/phase.rs to see required fields
        // Use serde_json::from_str with a minimal JSON object
    }

    #[tokio::test]
    async fn test_mock_worker_name() { /* MockWorker::new("test").name() == "test" */ }

    #[tokio::test]
    async fn test_mock_worker_default_execute_returns_ok() { /* .execute() returns Ok(WorkerResult) */ }

    #[tokio::test]
    async fn test_mock_worker_default_review_returns_approve() { /* .review() returns Ok with Approve verdict */ }

    #[tokio::test]
    async fn test_mock_worker_custom_execute_result() { /* .with_execute_result() overrides the return */ }

    #[tokio::test]
    async fn test_mock_worker_custom_review_result() { /* .with_review_result() overrides the return */ }

    #[tokio::test]
    async fn test_mock_worker_execute_count_increments() { /* Each .execute() call increments counter */ }

    #[tokio::test]
    async fn test_mock_worker_review_count_increments() { /* Each .review() call increments counter */ }

    #[test]
    fn test_mock_worker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockWorker>();
    }

    #[test]
    fn test_worker_trait_object_safety() {
        // Verify Worker can be used as dyn Worker (object-safe)
        fn _accepts_dyn(_w: &dyn Worker) {}
    }
}
```

3. Run `cargo test --lib council::worker` — tests FAIL

4. Implement:
- `Worker` async trait with name(), execute(), review() methods
- `MockWorker` with configurable responses, Arc<AtomicU32> call counters, builder pattern
- MockWorker::new(name) creates sensible defaults (empty diff, Approve verdict)

5. Run `cargo test --lib council::worker` — ALL pass
6. Run `cargo test` — no regressions
7. Commit: `git commit -m "feat(council): add Worker trait and MockWorker (T04)"`
```

---

## Task T05: Worktree creation and cleanup

**Slice:** S03 — Git Worktree Management
**Files to create:** `src/council/merge.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T05: Git worktree creation and cleanup.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T05)
- Forge uses git2 crate (v0.20.4, already in Cargo.toml) for git operations
- See src/tracker/git.rs for existing git2 usage patterns
- Worktrees are created under .forge/council-worktrees/<name>
- Tests should use tempfile crate (already in dev-dependencies) for temp git repos

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod merge;` to src/council/mod.rs
2. Create src/council/merge.rs with tests that create real temp git repos:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::process::Command;

    /// Helper: create a temp git repo with one initial commit
    fn create_test_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        Command::new("git").args(["init"]).current_dir(&path).output().unwrap();
        Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(&path).output().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(&path).output().unwrap();
        std::fs::write(path.join("README.md"), "# Test").unwrap();
        Command::new("git").args(["add", "."]).current_dir(&path).output().unwrap();
        Command::new("git").args(["commit", "-m", "initial"]).current_dir(&path).output().unwrap();
        (dir, path)
    }

    #[test]
    fn test_worktree_manager_new_valid_repo() { /* Opens successfully on a valid git repo */ }

    #[test]
    fn test_worktree_manager_new_invalid_path_errors() { /* Errors on non-repo path */ }

    #[test]
    fn test_create_worktree_returns_valid_path() { /* Returned PathBuf exists */ }

    #[test]
    fn test_create_worktree_directory_exists() { /* Worktree dir was created on disk */ }

    #[test]
    fn test_create_worktree_has_files_from_head() { /* README.md exists in worktree */ }

    #[test]
    fn test_remove_worktree_cleans_directory() { /* After remove, directory is gone */ }

    #[test]
    fn test_remove_worktree_nonexistent_is_noop() { /* Removing non-existent doesn't error */ }

    #[test]
    fn test_cleanup_all_removes_everything() { /* All council worktrees removed */ }

    #[test]
    fn test_two_worktrees_are_independent() {
        /* Changes in worktree-a don't appear in worktree-b */
    }
}
```

3. Run tests — FAIL
4. Implement WorktreeManager using git2 or std::process::Command for git worktree operations
5. Run `cargo test --lib council::merge` — ALL pass
6. Run `cargo test` — no regressions
7. Commit: `git commit -m "feat(council): add worktree creation and cleanup (T05)"`
```

---

## Task T06: Diff generation and patch parsing

**Slice:** S03 — Git Worktree Management
**Files to modify:** `src/council/merge.rs`

### Prompt

```
You are implementing Task T06: Diff generation and patch parsing.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T06)
- T05 complete: WorktreeManager with worktree creation/cleanup exists in src/council/merge.rs
- Reuse the create_test_repo() helper from T05 tests

STRICT TDD — WRITE TESTS FIRST:

Add these tests to the existing test module in src/council/merge.rs:

```rust
#[test]
fn test_generate_diff_empty_worktree() { /* No changes = empty diff */ }

#[test]
fn test_generate_diff_single_file_added() {
    /* Create worktree, add a new file, generate_diff returns diff containing the file */
}

#[test]
fn test_generate_diff_single_file_modified() {
    /* Modify existing file in worktree, diff shows the change */
}

#[test]
fn test_generate_diff_multiple_files() {
    /* Add/modify multiple files, all appear in diff */
}

#[test]
fn test_patch_set_parse_empty() { /* Empty string parses to empty PatchSet */ }

#[test]
fn test_patch_set_parse_single_file() { /* Single-file diff parses correctly */ }

#[test]
fn test_patch_set_parse_multi_file() { /* Multi-file diff parses into multiple entries */ }

#[test]
fn test_patch_set_files_changed() { /* files_changed() returns correct file paths */ }

#[test]
fn test_patch_set_is_empty_true() { /* Empty patch set reports is_empty() == true */ }

#[test]
fn test_patch_set_is_empty_false() { /* Non-empty patch set reports is_empty() == false */ }
```

Run tests — FAIL. Then implement:
- `WorktreeManager::generate_diff(worktree_path: &Path) -> Result<String>` — runs `git diff HEAD` in the worktree
- `PatchSet` struct with `parse(diff: &str) -> Result<PatchSet>`, `files_changed() -> Vec<&str>`, `is_empty() -> bool`

Run `cargo test --lib council::merge` — ALL pass. Commit: `git commit -m "feat(council): add diff generation and patch parsing (T06)"`
```

---

## Task T07: Patch application and conflict detection

**Slice:** S03 — Git Worktree Management
**Files to modify:** `src/council/merge.rs`

### Prompt

```
You are implementing Task T07: Patch application and conflict detection.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T07)
- T05-T06 complete: WorktreeManager, generate_diff, PatchSet exist
- MergeOutcome type exists in src/council/types.rs (from T01)

STRICT TDD — WRITE TESTS FIRST:

Add to existing test module in src/council/merge.rs:

```rust
#[test]
fn test_apply_patch_clean_success() {
    /* Create repo, generate a diff, apply it to a fresh copy — succeeds with Clean */
}

#[test]
fn test_apply_patch_conflict_detected() {
    /* Create two diffs that modify the same lines, applying second over first conflicts */
}

#[test]
fn test_apply_patch_invalid_patch_fails() {
    /* Garbage text as patch returns Failure */
}

#[test]
fn test_apply_patch_empty_patch_noop() {
    /* Empty patch string is a no-op, returns Clean("") */
}

#[test]
fn test_detect_conflicts_no_overlap() {
    /* Two patches touching different files have no conflicts */
}

#[test]
fn test_detect_conflicts_same_file_different_hunks() {
    /* Same file but non-overlapping line ranges — no conflict */
}

#[test]
fn test_detect_conflicts_same_file_overlapping_hunks() {
    /* Same file, overlapping lines — reports conflict */
}

#[test]
fn test_detect_conflicts_empty_patches() { /* No conflicts when both empty */ }

#[test]
fn test_apply_then_verify_file_contents() {
    /* Apply patch, read the actual files, verify content matches expected */
}
```

Run tests — FAIL. Then implement:
- `apply_patch(repo_path: &Path, patch: &str) -> Result<MergeOutcome>` using `git apply --check` then `git apply`
- `detect_conflicts(patch_a: &str, patch_b: &str) -> Vec<String>` parsing file paths and hunk ranges

Run `cargo test --lib council::merge` — ALL pass. Commit: `git commit -m "feat(council): add patch application and conflict detection (T07)"`
```

---

## Task T08: Prompt templates and anonymization

**Slice:** S04 — Prompt Templates
**Files to create:** `src/council/prompts.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T08: Prompt templates and label anonymization.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (Stage 2 and Stage 3 sections)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T08)
- Prompt templates are the interface between Forge and the LLMs — they must be precise
- Review prompts must request structured JSON output with scores
- Chairman prompts must request git-format patch output
- Labels are randomized to prevent model identity bias

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod prompts;` to src/council/mod.rs
2. Create src/council/prompts.rs with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_generate_label_pair_returns_two_different_labels() {
        let (a, b) = generate_label_pair();
        assert_ne!(a, b);
    }

    #[test]
    fn test_generate_label_pair_labels_from_known_pool() {
        // Run 100 times, all labels come from the defined pool
    }

    #[test]
    fn test_anonymize_label_maps_correctly() {
        let mut map = HashMap::new();
        map.insert("claude".to_string(), "Alpha".to_string());
        assert_eq!(anonymize_label("claude", &map), "Alpha");
    }

    #[test]
    fn test_review_prompt_contains_phase_name() { /* Phase name appears in output */ }

    #[test]
    fn test_review_prompt_contains_candidate_label() { /* "Candidate Alpha" appears */ }

    #[test]
    fn test_review_prompt_contains_diff_content() { /* Actual diff text is embedded */ }

    #[test]
    fn test_review_prompt_does_not_contain_worker_name() {
        /* "claude" and "codex" do NOT appear in review prompt */
    }

    #[test]
    fn test_review_prompt_requests_structured_json() { /* Prompt mentions JSON output format */ }

    #[test]
    fn test_chairman_synthesis_prompt_contains_both_diffs() { /* Both diff_a and diff_b present */ }

    #[test]
    fn test_chairman_synthesis_prompt_contains_review_feedback() { /* Review text included */ }

    #[test]
    fn test_chairman_synthesis_prompt_requests_git_format_patch() { /* "git format patch" or similar */ }

    #[test]
    fn test_chairman_synthesis_prompt_mentions_retry_budget() { /* Retry context included */ }
}
```

3. Run tests — FAIL
4. Implement: generate_label_pair(), anonymize_label(), review_prompt(), chairman_synthesis_prompt()
5. Define a const pool of 10+ label pairs (Alpha/Bravo, Echo/Foxtrot, etc.)
6. Run `cargo test --lib council::prompts` — ALL pass
7. Commit: `git commit -m "feat(council): add prompt templates and anonymization (T08)"`
```

---

## Task T09: Peer review orchestration

**Slice:** S05 — Peer Review Engine
**Files to create:** `src/council/reviewer.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T09: Peer review orchestration engine.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T09)
- T01-T08 complete: types, config, Worker trait, MockWorker, merge, prompts all exist
- This module orchestrates Stage 2: two workers review each other's diffs concurrently
- Uses MockWorker for all tests — no real CLI calls

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod reviewer;` to src/council/mod.rs
2. Create src/council/reviewer.rs with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::worker::MockWorker;
    use crate::council::config::CouncilConfig;
    use std::sync::Arc;

    fn default_config() -> CouncilConfig { CouncilConfig::default() }

    #[tokio::test]
    async fn test_peer_review_two_workers_each_reviews_other() {
        /* Worker A reviews Worker B's diff, and vice versa */
    }

    #[tokio::test]
    async fn test_peer_review_anonymization_enabled() {
        /* With anonymize_reviews=true, review calls use labels not names */
    }

    #[tokio::test]
    async fn test_peer_review_anonymization_disabled() {
        /* With anonymize_reviews=false, review calls use actual worker names */
    }

    #[tokio::test]
    async fn test_peer_review_concurrent_execution() {
        /* Both reviews complete — verify via call counts on both mocks */
    }

    #[tokio::test]
    async fn test_peer_review_label_map_populated() {
        /* ReviewRound.label_map has entries for both workers */
    }

    #[tokio::test]
    async fn test_peer_review_partial_failure_one_worker_errors() {
        /* If one review fails, the other still completes */
    }

    #[tokio::test]
    async fn test_peer_review_both_approve() { /* Both return Approve */ }

    #[tokio::test]
    async fn test_peer_review_one_requests_changes() { /* One Approve, one RequestChanges */ }

    #[tokio::test]
    async fn test_peer_review_both_request_changes() { /* Both RequestChanges */ }

    #[tokio::test]
    async fn test_peer_review_round_structure() {
        /* ReviewRound has correct number of reviews and label_map */
    }
}
```

3. Run tests — FAIL
4. Implement PeerReviewEngine and ReviewRound
5. Run `cargo test --lib council::reviewer` — ALL pass
6. Commit: `git commit -m "feat(council): add peer review engine (T09)"`
```

---

## Task T10: Chairman synthesis and retry cascade

**Slice:** S06 — Chairman Synthesis
**Files to create:** `src/council/chairman.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T10: Chairman synthesis with the full retry cascade.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (Failure Cascade section)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T10)
- T01-T09 complete: all dependencies (types, merge, prompts, reviewer) exist
- Chairman uses a Worker (MockWorker in tests) to make LLM calls
- Cascade: retry on conflict (budget 3) → winner-takes-all → escalate

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod chairman;` to src/council/mod.rs
2. Create src/council/chairman.rs with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::worker::MockWorker;
    use crate::council::types::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_chairman_clean_merge_first_attempt() {
        /* MockWorker returns a valid patch, apply succeeds → MergedPatch on first try */
    }

    #[tokio::test]
    async fn test_chairman_retry_on_conflict() {
        /* First attempt produces conflict, second succeeds — attempts == 2 */
    }

    #[tokio::test]
    async fn test_chairman_exhausts_retry_budget() {
        /* All 3 attempts produce conflicts → falls through to winner-takes-all */
    }

    #[tokio::test]
    async fn test_chairman_winner_takes_all_after_retries() {
        /* After exhausted retries, picks the worker with better review score */
    }

    #[tokio::test]
    async fn test_chairman_winner_selection_approve_beats_request_changes() {
        /* Approve verdict wins over RequestChanges */
    }

    #[tokio::test]
    async fn test_chairman_winner_selection_tie_break_alphabetical() {
        /* Same verdict → first alphabetically wins */
    }

    #[tokio::test]
    async fn test_chairman_escalation_when_both_rejected() {
        /* Both workers have critically low scores → Escalate */
    }

    #[tokio::test]
    async fn test_chairman_synthesis_result_tracks_attempts() { /* attempts field is correct */ }

    #[tokio::test]
    async fn test_chairman_synthesis_result_holds_merged_diff() { /* merged_diff is Some on success */ }

    #[tokio::test]
    async fn test_chairman_uses_chairman_model_from_config() {
        /* Verify the prompt/invocation references the configured model */
    }

    #[tokio::test]
    async fn test_chairman_passes_conflict_feedback_to_retry_prompt() {
        /* On retry, the conflict error message is included in the next prompt */
    }
}
```

3. Run tests — FAIL
4. Implement Chairman, ChairmanDecision, SynthesisResult with the three-tier cascade
5. Run `cargo test --lib council::chairman` — ALL pass
6. Commit: `git commit -m "feat(council): add chairman synthesis with retry cascade (T10)"`
```

---

## Task T11: Three-stage orchestration engine

**Slice:** S07 — Council Engine
**Files to create:** `src/council/engine.rs`
**Files to modify:** `src/council/mod.rs`

### Prompt

```
You are implementing Task T11: The three-stage council orchestration engine.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (full document)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T11)
- ALL dependencies complete: types, config, worker, merge, prompts, reviewer, chairman
- This is the MAIN entry point that ties everything together
- Uses MockWorker for all tests
- Must handle: parallel workers, worktree lifecycle, error paths

STRICT TDD — WRITE TESTS FIRST:

1. Add `pub mod engine;` to src/council/mod.rs
2. Create src/council/engine.rs with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::worker::MockWorker;
    use crate::council::config::CouncilConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_engine_full_flow_both_workers_succeed() {
        /* Two MockWorkers both return valid diffs → full pipeline completes */
    }

    #[tokio::test]
    async fn test_engine_full_flow_one_worker_fails_uses_other() {
        /* One MockWorker fails → uses the other's output directly, skip review */
    }

    #[tokio::test]
    async fn test_engine_all_workers_fail_returns_error() {
        /* Both MockWorkers fail → CouncilError::WorkerFailed */
    }

    #[tokio::test]
    async fn test_engine_creates_worktrees_for_each_worker() {
        /* Verify worktree paths passed to execute() are different */
    }

    #[tokio::test]
    async fn test_engine_cleans_up_worktrees_on_success() {
        /* After successful run, worktree directories no longer exist */
    }

    #[tokio::test]
    async fn test_engine_cleans_up_worktrees_on_error() {
        /* Even on failure, worktrees are cleaned up */
    }

    #[tokio::test]
    async fn test_engine_parallel_worker_execution() {
        /* Both workers' execute() called — verify via call counts */
    }

    #[tokio::test]
    async fn test_engine_passes_reviews_to_chairman() {
        /* Chairman receives review results from Stage 2 */
    }

    #[tokio::test]
    async fn test_engine_result_contains_all_worker_results() {
        /* CouncilPhaseResult.worker_results has 2 entries */
    }

    #[tokio::test]
    async fn test_engine_result_contains_all_review_results() {
        /* CouncilPhaseResult.review_results has 2 entries */
    }

    #[tokio::test]
    async fn test_engine_single_worker_skips_review() {
        /* Only one worker succeeds → no review, direct to output */
    }

    #[tokio::test]
    async fn test_engine_returns_council_phase_result() {
        /* Return type is CouncilPhaseResult with all fields populated */
    }
}
```

3. Run tests — FAIL
4. Implement CouncilEngine::new() and CouncilEngine::run_phase() tying all stages together
5. Update src/council/mod.rs with full public API exports
6. Run `cargo test --lib council::engine` — ALL pass
7. Commit: `git commit -m "feat(council): add three-stage council engine (T11)"`
```

---

## Task T12: ClaudeWorker implementation

**Slice:** S08 — Real Workers
**Files to modify:** `src/council/worker.rs`

### Prompt

```
You are implementing Task T12: ClaudeWorker — a real Worker that invokes Claude CLI.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T12)
- Read src/orchestrator/runner.rs (lines 280-360) to see existing Claude CLI invocation patterns
- ClaudeWorker wraps the existing pattern into the Worker trait
- Tests verify argument construction and output parsing, NOT actual CLI execution

STRICT TDD — WRITE TESTS FIRST:

Add to existing test module in src/council/worker.rs:

```rust
mod test_claude_worker {
    use super::*;

    #[test]
    fn test_claude_worker_name() { /* Returns "claude" */ }

    #[test]
    fn test_claude_worker_build_execute_args_includes_cwd() {
        /* --cwd <path> appears in args */
    }

    #[test]
    fn test_claude_worker_build_execute_args_includes_output_format() {
        /* --output-format stream-json appears */
    }

    #[test]
    fn test_claude_worker_build_execute_args_includes_skip_permissions() {
        /* --dangerously-skip-permissions appears */
    }

    #[test]
    fn test_claude_worker_parse_execute_output_valid() {
        /* Known-good Claude JSON output parses into WorkerResult */
    }

    #[test]
    fn test_claude_worker_parse_execute_output_empty() {
        /* Empty output returns WorkerResult with empty diff */
    }

    #[test]
    fn test_claude_worker_parse_execute_output_error_exit() {
        /* Error output parsed correctly */
    }

    #[test]
    fn test_claude_worker_build_review_args() {
        /* Review args include --print and the review prompt */
    }

    #[test]
    fn test_claude_worker_parse_review_output_approve() {
        /* Claude output with approve → ReviewResult with Approve verdict */
    }

    #[test]
    fn test_claude_worker_parse_review_output_request_changes() {
        /* Claude output with issues → ReviewResult with RequestChanges */
    }

    #[test]
    fn test_claude_worker_parse_review_output_malformed_json() {
        /* Malformed JSON → graceful error, not panic */
    }
}
```

Run tests — FAIL. Implement ClaudeWorker. Run `cargo test --lib council::worker::tests::test_claude_worker` — ALL pass.
Commit: `git commit -m "feat(council): add ClaudeWorker implementation (T12)"`
```

---

## Task T13: CodexWorker implementation

**Slice:** S08 — Real Workers
**Files to modify:** `src/council/worker.rs`

### Prompt

```
You are implementing Task T13: CodexWorker — a real Worker that invokes Codex CLI.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T13)
- Codex CLI uses: codex -q --json --model <model> --sandbox <mode> "prompt"
- CodexWorker reads model, reasoning_effort, sandbox from WorkerConfig
- GPT-5.4 with reasoning_effort: xhigh is the configured default

STRICT TDD — WRITE TESTS FIRST:

Add to existing test module in src/council/worker.rs:

```rust
mod test_codex_worker {
    use super::*;

    #[test]
    fn test_codex_worker_name() { /* Returns "codex" */ }

    #[test]
    fn test_codex_worker_build_execute_args_includes_model() {
        /* --model gpt-5.4 appears when config has model set */
    }

    #[test]
    fn test_codex_worker_build_execute_args_includes_reasoning_effort() {
        /* --reasoning-effort xhigh appears */
    }

    #[test]
    fn test_codex_worker_build_execute_args_includes_sandbox() {
        /* --sandbox workspace-write appears */
    }

    #[test]
    fn test_codex_worker_build_execute_args_defaults_without_model() {
        /* When model is None, --model flag is omitted */
    }

    #[test]
    fn test_codex_worker_parse_execute_output_valid() {
        /* Known-good Codex JSON output parses into WorkerResult */
    }

    #[test]
    fn test_codex_worker_parse_execute_output_empty() {
        /* Empty output → WorkerResult with empty diff */
    }

    #[test]
    fn test_codex_worker_build_review_args() {
        /* Review args include -q and --json */
    }

    #[test]
    fn test_codex_worker_parse_review_output_approve() {
        /* Codex output with approve → ReviewResult with Approve */
    }

    #[test]
    fn test_codex_worker_parse_review_output_request_changes() {
        /* Codex output with issues → ReviewResult with RequestChanges */
    }
}
```

Run tests — FAIL. Implement CodexWorker. Run `cargo test --lib council::worker::tests::test_codex_worker` — ALL pass.
Commit: `git commit -m "feat(council): add CodexWorker implementation (T13)"`
```

---

## Task T14: Worker factory function

**Slice:** S08 — Real Workers
**Files to modify:** `src/council/worker.rs`

### Prompt

```
You are implementing Task T14: Worker factory functions.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T14)
- T12-T13 complete: ClaudeWorker and CodexWorker both exist
- Factory creates the correct Worker impl based on WorkerConfig.cmd

STRICT TDD — WRITE TESTS FIRST:

Add to test module in src/council/worker.rs:

```rust
mod test_create {
    use super::*;
    use crate::council::config::CouncilConfig;

    #[test]
    fn test_create_workers_claude_and_codex() {
        /* Config with claude + codex workers → Vec with 2 Arc<dyn Worker> */
    }

    #[test]
    fn test_create_workers_unknown_cmd_errors() {
        /* Config with cmd = "unknown" → error */
    }

    #[test]
    fn test_create_workers_empty_config() {
        /* No workers configured → empty Vec */
    }

    #[test]
    fn test_create_chairman_worker_uses_chairman_model() {
        /* Chairman worker has the model from council.chairman_model */
    }

    #[test]
    fn test_create_workers_returns_arc_dyn_worker() {
        /* Return type is Vec<Arc<dyn Worker>> — trait object safety */
    }
}
```

Run tests — FAIL. Implement create_workers() and create_chairman_worker().
Run `cargo test --lib council::worker::tests::test_create` — ALL pass.
Commit: `git commit -m "feat(council): add worker factory functions (T14)"`
```

---

## Task T15: Per-phase council override on Phase struct

**Slice:** S09 — Orchestrator Integration
**Files to modify:** `src/phase.rs`

### Prompt

```
You are implementing Task T15: Adding council override field to Phase.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T15)
- Read src/phase.rs to understand the Phase struct and existing optional fields
- Follow the existing pattern of #[serde(default, skip_serializing_if = "Option::is_none")]

STRICT TDD — WRITE TESTS FIRST:

Add to the existing test module in src/phase.rs:

```rust
#[test]
fn test_phase_council_field_default_none() {
    /* Phase deserialized from JSON without "council" has council == None */
}

#[test]
fn test_phase_council_field_deserialize_true() {
    /* JSON with "council": true → Some(true) */
}

#[test]
fn test_phase_council_field_deserialize_false() {
    /* JSON with "council": false → Some(false) */
}

#[test]
fn test_phase_council_field_absent_in_json() {
    /* Existing phase JSON without council field still deserializes */
}

#[test]
fn test_phase_use_council_none_global_true() {
    /* phase.council = None, global = true → true (uses global) */
}

#[test]
fn test_phase_use_council_none_global_false() {
    /* phase.council = None, global = false → false (uses global) */
}

#[test]
fn test_phase_use_council_some_true_global_false() {
    /* phase.council = Some(true) overrides global false → true */
}

#[test]
fn test_phase_use_council_some_false_global_true() {
    /* phase.council = Some(false) overrides global true → false */
}
```

Run tests — FAIL. Implement:
- Add `council: Option<bool>` to Phase with serde annotations
- Add `pub fn use_council(&self, global_enabled: bool) -> bool`

Run `cargo test --lib phase` — ALL pass. Commit: `git commit -m "feat(council): add per-phase council override to Phase (T15)"`
```

---

## Task T16: Orchestrator council dispatch

**Slice:** S09 — Orchestrator Integration
**Files to modify:** `src/orchestrator/runner.rs`

### Prompt

```
You are implementing Task T16: Wiring the council engine into the orchestrator.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T16)
- Read src/orchestrator/runner.rs — understand ClaudeRunner and run_iteration()
- ALL council modules complete (T01-T15)
- Council is a PARALLEL path, not a modification — run_iteration() stays unchanged
- New method run_council_iteration() handles council-enabled phases
- should_use_council() resolves global config + phase override

STRICT TDD — WRITE TESTS FIRST:

Add to the test module in src/orchestrator/runner.rs (or create one if needed):

```rust
#[test]
fn test_should_use_council_disabled_globally() {
    /* council.enabled = false, no phase override → false */
}

#[test]
fn test_should_use_council_enabled_globally() {
    /* council.enabled = true, no phase override → true */
}

#[test]
fn test_should_use_council_phase_override_true() {
    /* council.enabled = false, phase.council = Some(true) → true */
}

#[test]
fn test_should_use_council_phase_override_false() {
    /* council.enabled = true, phase.council = Some(false) → false */
}

#[test]
fn test_council_iteration_result_has_promise_found() {
    /* Council result with promise signal → IterationResult.promise_found == true */
}

#[test]
fn test_council_iteration_result_has_output() {
    /* Council result maps to IterationResult with output text */
}

#[test]
fn test_council_iteration_result_has_signals() {
    /* Signals from council workers appear in IterationResult */
}
```

Run tests — FAIL. Implement:
- `should_use_council(&self, phase: &Phase) -> bool`
- `run_council_iteration()` that creates CouncilEngine, calls run_phase(), converts result
- `council_result_to_iteration_result()` conversion function

Run `cargo test --lib orchestrator::runner` — ALL pass.
Commit: `git commit -m "feat(council): wire council engine into orchestrator (T16)"`
```

---

## Task T17: Audit trail enrichment

**Slice:** S09 — Orchestrator Integration
**Files to modify:** `src/audit/mod.rs`, `src/council/types.rs`

### Prompt

```
You are implementing Task T17: Enriching the audit trail with council-specific data.

CONTEXT:
- Read docs/superpowers/specs/2026-03-10-council-engine-design.md (Audit Trail section)
- Read docs/superpowers/specs/2026-03-10-council-engine-implementation-plan.md (Task T17)
- Read src/audit/mod.rs to see existing audit types
- CouncilAuditData captures: workers used, review verdicts, merge attempts, chairman decision

STRICT TDD — WRITE TESTS FIRST:

Add CouncilAuditData tests to src/council/types.rs test module:

```rust
#[test]
fn test_council_audit_data_construction() {
    /* CouncilAuditData with all fields populated */
}

#[test]
fn test_council_audit_data_from_council_phase_result() {
    /* Conversion from CouncilPhaseResult → CouncilAuditData */
}
```

Add tests to src/audit/mod.rs test module:

```rust
#[test]
fn test_iteration_audit_council_data_default_none() {
    /* Default IterationAudit has council_data == None */
}

#[test]
fn test_iteration_audit_council_data_serialization() {
    /* IterationAudit with council_data serializes to JSON with council section */
}

#[test]
fn test_iteration_audit_backward_compat_no_council_field() {
    /* Existing audit JSON without council_data still deserializes */
}
```

Run tests — FAIL. Implement:
- Add CouncilAuditData to src/council/types.rs
- Add `council_data: Option<CouncilAuditData>` to IterationAudit
- Add `CouncilPhaseResult::to_audit_data()` conversion

Run `cargo test --lib audit && cargo test --lib council::types` — ALL pass.
Commit: `git commit -m "feat(council): add council audit trail enrichment (T17)"`
```

---

## Final Verification

After all 17 tasks are complete:

```bash
# Full test suite
cargo test

# Build in release mode
cargo build --release

# Verify the council module compiles and exports correctly
cargo doc --no-deps --open  # Check src/council/ appears in docs

# Manual smoke test
echo '[council]
enabled = false' >> .forge/forge.toml
cargo run -- config validate  # Should accept the new section
```
