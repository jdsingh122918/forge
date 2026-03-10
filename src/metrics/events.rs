#[derive(Debug, Clone)]
pub enum MetricEvent {
    RunStarted { run_id: String, issue_id: Option<i64> },
    RunCompleted { run_id: String, success: bool, duration_secs: f64, phases_total: i32, phases_passed: i32 },
    PhaseStarted { run_id: String, phase_number: i32, phase_name: String, budget: i32 },
    PhaseCompleted { run_id: String, phase_number: i32, outcome: String, iterations_used: i32, duration_secs: f64 },
    IterationRecorded { run_id: String, phase_number: i32, iteration: i32, duration_secs: f64 },
    ReviewRecorded { run_id: String, phase_number: i32, specialist_type: String, verdict: String },
    CompactionRecorded { run_id: String, phase_number: i32, iterations_compacted: i32, compression_ratio: f64 },
}
