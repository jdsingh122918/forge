//! DAG scheduler for parallel phase execution.
//!
//! This module provides phase-level parallelism through a directed acyclic graph (DAG)
//! scheduler. It builds a dependency graph from phases.json and executes phases in
//! parallel waves while respecting dependencies.
//!
//! ## Architecture
//!
//! The DAG scheduler has three main components:
//!
//! 1. **Builder** - Constructs a DAG from phases with their dependencies
//! 2. **Scheduler** - Computes execution waves and manages ready states
//! 3. **Executor** - Runs phases in parallel with review integration
//!
//! ## Example
//!
//! ```no_run
//! use forge::dag::{DagScheduler, DagConfig};
//! use forge::phase::Phase;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let phases = vec![
//!     Phase::new("01", "Setup", "SETUP DONE", 5, "Initial setup", vec![]),
//!     Phase::new("02", "Core", "CORE DONE", 10, "Core work", vec!["01".to_string()]),
//!     Phase::new("03", "Tests", "TESTS DONE", 8, "Testing", vec!["01".to_string()]),
//!     Phase::new("04", "Docs", "DOCS DONE", 5, "Documentation", vec!["02".to_string(), "03".to_string()]),
//! ];
//!
//! let config = DagConfig::default();
//! let scheduler = DagScheduler::from_phases(&phases, config)?;
//!
//! // Compute execution waves
//! let waves = scheduler.compute_waves();
//! // Wave 0: [01] - no dependencies
//! // Wave 1: [02, 03] - both depend only on 01
//! // Wave 2: [04] - depends on 02 and 03
//! # Ok(())
//! # }
//! ```

mod builder;
mod executor;
mod scheduler;
mod state;

pub use builder::DagBuilder;
pub use executor::{DagExecutor, ExecutionResult, ExecutorConfig, PhaseEvent};
pub use scheduler::{DagConfig, DagScheduler, PhaseNode, PhaseStatus, ReviewConfig, ReviewMode, SwarmBackend};
pub use state::{DagState, DagSummary, PhaseResult};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;

    fn create_test_phases() -> Vec<Phase> {
        vec![
            Phase::new("01", "Setup", "SETUP DONE", 5, "Initial setup", vec![]),
            Phase::new(
                "02",
                "Core A",
                "CORE A DONE",
                10,
                "Core work A",
                vec!["01".to_string()],
            ),
            Phase::new(
                "03",
                "Core B",
                "CORE B DONE",
                8,
                "Core work B",
                vec!["01".to_string()],
            ),
            Phase::new(
                "04",
                "Integration",
                "INTEGRATION DONE",
                5,
                "Integration",
                vec!["02".to_string(), "03".to_string()],
            ),
        ]
    }

    #[test]
    fn test_dag_construction() {
        let phases = create_test_phases();
        let config = DagConfig::default();
        let scheduler = DagScheduler::from_phases(&phases, config).unwrap();

        assert_eq!(scheduler.phase_count(), 4);
    }

    #[test]
    fn test_wave_computation() {
        let phases = create_test_phases();
        let config = DagConfig::default();
        let scheduler = DagScheduler::from_phases(&phases, config).unwrap();

        let waves = scheduler.compute_waves();

        // Wave 0: Phase 01 (no dependencies)
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0], vec!["01"]);
        // Wave 1: Phases 02 and 03 (both depend only on 01)
        assert!(waves[1].contains(&"02".to_string()));
        assert!(waves[1].contains(&"03".to_string()));
        // Wave 2: Phase 04 (depends on 02 and 03)
        assert_eq!(waves[2], vec!["04"]);
    }

    #[test]
    fn test_cycle_detection() {
        // Create phases with a cycle: 01 -> 02 -> 03 -> 01
        let phases = vec![
            Phase::new("01", "A", "A DONE", 5, "A", vec!["03".to_string()]),
            Phase::new("02", "B", "B DONE", 5, "B", vec!["01".to_string()]),
            Phase::new("03", "C", "C DONE", 5, "C", vec!["02".to_string()]),
        ];

        let config = DagConfig::default();
        let result = DagScheduler::from_phases(&phases, config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle") || err.contains("Cycle"));
    }

    #[test]
    fn test_missing_dependency() {
        let phases = vec![Phase::new(
            "01",
            "A",
            "A DONE",
            5,
            "A",
            vec!["nonexistent".to_string()],
        )];

        let config = DagConfig::default();
        let result = DagScheduler::from_phases(&phases, config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown dependency") || err.contains("nonexistent"));
    }
}
