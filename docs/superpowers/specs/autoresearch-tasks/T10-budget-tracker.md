# Task T10: Budget Tracker

## Context
This task implements the `BudgetTracker` struct that enforces a $25 total spend cap across the autoresearch experiment loop. Each experiment involves multiple LLM calls (Claude for prompt mutation, Claude/Codex for benchmark runs, Claude for judging). The tracker estimates cost from token counts and model-specific pricing, persists spend data so it survives resume, and provides a `can_afford()` check before each experiment. Part of Slice S04 (Experiment Loop + CLI Command).

## Prerequisites
- T09 complete: `src/cmd/autoresearch/mod.rs` exists and compiles.
- `serde`, `anyhow`, and `chrono` are available in `Cargo.toml`.

## Session Startup
Read these files:
1. `src/cmd/autoresearch/mod.rs` — existing autoresearch module (from T09)
2. `src/audit/mod.rs` — `TokenUsage` struct pattern
3. `Cargo.toml` — confirm `serde` and `chrono` are available

## TDD Sequence

### Step 1: Red — `test_budget_tracker_new_defaults`
Create `src/cmd/autoresearch/budget.rs` with the `BudgetTracker` struct.

```rust
// src/cmd/autoresearch/budget.rs

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Pricing tiers for supported LLM models.
/// Prices are in USD per million tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model_name: String,
    pub input_price_per_mtok: f64,
    pub output_price_per_mtok: f64,
}

impl ModelPricing {
    /// Claude Sonnet approximate pricing.
    pub fn claude_sonnet() -> Self {
        Self {
            model_name: "claude-sonnet".to_string(),
            input_price_per_mtok: 3.0,
            output_price_per_mtok: 15.0,
        }
    }

    /// GPT 5.4 approximate pricing.
    pub fn gpt_5_4() -> Self {
        Self {
            model_name: "gpt-5.4".to_string(),
            input_price_per_mtok: 2.0,
            output_price_per_mtok: 8.0,
        }
    }

    /// Estimate cost for a single call given token counts.
    pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_price_per_mtok;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_price_per_mtok;
        input_cost + output_cost
    }
}

/// A single cost record for one LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub estimated_cost_usd: f64,
    pub description: String,
}

/// Tracks cumulative spend and enforces a budget cap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetTracker {
    /// Maximum allowed spend in USD.
    pub budget_cap_usd: f64,
    /// Running total of estimated spend in USD.
    pub total_spent_usd: f64,
    /// Individual cost records (for audit trail).
    pub records: Vec<CostRecord>,
}

impl BudgetTracker {
    /// Create a new tracker with the given budget cap.
    pub fn new(budget_cap_usd: f64) -> Self {
        Self {
            budget_cap_usd,
            total_spent_usd: 0.0,
            records: Vec::new(),
        }
    }

    /// Check if there is enough budget remaining for an estimated cost.
    pub fn can_afford(&self, estimated_cost: f64) -> bool {
        self.total_spent_usd + estimated_cost <= self.budget_cap_usd
    }

    /// Remaining budget in USD.
    pub fn remaining(&self) -> f64 {
        (self.budget_cap_usd - self.total_spent_usd).max(0.0)
    }

    /// Record a cost. Returns error if this would exceed the budget cap.
    pub fn record_cost(
        &mut self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        pricing: &ModelPricing,
        description: &str,
    ) -> Result<f64> {
        let cost = pricing.estimate_cost(input_tokens, output_tokens);
        if self.total_spent_usd + cost > self.budget_cap_usd {
            anyhow::bail!(
                "Budget exceeded: ${:.4} would bring total to ${:.4}, cap is ${:.2}",
                cost,
                self.total_spent_usd + cost,
                self.budget_cap_usd
            );
        }
        self.total_spent_usd += cost;
        self.records.push(CostRecord {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            estimated_cost_usd: cost,
            description: description.to_string(),
        });
        Ok(cost)
    }

    /// Restore budget state from a results TSV file.
    /// Each kept/discarded row represents one experiment that cost approximately
    /// the given `cost_per_experiment` (a rough estimate used for resume).
    pub fn restore_from_experiment_count(
        &mut self,
        experiment_count: usize,
        avg_cost_per_experiment: f64,
    ) {
        let restored_spend = experiment_count as f64 * avg_cost_per_experiment;
        self.total_spent_usd = restored_spend;
    }

    /// Serialize to JSON for persistence.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to serialize BudgetTracker")
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("Failed to deserialize BudgetTracker")
    }

    /// Save to a file path.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json).context("Failed to write budget tracker file")?;
        Ok(())
    }

    /// Load from a file path. Returns None if file does not exist.
    pub fn load(path: &std::path::Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .context("Failed to read budget tracker file")?;
        let tracker = Self::from_json(&content)?;
        Ok(Some(tracker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_tracker_new_defaults() {
        let tracker = BudgetTracker::new(25.0);
        assert!((tracker.budget_cap_usd - 25.0).abs() < f64::EPSILON);
        assert!((tracker.total_spent_usd - 0.0).abs() < f64::EPSILON);
        assert!(tracker.records.is_empty());
        assert!((tracker.remaining() - 25.0).abs() < f64::EPSILON);
    }
}
```

