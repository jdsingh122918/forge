//! Budget tracking for autoresearch experiment loops.
//!
//! Enforces a total spend cap by estimating cost from token counts and
//! model-specific pricing, persisting spend data for resume, and providing
//! a [`BudgetTracker::can_afford`] gate before each experiment.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Pricing tiers for supported LLM models.
///
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

    /// Remaining budget in USD, floored at zero.
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

    /// Restore budget state from a previous experiment count.
    ///
    /// Each row in results.tsv represents one experiment that cost approximately
    /// `avg_cost_per_experiment`. This allows resuming a run without exact cost records.
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

    /// Load from a file path. Returns `None` if file does not exist.
    pub fn load(path: &std::path::Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let content =
            std::fs::read_to_string(path).context("Failed to read budget tracker file")?;
        let tracker = Self::from_json(&content)?;
        Ok(Some(tracker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- BudgetTracker::new defaults ---

    #[test]
    fn test_budget_tracker_new_defaults() {
        let tracker = BudgetTracker::new(25.0);
        assert!((tracker.budget_cap_usd - 25.0).abs() < f64::EPSILON);
        assert!((tracker.total_spent_usd - 0.0).abs() < f64::EPSILON);
        assert!(tracker.records.is_empty());
        assert!((tracker.remaining() - 25.0).abs() < f64::EPSILON);
    }

    // --- ModelPricing::estimate_cost ---

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

    // --- BudgetTracker::can_afford ---

    #[test]
    fn test_can_afford_within_budget() {
        let tracker = BudgetTracker::new(1.0);
        assert!(tracker.can_afford(0.50));
        assert!(tracker.can_afford(1.00)); // exactly at cap
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
        tracker
            .record_cost("claude", 10_000, 2_000, &pricing, "test call")
            .unwrap();
        assert!(tracker.can_afford(0.50));
        assert!(!tracker.can_afford(1.00)); // 0.06 + 1.00 > 1.0
    }

    // --- BudgetTracker::record_cost ---

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

    // --- BudgetTracker::remaining ---

    #[test]
    fn test_remaining_never_negative() {
        let mut tracker = BudgetTracker::new(0.01);
        // Manually set spend above cap to test the floor
        tracker.total_spent_usd = 0.02;
        assert!((tracker.remaining() - 0.0).abs() < f64::EPSILON);
    }

    // --- BudgetTracker::restore_from_experiment_count ---

    #[test]
    fn test_restore_from_experiment_count() {
        let mut tracker = BudgetTracker::new(25.0);
        tracker.restore_from_experiment_count(10, 0.40);
        // 10 experiments * $0.40 = $4.00
        assert!((tracker.total_spent_usd - 4.0).abs() < 0.0001);
        assert!((tracker.remaining() - 21.0).abs() < 0.0001);
    }

    // --- JSON serialization ---

    #[test]
    fn test_json_roundtrip() {
        let mut tracker = BudgetTracker::new(25.0);
        let pricing = ModelPricing::claude_sonnet();
        tracker
            .record_cost("claude", 5_000, 1_000, &pricing, "roundtrip test")
            .unwrap();

        let json = tracker.to_json().unwrap();
        let restored = BudgetTracker::from_json(&json).unwrap();

        assert!((restored.budget_cap_usd - 25.0).abs() < f64::EPSILON);
        assert!((restored.total_spent_usd - tracker.total_spent_usd).abs() < 0.0001);
        assert_eq!(restored.records.len(), 1);
        assert_eq!(restored.records[0].model, "claude");
    }

    // --- File persistence ---

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.json");

        let mut tracker = BudgetTracker::new(25.0);
        let pricing = ModelPricing::claude_sonnet();
        tracker
            .record_cost("claude", 5_000, 1_000, &pricing, "save test")
            .unwrap();
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
}
