//! Sub-phase delegation module for Forge.
//!
//! This module provides functionality for:
//! - Spawning sub-phases when scope discovery occurs
//! - Managing sub-phase execution flow
//! - Coordinating parent-child phase relationships
//! - Budget carving from parent phases

mod executor;
mod manager;

pub use executor::SubPhaseExecutor;
pub use manager::SubPhaseManager;

use crate::phase::{Phase, SubPhase};
use crate::signals::SubPhaseSpawnSignal;

/// Result of a sub-phase spawn request validation.
#[derive(Debug, Clone)]
pub enum SpawnValidation {
    /// The spawn request is valid and can proceed.
    Valid,
    /// The budget exceeds the parent's remaining budget.
    InsufficientBudget {
        requested: u32,
        available: u32,
    },
    /// The promise tag is invalid (e.g., empty or duplicate).
    InvalidPromise(String),
    /// The name is invalid (e.g., empty).
    InvalidName(String),
    /// Too many sub-phases already spawned.
    TooManySubPhases {
        current: usize,
        max: usize,
    },
}

impl SpawnValidation {
    /// Check if the validation passed.
    pub fn is_valid(&self) -> bool {
        matches!(self, SpawnValidation::Valid)
    }

    /// Get an error message if validation failed.
    pub fn error_message(&self) -> Option<String> {
        match self {
            SpawnValidation::Valid => None,
            SpawnValidation::InsufficientBudget { requested, available } => {
                Some(format!(
                    "Requested budget {} exceeds available budget {}",
                    requested, available
                ))
            }
            SpawnValidation::InvalidPromise(reason) => {
                Some(format!("Invalid promise: {}", reason))
            }
            SpawnValidation::InvalidName(reason) => {
                Some(format!("Invalid name: {}", reason))
            }
            SpawnValidation::TooManySubPhases { current, max } => {
                Some(format!(
                    "Too many sub-phases: {} already spawned (max {})",
                    current, max
                ))
            }
        }
    }
}

/// Configuration for sub-phase spawning behavior.
#[derive(Debug, Clone)]
pub struct SubPhaseConfig {
    /// Maximum number of sub-phases that can be spawned from a single parent.
    pub max_sub_phases: usize,
    /// Minimum budget that must remain in parent after spawning.
    pub min_parent_budget_reserve: u32,
    /// Whether to auto-approve sub-phase spawns.
    pub auto_approve_spawns: bool,
}

impl Default for SubPhaseConfig {
    fn default() -> Self {
        Self {
            max_sub_phases: 10,
            min_parent_budget_reserve: 1,
            auto_approve_spawns: false,
        }
    }
}

/// Validate a sub-phase spawn signal against the parent phase.
pub fn validate_spawn(
    signal: &SubPhaseSpawnSignal,
    parent: &Phase,
    config: &SubPhaseConfig,
) -> SpawnValidation {
    // Check name
    if signal.name.trim().is_empty() {
        return SpawnValidation::InvalidName("Name cannot be empty".to_string());
    }

    // Check promise
    if signal.promise.trim().is_empty() {
        return SpawnValidation::InvalidPromise("Promise cannot be empty".to_string());
    }

    // Check for duplicate promise in existing sub-phases
    if parent.sub_phases.iter().any(|sp| sp.promise == signal.promise) {
        return SpawnValidation::InvalidPromise(format!(
            "Promise '{}' already used by another sub-phase",
            signal.promise
        ));
    }

    // Check budget
    let available = parent.remaining_budget().saturating_sub(config.min_parent_budget_reserve);
    if signal.budget > available {
        return SpawnValidation::InsufficientBudget {
            requested: signal.budget,
            available,
        };
    }

    // Check sub-phase count
    if parent.sub_phases.len() >= config.max_sub_phases {
        return SpawnValidation::TooManySubPhases {
            current: parent.sub_phases.len(),
            max: config.max_sub_phases,
        };
    }

    SpawnValidation::Valid
}