Run and verify the test fails (file does not exist), then create the file and make it pass:
```bash
cargo test --lib cmd::autoresearch::budget::tests::test_budget_tracker_new_defaults
```

### Step 2: Red — `test_model_pricing_estimate_cost`
Test the cost estimation for a known token count.

```rust
    #[test]
    fn test_model_pricing_estimate_cost_claude() {
        let pricing = ModelPricing::claude_sonnet();
        // 10,000 input tokens at $3/MTok = $0.03
        // 2,000 output tokens at $15/MTok = $0.03
        // Total = $0.06
        let cost = pricing.estimate_cost(10_000, 2_000);
        assert!(
            (cost - 0.06).abs() < 0.0001,
            "expected ~$0.06, got ${:.6}",
            cost
        );
    }

    #[test]
    fn test_model_pricing_estimate_cost_gpt() {
        let pricing = ModelPricing::gpt_5_4();
        // 10,000 input tokens at $2/MTok = $0.02
        // 2,000 output tokens at $8/MTok = $0.016
        // Total = $0.036
        let cost = pricing.estimate_cost(10_000, 2_000);
        assert!(
            (cost - 0.036).abs() < 0.0001,
            "expected ~$0.036, got ${:.6}",
            cost
        );
    }

    #[test]
    fn test_model_pricing_zero_tokens() {
        let pricing = ModelPricing::claude_sonnet();
        let cost = pricing.estimate_cost(0, 0);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }
```

### Step 3: Green — implement and make pass

### Step 4: Red — `test_can_afford`
Test the budget gate check.

```rust
    #[test]
    fn test_can_afford_within_budget() {
        let tracker = BudgetTracker::new(1.0);
        assert!(tracker.can_afford(0.50));
        assert!(tracker.can_afford(1.00));  // exactly at cap
    }

    #[test]
    fn test_can_afford_exceeds_budget() {
        let tracker = BudgetTracker::new(1.0);
        assert!(!tracker.can_afford(1.01));
    }

    #[test]
    fn test_can_afford_after_spending() {
        let mut tracker = BudgetTracker::new(1.0);
        let pricing = ModelPricing::claude_sonnet();
        // Spend ~$0.06
        tracker.record_cost("claude", 10_000, 2_000, &pricing, "test call").unwrap();
        assert!(tracker.can_afford(0.50));
        assert!(!tracker.can_afford(1.00)); // 0.06 + 1.00 > 1.0
    }
```

### Step 5: Red — `test_record_cost_updates_total`
Test that `record_cost` updates `total_spent_usd` and appends a record.

```rust
    #[test]
    fn test_record_cost_updates_total() {
        let mut tracker = BudgetTracker::new(25.0);
        let pricing = ModelPricing::claude_sonnet();

        let cost = tracker
            .record_cost("claude-sonnet", 10_000, 2_000, &pricing, "mutation call")
            .unwrap();

        assert!((cost - 0.06).abs() < 0.0001);
        assert!((tracker.total_spent_usd - 0.06).abs() < 0.0001);
        assert_eq!(tracker.records.len(), 1);
        assert_eq!(tracker.records[0].model, "claude-sonnet");
        assert_eq!(tracker.records[0].description, "mutation call");
        assert_eq!(tracker.records[0].input_tokens, 10_000);
        assert_eq!(tracker.records[0].output_tokens, 2_000);
    }
```

### Step 6: Red — `test_record_cost_rejects_over_budget`
Verify that `record_cost` returns an error when the cost would exceed the cap.

```rust
    #[test]
    fn test_record_cost_rejects_over_budget() {
        let mut tracker = BudgetTracker::new(0.05);
        let pricing = ModelPricing::claude_sonnet();

        // This costs ~$0.06 which exceeds the $0.05 cap
        let result = tracker.record_cost("claude", 10_000, 2_000, &pricing, "over budget");
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Budget exceeded"),
            "error message must mention budget exceeded"
        );
        // Spend must NOT have increased
        assert!((tracker.total_spent_usd - 0.0).abs() < f64::EPSILON);
        assert!(tracker.records.is_empty());
    }
```

