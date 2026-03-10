use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SummaryStats {
    pub total_runs: i64,
    pub successful_runs: i64,
    /// Ratio of successful runs to total runs, in [0.0, 1.0].
    pub success_rate: f64,
    pub avg_duration_secs: f64,
    pub total_phases: i64,
    pub avg_iterations_per_phase: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseNameStats {
    pub phase_name: String,
    pub run_count: i64,
    pub avg_iterations: f64,
    pub avg_duration_secs: f64,
    /// Ratio of iterations used to budget, in [0.0, 1.0].
    pub budget_utilization: f64,
    pub success_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewStats {
    pub specialist_type: String,
    pub total_reviews: i64,
    /// Ratio of reviews with verdict "pass" to total reviews, in [0.0, 1.0].
    pub pass_rate: f64,
    pub avg_findings: f64,
    pub avg_critical: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub issue_id: Option<i64>,
    pub success: bool,
    pub duration_secs: Option<f64>,
    pub phases_total: Option<i32>,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenDailyUsage {
    pub date: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
}