/// Create a SubPhase from a spawn signal.
pub fn spawn_from_signal(signal: &SubPhaseSpawnSignal, parent: &Phase) -> SubPhase {
    let order = parent.sub_phases.len() as u32 + 1;
    let mut sub_phase = SubPhase::new(
        &parent.number,
        order,
        &signal.name,
        &signal.promise,
        signal.budget,
        &signal.reasoning,
    );
    sub_phase.skills = signal.skills.clone();
    sub_phase
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::SubPhaseStatus;

    fn create_test_phase() -> Phase {
        Phase::new(
            "05",
            "Authentication",
            "AUTH COMPLETE",
            20,
            "Implement authentication",
            vec![],
        )
    }

    #[test]
    fn test_validate_spawn_valid() {
        let phase = create_test_phase();
        let signal = SubPhaseSpawnSignal::new("OAuth setup", "OAUTH DONE", 5);
        let config = SubPhaseConfig::default();

        let result = validate_spawn(&signal, &phase, &config);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_spawn_insufficient_budget() {
        let phase = create_test_phase();
        let signal = SubPhaseSpawnSignal::new("Big task", "BIG DONE", 25);
        let config = SubPhaseConfig::default();

        match validate_spawn(&signal, &phase, &config) {
            SpawnValidation::InsufficientBudget { requested, available } => {
                assert_eq!(requested, 25);
                assert!(available < 25);
            }
            _ => panic!("Expected InsufficientBudget"),
        }
    }

    #[test]
    fn test_validate_spawn_empty_name() {
        let phase = create_test_phase();
        let signal = SubPhaseSpawnSignal::new("", "DONE", 5);
        let config = SubPhaseConfig::default();

        match validate_spawn(&signal, &phase, &config) {
            SpawnValidation::InvalidName(_) => {}
            _ => panic!("Expected InvalidName"),
        }
    }

    #[test]
    fn test_validate_spawn_empty_promise() {
        let phase = create_test_phase();
        let signal = SubPhaseSpawnSignal::new("Task", "", 5);
        let config = SubPhaseConfig::default();

        match validate_spawn(&signal, &phase, &config) {
            SpawnValidation::InvalidPromise(_) => {}
            _ => panic!("Expected InvalidPromise"),
        }
    }

    #[test]
    fn test_spawn_from_signal() {
        let phase = create_test_phase();
        let signal = SubPhaseSpawnSignal::with_reasoning(
            "OAuth setup",
            "OAUTH DONE",
            5,
            "OAuth needs its own phase",
        );

        let sub_phase = spawn_from_signal(&signal, &phase);

        assert_eq!(sub_phase.number, "05.1");
        assert_eq!(sub_phase.name, "OAuth setup");
        assert_eq!(sub_phase.promise, "OAUTH DONE");
        assert_eq!(sub_phase.budget, 5);
        assert_eq!(sub_phase.parent_phase, "05");
        assert_eq!(sub_phase.order, 1);
        assert_eq!(sub_phase.status, SubPhaseStatus::Pending);
    }

    #[test]
    fn test_spawn_ordering() {
        let mut phase = create_test_phase();

        // Add first sub-phase
        let signal1 = SubPhaseSpawnSignal::new("First", "FIRST DONE", 3);
        let sp1 = spawn_from_signal(&signal1, &phase);
        phase.sub_phases.push(sp1);

        // Add second sub-phase
        let signal2 = SubPhaseSpawnSignal::new("Second", "SECOND DONE", 4);
        let sp2 = spawn_from_signal(&signal2, &phase);
        phase.sub_phases.push(sp2);

        assert_eq!(phase.sub_phases[0].number, "05.1");
        assert_eq!(phase.sub_phases[1].number, "05.2");
        assert_eq!(phase.sub_phases[0].order, 1);
        assert_eq!(phase.sub_phases[1].order, 2);
    }
}