### Step 7: Red — `test_remaining_never_negative`
Test that `remaining()` is floored at zero.

```rust
    #[test]
    fn test_remaining_never_negative() {
        let mut tracker = BudgetTracker::new(0.01);
        // Manually set spend above cap to test the floor
        tracker.total_spent_usd = 0.02;
        assert!((tracker.remaining() - 0.0).abs() < f64::EPSILON);
    }
```

### Step 8: Red — `test_restore_from_experiment_count`
Test resume-from-results logic.

```rust
    #[test]
    fn test_restore_from_experiment_count() {
        let mut tracker = BudgetTracker::new(25.0);
        tracker.restore_from_experiment_count(10, 0.40);
        // 10 experiments * $0.40 = $4.00
        assert!((tracker.total_spent_usd - 4.0).abs() < 0.0001);
        assert!((tracker.remaining() - 21.0).abs() < 0.0001);
    }
```

### Step 9: Red — `test_json_roundtrip`
Test serialization/deserialization.

```rust
    #[test]
    fn test_json_roundtrip() {
        let mut tracker = BudgetTracker::new(25.0);
        let pricing = ModelPricing::claude_sonnet();
        tracker.record_cost("claude", 5_000, 1_000, &pricing, "roundtrip test").unwrap();

        let json = tracker.to_json().unwrap();
        let restored = BudgetTracker::from_json(&json).unwrap();

        assert!((restored.budget_cap_usd - 25.0).abs() < f64::EPSILON);
        assert!((restored.total_spent_usd - tracker.total_spent_usd).abs() < 0.0001);
        assert_eq!(restored.records.len(), 1);
        assert_eq!(restored.records[0].model, "claude");
    }
```

### Step 10: Red — `test_save_and_load`
Test file persistence.

```rust
    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.json");

        let mut tracker = BudgetTracker::new(25.0);
        let pricing = ModelPricing::claude_sonnet();
        tracker.record_cost("claude", 5_000, 1_000, &pricing, "save test").unwrap();
        tracker.save(&path).unwrap();

        let loaded = BudgetTracker::load(&path).unwrap().unwrap();
        assert!((loaded.total_spent_usd - tracker.total_spent_usd).abs() < 0.0001);
        assert_eq!(loaded.records.len(), 1);
    }

    #[test]
    fn test_load_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = BudgetTracker::load(&path).unwrap();
        assert!(result.is_none());
    }
```

### Step 11: Refactor
- Add doc comments to all public items.
- Run `cargo clippy`.
- Ensure `budget.rs` is declared in `src/cmd/autoresearch/mod.rs` with `pub mod budget;`.

## Files
- Create: `src/cmd/autoresearch/budget.rs`
- Modify: `src/cmd/autoresearch/mod.rs` (add `pub mod budget;`)

## Must-Haves (Verification)
- [ ] Truth: `cargo test --lib cmd::autoresearch::budget` passes (12+ tests green)
- [ ] Artifact: `src/cmd/autoresearch/budget.rs` exists with `BudgetTracker`, `ModelPricing`, `CostRecord`
- [ ] Key Link: `BudgetTracker::can_afford()` returns false when spend + estimated cost > cap
- [ ] Key Link: `BudgetTracker::record_cost()` returns `Err` when cost would exceed cap (and does NOT update spend)
- [ ] Key Link: `BudgetTracker::save()`/`load()` persist to and restore from JSON files
- [ ] Key Link: `BudgetTracker::restore_from_experiment_count()` allows resume from results.tsv row count

## Verification Commands
```bash
# All budget tracker tests pass
cargo test --lib cmd::autoresearch::budget -- --nocapture

# Full build succeeds
cargo build 2>&1

# Clippy clean
cargo clippy -- -D warnings 2>&1
```

## Definition of Done
1. `BudgetTracker` struct with `new()`, `can_afford()`, `remaining()`, `record_cost()`, `restore_from_experiment_count()`, `save()`, `load()`, `to_json()`, `from_json()`.
2. `ModelPricing` struct with `claude_sonnet()`, `gpt_5_4()`, `estimate_cost()`.
3. `CostRecord` struct capturing individual call costs.
4. 12+ tests covering: defaults, pricing math, budget gate, over-budget rejection, remaining floor, resume restoration, JSON roundtrip, file save/load, nonexistent file load.
5. `cargo test --lib cmd::autoresearch::budget` passes.
6. `cargo clippy` clean.
