# Implementation Spec: Budget Tracker for Autoresearch Experiment Loop

> Generated from: docs/superpowers/specs/autoresearch-tasks/T10-budget-tracker.md
> Generated at: 2026-03-11T21:49:26.112710+00:00

## Goal

Implement a BudgetTracker struct that enforces a $25 total spend cap across autoresearch experiments by estimating cost from token counts and model-specific pricing, persisting spend data for resume, and providing a can_afford() gate before each experiment.

## Components

| Component | Description | Complexity | Dependencies |
|-----------|-------------|------------|--------------|
| ModelPricing | Pricing tiers for supported LLM models (Claude Sonnet, GPT 5.4) with per-million-token input/output rates and a cost estimation method | low | - |
| CostRecord | Data struct capturing a single LLM call's model, token counts, estimated cost, and description for audit trail | low | ModelPricing |
| BudgetTracker | Core tracker enforcing budget cap with can_afford() gate, record_cost() with rejection on overspend, remaining() floor, restore_from_experiment_count() for resume, and JSON file persistence via save()/load() | medium | ModelPricing, CostRecord |

## Code Patterns

### Cost estimation from token counts

```
pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32) -> f64 {
    let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_price_per_mtok;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_price_per_mtok;
    input_cost + output_cost
}
```

### Budget-guarded cost recording

```
pub fn record_cost(&mut self, model: &str, input_tokens: u32, output_tokens: u32, pricing: &ModelPricing, description: &str) -> Result<f64> {
    let cost = pricing.estimate_cost(input_tokens, output_tokens);
    if self.total_spent_usd + cost > self.budget_cap_usd {
        anyhow::bail!("Budget exceeded: ${:.4} would bring total to ${:.4}, cap is ${:.2}", cost, self.total_spent_usd + cost, self.budget_cap_usd);
    }
    self.total_spent_usd += cost;
    self.records.push(CostRecord { ... });
    Ok(cost)
}
```

### JSON file persistence with Option return

```
pub fn load(path: &std::path::Path) -> Result<Option<Self>> {
    if !path.exists() { return Ok(None); }
    let content = std::fs::read_to_string(path).context("Failed to read budget tracker file")?;
    let tracker = Self::from_json(&content)?;
    Ok(Some(tracker))
}
```

## Acceptance Criteria

- [ ] cargo test --lib cmd::autoresearch::budget passes with 12+ tests green
- [ ] src/cmd/autoresearch/budget.rs exists with BudgetTracker, ModelPricing, CostRecord structs
- [ ] BudgetTracker::can_afford() returns false when spend + estimated cost > cap
- [ ] BudgetTracker::record_cost() returns Err when cost would exceed cap and does NOT update spend
- [ ] BudgetTracker::save()/load() persist to and restore from JSON files
- [ ] BudgetTracker::restore_from_experiment_count() allows resume from results.tsv row count
- [ ] cargo clippy -- -D warnings passes clean
- [ ] pub mod budget; declared in src/cmd/autoresearch/mod.rs

---
*This spec was auto-generated from a design document.*
*Original design: docs/superpowers/specs/autoresearch-tasks/T10-budget-tracker.md*
