//! Phase definition and JSON loading for the forge orchestrator.
//!
//! This module provides:
//! - `Phase` struct representing a single implementation phase
//! - `SubPhase` struct for dynamically spawned child phases
//! - `PhasesFile` struct representing the full phases.json format
//! - Loading functions for JSON-based phase configuration
//! - Default IdCheck phases as a fallback

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::forge_config::PermissionMode;
use crate::review::SpecialistType;

/// Phase type for TDD workflow.
/// Used by `forge implement` to distinguish test phases from implementation phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseType {
    /// Test phase - writes tests first
    Test,
    /// Implement phase - writes implementation code
    Implement,
}

/// Configuration for review specialists on a phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseReviewSettings {
    /// Review specialists to invoke after phase completion.
    #[serde(default)]
    pub specialists: Vec<PhaseSpecialistConfig>,
    /// Whether to run reviews in parallel.
    #[serde(default = "default_parallel")]
    pub parallel: bool,
}

fn default_parallel() -> bool {
    true
}

impl Default for PhaseReviewSettings {
    fn default() -> Self {
        Self {
            specialists: Vec::new(),
            parallel: true,
        }
    }
}

impl PhaseReviewSettings {
    /// Check if any review specialists are configured.
    pub fn is_empty(&self) -> bool {
        self.specialists.is_empty()
    }

    /// Check if any gating specialists are configured.
    pub fn has_gating(&self) -> bool {
        self.specialists.iter().any(|s| s.gate)
    }
}

/// Configuration for a single review specialist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhaseSpecialistConfig {
    /// Type of specialist.
    pub specialist_type: SpecialistType,
    /// Whether this review gates phase completion.
    #[serde(default)]
    pub gate: bool,
    /// Custom focus areas (optional).
    #[serde(default)]
    pub focus_areas: Vec<String>,
}

/// Represents a single implementation phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Phase {
    /// Phase number (e.g., "01", "02")
    pub number: String,
    /// Human-readable name of the phase
    pub name: String,
    /// Promise tag that indicates phase completion
    pub promise: String,
    /// Maximum iterations (budget) allowed for this phase
    pub budget: u32,
    /// Reasoning for why this phase exists and what it accomplishes
    #[serde(default)]
    pub reasoning: String,
    /// List of phase numbers that this phase depends on
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// List of skill names to load for this phase
    #[serde(default)]
    pub skills: Vec<String>,
    /// Permission mode for this phase (defaults to Standard)
    /// - Standard: Approve phase start, auto-continue iterations
    /// - Autonomous: Auto-approve if within budget and making progress
    /// - Readonly: Planning/research phases, no file modifications
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// Parent phase number for sub-phases (e.g., "05" for sub-phase "05.1")
    /// None for top-level phases, Some("05") for sub-phases of phase 05
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_phase: Option<String>,
    /// Sub-phases spawned from this phase during execution
    /// These are populated dynamically during orchestration
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_phases: Vec<SubPhase>,
    /// Phase type for TDD workflow (test or implement)
    /// Used by `forge implement` to distinguish test phases from implementation phases
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_type: Option<PhaseType>,
    /// Review specialists configuration for post-phase quality gates
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviews: Option<PhaseReviewSettings>,
}

/// Represents a sub-phase that is dynamically spawned from a parent phase.
/// Sub-phases are created when a phase discovers its scope is larger than expected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubPhase {
    /// Sub-phase number (e.g., "05.1", "05.2")
    pub number: String,
    /// Human-readable name of the sub-phase
    pub name: String,
    /// Promise tag that indicates sub-phase completion
    pub promise: String,
    /// Budget carved from parent's remaining budget
    pub budget: u32,
    /// Reasoning for why this sub-phase was spawned
    #[serde(default)]
    pub reasoning: String,
    /// Parent phase number (e.g., "05")
    pub parent_phase: String,
    /// Order within parent's sub-phases (1, 2, 3, ...)
    pub order: u32,
    /// List of skill names to load for this sub-phase (inherits from parent by default)
    #[serde(default)]
    pub skills: Vec<String>,
    /// Permission mode (inherits from parent by default)
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// Status of the sub-phase
    #[serde(default)]
    pub status: SubPhaseStatus,
}

/// Status of a sub-phase in the execution lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubPhaseStatus {
    /// Sub-phase is waiting to be executed
    #[default]
    Pending,
    /// Sub-phase is currently being executed
    InProgress,
    /// Sub-phase completed successfully
    Completed,
    /// Sub-phase failed (max iterations reached or error)
    Failed,
    /// Sub-phase was skipped
    Skipped,
}

impl SubPhase {
    /// Create a new sub-phase with the given parameters.
    pub fn new(
        parent_phase: &str,
        order: u32,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
    ) -> Self {
        Self {
            number: format!("{}.{}", parent_phase, order),
            name: name.to_string(),
            promise: promise.to_string(),
            budget,
            reasoning: reasoning.to_string(),
            parent_phase: parent_phase.to_string(),
            order,
            skills: Vec::new(),
            permission_mode: PermissionMode::default(),
            status: SubPhaseStatus::Pending,
        }
    }

    /// Convert this sub-phase to a full Phase for execution.
    /// The resulting Phase inherits parent properties where applicable.
    pub fn to_phase(&self, parent: &Phase) -> Phase {
        Phase {
            number: self.number.clone(),
            name: self.name.clone(),
            promise: self.promise.clone(),
            budget: self.budget,
            reasoning: self.reasoning.clone(),
            depends_on: vec![parent.number.clone()],
            skills: if self.skills.is_empty() {
                parent.skills.clone()
            } else {
                self.skills.clone()
            },
            permission_mode: if self.permission_mode == PermissionMode::default() {
                parent.permission_mode
            } else {
                self.permission_mode
            },
            parent_phase: Some(parent.number.clone()),
            sub_phases: Vec::new(),
            phase_type: parent.phase_type,
            reviews: parent.reviews.clone(),
        }
    }

    /// Check if this sub-phase is pending execution.
    pub fn is_pending(&self) -> bool {
        self.status == SubPhaseStatus::Pending
    }

    /// Check if this sub-phase is complete (success or failure).
    pub fn is_complete(&self) -> bool {
        matches!(
            self.status,
            SubPhaseStatus::Completed | SubPhaseStatus::Failed | SubPhaseStatus::Skipped
        )
    }
}

impl Phase {
    /// Create a new Phase with all fields.
    pub fn new(
        number: &str,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
        depends_on: Vec<String>,
    ) -> Self {
        Self {
            number: number.to_string(),
            name: name.to_string(),
            promise: promise.to_string(),
            budget,
            reasoning: reasoning.to_string(),
            depends_on,
            skills: Vec::new(),
            permission_mode: PermissionMode::default(),
            parent_phase: None,
            sub_phases: Vec::new(),
            phase_type: None,
            reviews: None,
        }
    }

    /// Create a new Phase with skills.
    pub fn with_skills(
        number: &str,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
        depends_on: Vec<String>,
        skills: Vec<String>,
    ) -> Self {
        Self {
            number: number.to_string(),
            name: name.to_string(),
            promise: promise.to_string(),
            budget,
            reasoning: reasoning.to_string(),
            depends_on,
            skills,
            permission_mode: PermissionMode::default(),
            parent_phase: None,
            sub_phases: Vec::new(),
            phase_type: None,
            reviews: None,
        }
    }

    /// Create a new Phase with permission mode.
    pub fn with_permission_mode(
        number: &str,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
        depends_on: Vec<String>,
        permission_mode: PermissionMode,
    ) -> Self {
        Self {
            number: number.to_string(),
            name: name.to_string(),
            promise: promise.to_string(),
            budget,
            reasoning: reasoning.to_string(),
            depends_on,
            skills: Vec::new(),
            permission_mode,
            parent_phase: None,
            sub_phases: Vec::new(),
            phase_type: None,
            reviews: None,
        }
    }

    /// Backward-compatible accessor for max_iterations (now called budget).
    #[inline]
    pub fn max_iterations(&self) -> u32 {
        self.budget
    }

    /// Backward-compatible accessor for description (now called name).
    #[inline]
    pub fn description(&self) -> &str {
        &self.name
    }

    /// Check if this phase is a sub-phase (has a parent).
    #[inline]
    pub fn is_sub_phase(&self) -> bool {
        self.parent_phase.is_some()
    }

    /// Check if this phase has any sub-phases.
    #[inline]
    pub fn has_sub_phases(&self) -> bool {
        !self.sub_phases.is_empty()
    }

    /// Get pending sub-phases that haven't been executed yet.
    pub fn pending_sub_phases(&self) -> Vec<&SubPhase> {
        self.sub_phases
            .iter()
            .filter(|sp| sp.is_pending())
            .collect()
    }

    /// Get all sub-phases that are not yet complete.
    pub fn incomplete_sub_phases(&self) -> Vec<&SubPhase> {
        self.sub_phases
            .iter()
            .filter(|sp| !sp.is_complete())
            .collect()
    }

    /// Add a new sub-phase to this phase.
    /// Returns the sub-phase number that was assigned.
    pub fn add_sub_phase(
        &mut self,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
    ) -> String {
        let order = self.sub_phases.len() as u32 + 1;
        let sub_phase = SubPhase::new(&self.number, order, name, promise, budget, reasoning);
        let number = sub_phase.number.clone();
        self.sub_phases.push(sub_phase);
        number
    }

    /// Add a sub-phase with specific skills.
    pub fn add_sub_phase_with_skills(
        &mut self,
        name: &str,
        promise: &str,
        budget: u32,
        reasoning: &str,
        skills: Vec<String>,
    ) -> String {
        let order = self.sub_phases.len() as u32 + 1;
        let mut sub_phase = SubPhase::new(&self.number, order, name, promise, budget, reasoning);
        sub_phase.skills = skills;
        let number = sub_phase.number.clone();
        self.sub_phases.push(sub_phase);
        number
    }

    /// Get a mutable reference to a sub-phase by number.
    pub fn get_sub_phase_mut(&mut self, number: &str) -> Option<&mut SubPhase> {
        self.sub_phases.iter_mut().find(|sp| sp.number == number)
    }

    /// Get an immutable reference to a sub-phase by number.
    pub fn get_sub_phase(&self, number: &str) -> Option<&SubPhase> {
        self.sub_phases.iter().find(|sp| sp.number == number)
    }

    /// Update a sub-phase's status.
    pub fn update_sub_phase_status(&mut self, number: &str, status: SubPhaseStatus) -> bool {
        if let Some(sp) = self.get_sub_phase_mut(number) {
            sp.status = status;
            true
        } else {
            false
        }
    }

    /// Calculate remaining budget after accounting for sub-phase allocations.
    pub fn remaining_budget(&self) -> u32 {
        let allocated: u32 = self.sub_phases.iter().map(|sp| sp.budget).sum();
        self.budget.saturating_sub(allocated)
    }

    /// Check if all sub-phases are complete.
    pub fn all_sub_phases_complete(&self) -> bool {
        self.sub_phases.is_empty() || self.sub_phases.iter().all(|sp| sp.is_complete())
    }

    /// Check if this phase has review configuration.
    pub fn has_reviews(&self) -> bool {
        self.reviews.as_ref().is_some_and(|r| !r.is_empty())
    }

    /// Check if this phase has gating reviews.
    pub fn has_gating_reviews(&self) -> bool {
        self.reviews.as_ref().is_some_and(|r| r.has_gating())
    }

    /// Get the review settings for this phase.
    pub fn review_settings(&self) -> Option<&PhaseReviewSettings> {
        self.reviews.as_ref()
    }

    /// Get the next sub-phase to execute.
    pub fn next_sub_phase(&self) -> Option<&SubPhase> {
        self.sub_phases.iter().find(|sp| sp.is_pending())
    }

    /// Convert all sub-phases to executable Phase objects.
    pub fn sub_phases_as_phases(&self) -> Vec<Phase> {
        self.sub_phases.iter().map(|sp| sp.to_phase(self)).collect()
    }
}

/// Represents the full phases.json file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhasesFile {
    /// Hash of the spec file used to generate these phases
    pub spec_hash: String,
    /// Timestamp when phases were generated
    pub generated_at: String,
    /// List of phases
    pub phases: Vec<Phase>,
}

impl PhasesFile {
    /// Load phases from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read phases file: {}", path.display()))?;

        let phases_file: PhasesFile = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse phases JSON: {}", path.display()))?;

        Ok(phases_file)
    }

    /// Save phases to a JSON file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize phases to JSON")?;

        std::fs::write(path, content)
            .with_context(|| format!("Failed to write phases file: {}", path.display()))?;

        Ok(())
    }

    /// Get all phases.
    pub fn get_all_phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Get a specific phase by number.
    pub fn get_phase(&self, number: &str) -> Option<&Phase> {
        self.phases.iter().find(|p| p.number == number)
    }

    /// Get a mutable reference to a specific phase by number.
    pub fn get_phase_mut(&mut self, number: &str) -> Option<&mut Phase> {
        self.phases.iter_mut().find(|p| p.number == number)
    }

    /// Get phases starting from a given phase number.
    pub fn get_phases_from(&self, start: &str) -> Vec<&Phase> {
        self.phases
            .iter()
            .filter(|p| p.number.as_str() >= start)
            .collect()
    }

    /// Get a sub-phase by its full number (e.g., "05.1").
    pub fn get_sub_phase(&self, number: &str) -> Option<(&Phase, &SubPhase)> {
        // Parse parent phase number from sub-phase number
        if let Some(dot_pos) = number.find('.') {
            let parent_number = &number[..dot_pos];
            if let Some(parent) = self.get_phase(parent_number)
                && let Some(sub_phase) = parent.get_sub_phase(number)
            {
                return Some((parent, sub_phase));
            }
        }
        None
    }

    /// Get a mutable reference to a sub-phase by its full number.
    pub fn get_sub_phase_mut(&mut self, number: &str) -> Option<&mut SubPhase> {
        if let Some(dot_pos) = number.find('.') {
            let parent_number = &number[..dot_pos];
            if let Some(parent) = self.get_phase_mut(parent_number) {
                return parent.get_sub_phase_mut(number);
            }
        }
        None
    }

    /// Add sub-phases to a parent phase.
    /// Returns the sub-phase numbers that were assigned.
    pub fn add_sub_phases_to_phase(
        &mut self,
        parent_number: &str,
        sub_phases: Vec<(String, String, u32, String)>, // (name, promise, budget, reasoning)
    ) -> Result<Vec<String>> {
        let parent = self
            .get_phase_mut(parent_number)
            .ok_or_else(|| anyhow::anyhow!("Parent phase {} not found", parent_number))?;

        let numbers: Vec<String> = sub_phases
            .into_iter()
            .map(|(name, promise, budget, reasoning)| {
                parent.add_sub_phase(&name, &promise, budget, &reasoning)
            })
            .collect();

        Ok(numbers)
    }

    /// Update a sub-phase's status.
    pub fn update_sub_phase_status(&mut self, number: &str, status: SubPhaseStatus) -> bool {
        if let Some(sub_phase) = self.get_sub_phase_mut(number) {
            sub_phase.status = status;
            true
        } else {
            false
        }
    }

    /// Get all phases including sub-phases as Phase objects.
    /// This flattens the hierarchy for execution purposes.
    pub fn get_all_phases_flattened(&self) -> Vec<Phase> {
        let mut result = Vec::new();
        for phase in &self.phases {
            result.push(phase.clone());
            for sub_phase in &phase.sub_phases {
                result.push(sub_phase.to_phase(phase));
            }
        }
        result
    }

    /// Count total phases including sub-phases.
    pub fn total_phase_count(&self) -> usize {
        self.phases.len()
            + self
                .phases
                .iter()
                .map(|p| p.sub_phases.len())
                .sum::<usize>()
    }
}

/// Get the default IdCheck phases as a fallback.
/// This provides backward compatibility when phases.json doesn't exist.
pub fn get_default_phases() -> Vec<Phase> {
    vec![
        Phase::new(
            "01",
            "Project scaffolding",
            "SCAFFOLD COMPLETE",
            12,
            "Set up the basic project structure",
            vec![],
        ),
        Phase::new(
            "02",
            "Database schema and migrations",
            "MIGRATIONS COMPLETE",
            15,
            "Create database schema",
            vec!["01".into()],
        ),
        Phase::new(
            "03",
            "Configuration module",
            "CONFIG COMPLETE",
            10,
            "Set up configuration handling",
            vec!["01".into()],
        ),
        Phase::new(
            "04",
            "JWT service",
            "JWT COMPLETE",
            15,
            "Implement JWT token service",
            vec!["03".into()],
        ),
        Phase::new(
            "05",
            "Password hashing",
            "HASHING COMPLETE",
            10,
            "Implement secure password hashing",
            vec!["03".into()],
        ),
        Phase::new(
            "06",
            "Health endpoints",
            "HEALTH COMPLETE",
            8,
            "Add health check endpoints",
            vec!["01".into()],
        ),
        Phase::new(
            "07",
            "Auth: register and login",
            "AUTH BASIC COMPLETE",
            20,
            "Basic authentication flow",
            vec!["02".into(), "04".into(), "05".into()],
        ),
        Phase::new(
            "08",
            "Auth: refresh token rotation",
            "REFRESH COMPLETE",
            15,
            "Token refresh mechanism",
            vec!["07".into()],
        ),
        Phase::new(
            "09",
            "Auth: magic links",
            "MAGIC LINK COMPLETE",
            15,
            "Passwordless magic link auth",
            vec!["07".into()],
        ),
        Phase::new(
            "10",
            "Auth: OTP",
            "OTP COMPLETE",
            15,
            "One-time password authentication",
            vec!["07".into()],
        ),
        Phase::new(
            "11",
            "OAuth: Google with PKCE",
            "OAUTH GOOGLE COMPLETE",
            20,
            "Google OAuth integration",
            vec!["07".into()],
        ),
        Phase::new(
            "12",
            "OAuth: all providers",
            "OAUTH ALL COMPLETE",
            25,
            "All OAuth providers",
            vec!["11".into()],
        ),
        Phase::new(
            "13",
            "User endpoints",
            "USER ENDPOINTS COMPLETE",
            12,
            "User management API",
            vec!["07".into()],
        ),
        Phase::new(
            "14",
            "Teams and membership",
            "TEAMS COMPLETE",
            20,
            "Team functionality",
            vec!["13".into()],
        ),
        Phase::new(
            "15",
            "Team invitations",
            "INVITATIONS COMPLETE",
            15,
            "Team invitation system",
            vec!["14".into()],
        ),
        Phase::new(
            "16",
            "Admin API endpoints",
            "ADMIN API COMPLETE",
            15,
            "Admin management API",
            vec!["13".into()],
        ),
        Phase::new(
            "17",
            "Rate limiting and lockout",
            "SECURITY COMPLETE",
            15,
            "Security features",
            vec!["07".into()],
        ),
        Phase::new(
            "18",
            "Audit logging",
            "AUDIT COMPLETE",
            12,
            "Audit trail system",
            vec!["07".into()],
        ),
        Phase::new(
            "19",
            "Background cleanup jobs",
            "CLEANUP COMPLETE",
            10,
            "Background job system",
            vec!["02".into()],
        ),
        Phase::new(
            "20",
            "Admin dashboard",
            "DASHBOARD COMPLETE",
            30,
            "Admin web interface",
            vec!["16".into()],
        ),
        Phase::new(
            "21",
            "Docker and graceful shutdown",
            "DOCKER COMPLETE",
            12,
            "Containerization",
            vec!["06".into()],
        ),
        Phase::new(
            "22",
            "Integration testing",
            "INTEGRATION COMPLETE",
            20,
            "Full integration tests",
            vec!["01".into()],
        ),
    ]
}

/// Get all default phases (convenience function).
pub fn get_all_phases() -> Vec<Phase> {
    get_default_phases()
}

/// Get a specific default phase by number.
pub fn get_phase(number: &str) -> Option<Phase> {
    get_default_phases()
        .into_iter()
        .find(|p| p.number == number)
}

/// Get default phases starting from a given phase number.
pub fn get_phases_from(start: &str) -> Vec<Phase> {
    get_default_phases()
        .into_iter()
        .filter(|p| p.number.as_str() >= start)
        .collect()
}

/// Try to load phases from a file, falling back to defaults if not found.
pub fn load_phases_or_default(phases_file: Option<&Path>) -> Result<Vec<Phase>> {
    match phases_file {
        Some(path) if path.exists() => {
            let pf = PhasesFile::load(path)?;
            Ok(pf.phases)
        }
        _ => Ok(get_default_phases()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // =========================================
    // Phase struct tests
    // =========================================

    #[test]
    fn test_phase_new() {
        let phase = Phase::new(
            "01",
            "Project scaffolding",
            "SCAFFOLD COMPLETE",
            12,
            "Set up the basic structure",
            vec!["00".into()],
        );

        assert_eq!(phase.number, "01");
        assert_eq!(phase.name, "Project scaffolding");
        assert_eq!(phase.promise, "SCAFFOLD COMPLETE");
        assert_eq!(phase.budget, 12);
        assert_eq!(phase.reasoning, "Set up the basic structure");
        assert_eq!(phase.depends_on, vec!["00"]);
    }

    #[test]
    fn test_phase_backward_compat_accessors() {
        let phase = Phase::new("01", "Test", "DONE", 10, "reason", vec![]);

        // max_iterations should return budget
        assert_eq!(phase.max_iterations(), 10);

        // description should return name
        assert_eq!(phase.description(), "Test");
    }

    #[test]
    fn test_phase_serialization() {
        let phase = Phase::new(
            "01",
            "Project scaffolding",
            "SCAFFOLD COMPLETE",
            12,
            "Set up structure",
            vec!["00".into()],
        );

        let json = serde_json::to_string(&phase).unwrap();
        let parsed: Phase = serde_json::from_str(&json).unwrap();

        assert_eq!(phase, parsed);
    }

    #[test]
    fn test_phase_deserialization_with_defaults() {
        // Test that missing optional fields get defaults
        let json = r#"{
            "number": "01",
            "name": "Test",
            "promise": "DONE",
            "budget": 5
        }"#;

        let phase: Phase = serde_json::from_str(json).unwrap();

        assert_eq!(phase.reasoning, "");
        assert!(phase.depends_on.is_empty());
        // Permission mode defaults to Standard
        assert_eq!(phase.permission_mode, PermissionMode::Standard);
    }

    #[test]
    fn test_phase_with_permission_mode() {
        let phase = Phase::with_permission_mode(
            "01",
            "Database migration",
            "MIGRATION DONE",
            10,
            "Run database migrations",
            vec![],
            PermissionMode::Standard,
        );

        assert_eq!(phase.permission_mode, PermissionMode::Standard);
        assert!(phase.skills.is_empty());
    }

    #[test]
    fn test_phase_permission_mode_serialization() {
        // Test serializing and deserializing phases with different permission modes
        let phase = Phase::with_permission_mode(
            "01",
            "Research",
            "RESEARCH DONE",
            5,
            "Explore options",
            vec![],
            PermissionMode::Readonly,
        );

        let json = serde_json::to_string(&phase).unwrap();
        assert!(json.contains("\"permission_mode\":\"readonly\""));

        let parsed: Phase = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.permission_mode, PermissionMode::Readonly);
    }

    #[test]
    fn test_phase_permission_mode_from_json() {
        // Test parsing permission_mode from JSON
        let json = r#"{
            "number": "07",
            "name": "Database migration",
            "promise": "MIGRATION COMPLETE",
            "budget": 12,
            "permission_mode": "strict"
        }"#;

        let phase: Phase = serde_json::from_str(json).unwrap();
        assert_eq!(phase.permission_mode, PermissionMode::Standard); // "strict" maps to Standard

        // Test autonomous mode
        let json_autonomous = r#"{
            "number": "08",
            "name": "Test automation",
            "promise": "TESTS COMPLETE",
            "budget": 20,
            "permission_mode": "autonomous"
        }"#;

        let phase_auto: Phase = serde_json::from_str(json_autonomous).unwrap();
        assert_eq!(phase_auto.permission_mode, PermissionMode::Autonomous);
    }

    // =========================================
    // PhasesFile tests
    // =========================================

    fn create_test_phases_json() -> String {
        r#"{
            "spec_hash": "abc123def456",
            "generated_at": "2026-01-23T12:00:00Z",
            "phases": [
                {
                    "number": "01",
                    "name": "Project Scaffold",
                    "promise": "SCAFFOLD COMPLETE",
                    "budget": 8,
                    "reasoning": "Initial project setup",
                    "depends_on": []
                },
                {
                    "number": "02",
                    "name": "Database Setup",
                    "promise": "DB COMPLETE",
                    "budget": 10,
                    "reasoning": "Set up database schema",
                    "depends_on": ["01"]
                },
                {
                    "number": "03",
                    "name": "API Layer",
                    "promise": "API COMPLETE",
                    "budget": 15,
                    "reasoning": "Build API endpoints",
                    "depends_on": ["01", "02"]
                }
            ]
        }"#
        .to_string()
    }

    #[test]
    fn test_phases_file_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, create_test_phases_json()).unwrap();

        let pf = PhasesFile::load(&path).unwrap();

        assert_eq!(pf.spec_hash, "abc123def456");
        assert_eq!(pf.generated_at, "2026-01-23T12:00:00Z");
        assert_eq!(pf.phases.len(), 3);
        assert_eq!(pf.phases[0].number, "01");
        assert_eq!(pf.phases[0].name, "Project Scaffold");
    }

    #[test]
    fn test_phases_file_load_not_found() {
        let result = PhasesFile::load(Path::new("/nonexistent/path/phases.json"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read phases file")
        );
    }

    #[test]
    fn test_phases_file_load_invalid_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, "{ invalid json }").unwrap();

        let result = PhasesFile::load(&path);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to parse phases JSON")
        );
    }

    #[test]
    fn test_phases_file_save() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");

        let pf = PhasesFile {
            spec_hash: "test123".to_string(),
            generated_at: "2026-01-23T12:00:00Z".to_string(),
            phases: vec![Phase::new(
                "01",
                "Test Phase",
                "TEST DONE",
                5,
                "Testing",
                vec![],
            )],
        };

        pf.save(&path).unwrap();

        // Load it back and verify
        let loaded = PhasesFile::load(&path).unwrap();
        assert_eq!(loaded.spec_hash, "test123");
        assert_eq!(loaded.phases.len(), 1);
        assert_eq!(loaded.phases[0].number, "01");
    }

    #[test]
    fn test_phases_file_get_all_phases() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, create_test_phases_json()).unwrap();

        let pf = PhasesFile::load(&path).unwrap();
        let phases = pf.get_all_phases();

        assert_eq!(phases.len(), 3);
    }

    #[test]
    fn test_phases_file_get_phase() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, create_test_phases_json()).unwrap();

        let pf = PhasesFile::load(&path).unwrap();

        let phase = pf.get_phase("02").unwrap();
        assert_eq!(phase.name, "Database Setup");
        assert_eq!(phase.promise, "DB COMPLETE");

        let missing = pf.get_phase("99");
        assert!(missing.is_none());
    }

    #[test]
    fn test_phases_file_get_phases_from() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, create_test_phases_json()).unwrap();

        let pf = PhasesFile::load(&path).unwrap();
        let phases = pf.get_phases_from("02");

        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].number, "02");
        assert_eq!(phases[1].number, "03");
    }

    // =========================================
    // Default phases tests
    // =========================================

    #[test]
    fn test_get_default_phases() {
        let phases = get_default_phases();
        assert_eq!(phases.len(), 22);
        assert_eq!(phases[0].number, "01");
        assert_eq!(phases[21].number, "22");
    }

    #[test]
    fn test_get_all_phases() {
        let phases = get_all_phases();
        assert_eq!(phases.len(), 22);
        assert_eq!(phases[0].number, "01");
    }

    #[test]
    fn test_get_phase() {
        let phase = get_phase("07").unwrap();
        assert_eq!(phase.promise, "AUTH BASIC COMPLETE");
        assert_eq!(phase.budget, 20);
        assert_eq!(phase.name, "Auth: register and login");
    }

    #[test]
    fn test_get_phases_from() {
        let phases = get_phases_from("20");
        assert_eq!(phases.len(), 3);
        assert_eq!(phases[0].number, "20");
    }

    // =========================================
    // load_phases_or_default tests
    // =========================================

    #[test]
    fn test_load_phases_or_default_with_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phases.json");
        fs::write(&path, create_test_phases_json()).unwrap();

        let phases = load_phases_or_default(Some(&path)).unwrap();

        // Should load from file
        assert_eq!(phases.len(), 3);
        assert_eq!(phases[0].name, "Project Scaffold");
    }

    #[test]
    fn test_load_phases_or_default_file_not_found() {
        let path = Path::new("/nonexistent/phases.json");

        let phases = load_phases_or_default(Some(path)).unwrap();

        // Should fall back to defaults
        assert_eq!(phases.len(), 22);
        assert_eq!(phases[0].name, "Project scaffolding");
    }

    #[test]
    fn test_load_phases_or_default_none() {
        let phases = load_phases_or_default(None).unwrap();

        // Should return defaults
        assert_eq!(phases.len(), 22);
    }

    // =========================================
    // Field mapping verification tests
    // =========================================

    #[test]
    fn test_field_mapping_description_to_name() {
        // Verify old 'description' is now accessed via 'name'
        let phase = get_phase("01").unwrap();
        assert_eq!(phase.name, "Project scaffolding");
        assert_eq!(phase.description(), "Project scaffolding");
    }

    #[test]
    fn test_field_mapping_max_iterations_to_budget() {
        // Verify old 'max_iterations' is now accessed via 'budget'
        let phase = get_phase("01").unwrap();
        assert_eq!(phase.budget, 12);
        assert_eq!(phase.max_iterations(), 12);
    }

    // =========================================
    // Sub-phase tests
    // =========================================

    #[test]
    fn test_sub_phase_new() {
        let sub = SubPhase::new("05", 1, "OAuth setup", "OAUTH DONE", 5, "OAuth is complex");

        assert_eq!(sub.number, "05.1");
        assert_eq!(sub.name, "OAuth setup");
        assert_eq!(sub.promise, "OAUTH DONE");
        assert_eq!(sub.budget, 5);
        assert_eq!(sub.parent_phase, "05");
        assert_eq!(sub.order, 1);
        assert_eq!(sub.status, SubPhaseStatus::Pending);
    }

    #[test]
    fn test_sub_phase_to_phase() {
        let parent = Phase::new(
            "05",
            "Authentication",
            "AUTH COMPLETE",
            20,
            "Implement auth",
            vec![],
        );
        let sub = SubPhase::new("05", 1, "OAuth setup", "OAUTH DONE", 5, "OAuth is complex");

        let phase = sub.to_phase(&parent);

        assert_eq!(phase.number, "05.1");
        assert_eq!(phase.name, "OAuth setup");
        assert_eq!(phase.promise, "OAUTH DONE");
        assert_eq!(phase.budget, 5);
        assert_eq!(phase.parent_phase, Some("05".to_string()));
        assert!(phase.is_sub_phase());
    }

    #[test]
    fn test_sub_phase_status_lifecycle() {
        let sub = SubPhase::new("05", 1, "Task", "DONE", 3, "reason");

        assert!(sub.is_pending());
        assert!(!sub.is_complete());

        // Test all terminal states
        let mut completed = sub.clone();
        completed.status = SubPhaseStatus::Completed;
        assert!(completed.is_complete());

        let mut failed = sub.clone();
        failed.status = SubPhaseStatus::Failed;
        assert!(failed.is_complete());

        let mut skipped = sub.clone();
        skipped.status = SubPhaseStatus::Skipped;
        assert!(skipped.is_complete());

        let mut in_progress = sub.clone();
        in_progress.status = SubPhaseStatus::InProgress;
        assert!(!in_progress.is_pending());
        assert!(!in_progress.is_complete());
    }

    #[test]
    fn test_phase_add_sub_phase() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);

        let num1 = phase.add_sub_phase("OAuth", "OAUTH DONE", 5, "OAuth setup");
        let num2 = phase.add_sub_phase("JWT", "JWT DONE", 4, "JWT handling");

        assert_eq!(num1, "05.1");
        assert_eq!(num2, "05.2");
        assert_eq!(phase.sub_phases.len(), 2);
        assert!(phase.has_sub_phases());
    }

    #[test]
    fn test_phase_remaining_budget() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);

        assert_eq!(phase.remaining_budget(), 20);

        phase.add_sub_phase("OAuth", "OAUTH DONE", 5, "reason");
        assert_eq!(phase.remaining_budget(), 15);

        phase.add_sub_phase("JWT", "JWT DONE", 8, "reason");
        assert_eq!(phase.remaining_budget(), 7);
    }

    #[test]
    fn test_phase_sub_phase_completion() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);

        // No sub-phases = all complete
        assert!(phase.all_sub_phases_complete());

        phase.add_sub_phase("OAuth", "OAUTH DONE", 5, "reason");
        phase.add_sub_phase("JWT", "JWT DONE", 4, "reason");

        // Has pending sub-phases = not complete
        assert!(!phase.all_sub_phases_complete());

        // Complete first sub-phase
        phase.update_sub_phase_status("05.1", SubPhaseStatus::Completed);
        assert!(!phase.all_sub_phases_complete());

        // Complete second sub-phase
        phase.update_sub_phase_status("05.2", SubPhaseStatus::Completed);
        assert!(phase.all_sub_phases_complete());
    }

    #[test]
    fn test_phase_get_sub_phase() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);
        phase.add_sub_phase("OAuth", "OAUTH DONE", 5, "reason");

        let sub = phase.get_sub_phase("05.1");
        assert!(sub.is_some());
        assert_eq!(sub.unwrap().name, "OAuth");

        let missing = phase.get_sub_phase("05.99");
        assert!(missing.is_none());
    }

    #[test]
    fn test_phase_next_sub_phase() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);

        // No sub-phases
        assert!(phase.next_sub_phase().is_none());

        phase.add_sub_phase("First", "FIRST DONE", 3, "reason");
        phase.add_sub_phase("Second", "SECOND DONE", 4, "reason");

        // First pending is returned
        let next = phase.next_sub_phase();
        assert!(next.is_some());
        assert_eq!(next.unwrap().name, "First");

        // Mark first as complete
        phase.update_sub_phase_status("05.1", SubPhaseStatus::Completed);

        // Now second is next
        let next = phase.next_sub_phase();
        assert!(next.is_some());
        assert_eq!(next.unwrap().name, "Second");

        // Mark second as complete
        phase.update_sub_phase_status("05.2", SubPhaseStatus::Completed);

        // No more pending
        assert!(phase.next_sub_phase().is_none());
    }

    #[test]
    fn test_phase_sub_phases_as_phases() {
        let mut parent = Phase::with_skills(
            "05",
            "Auth",
            "AUTH DONE",
            20,
            "reason",
            vec![],
            vec!["rust-conventions".to_string()],
        );
        parent.add_sub_phase("OAuth", "OAUTH DONE", 5, "reason");
        parent.add_sub_phase("JWT", "JWT DONE", 4, "reason");

        let phases = parent.sub_phases_as_phases();

        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].number, "05.1");
        assert_eq!(phases[1].number, "05.2");

        // Skills are inherited
        assert_eq!(phases[0].skills, vec!["rust-conventions"]);
    }

    #[test]
    fn test_phases_file_sub_phase_support() {
        let mut pf = PhasesFile {
            spec_hash: "test".to_string(),
            generated_at: "2026-01-24".to_string(),
            phases: vec![Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![])],
        };

        // Add sub-phases
        let numbers = pf
            .add_sub_phases_to_phase(
                "05",
                vec![
                    (
                        "OAuth".to_string(),
                        "OAUTH DONE".to_string(),
                        5,
                        "reason".to_string(),
                    ),
                    (
                        "JWT".to_string(),
                        "JWT DONE".to_string(),
                        4,
                        "reason".to_string(),
                    ),
                ],
            )
            .unwrap();

        assert_eq!(numbers, vec!["05.1", "05.2"]);

        // Get sub-phase
        let (parent, sub) = pf.get_sub_phase("05.1").unwrap();
        assert_eq!(parent.number, "05");
        assert_eq!(sub.name, "OAuth");

        // Update status
        assert!(pf.update_sub_phase_status("05.1", SubPhaseStatus::Completed));
        let (_, sub) = pf.get_sub_phase("05.1").unwrap();
        assert_eq!(sub.status, SubPhaseStatus::Completed);

        // Total count
        assert_eq!(pf.total_phase_count(), 3); // 1 parent + 2 sub-phases
    }

    #[test]
    fn test_phases_file_flattened() {
        let mut pf = PhasesFile {
            spec_hash: "test".to_string(),
            generated_at: "2026-01-24".to_string(),
            phases: vec![
                Phase::new("01", "First", "FIRST DONE", 10, "reason", vec![]),
                Phase::new("02", "Second", "SECOND DONE", 15, "reason", vec![]),
            ],
        };

        pf.add_sub_phases_to_phase(
            "01",
            vec![(
                "Sub A".to_string(),
                "A DONE".to_string(),
                3,
                "reason".to_string(),
            )],
        )
        .unwrap();

        let flattened = pf.get_all_phases_flattened();

        assert_eq!(flattened.len(), 3);
        assert_eq!(flattened[0].number, "01");
        assert_eq!(flattened[1].number, "01.1");
        assert_eq!(flattened[2].number, "02");
    }

    #[test]
    fn test_sub_phase_serialization() {
        let sub = SubPhase::new("05", 1, "OAuth", "OAUTH DONE", 5, "OAuth is complex");

        let json = serde_json::to_string(&sub).unwrap();
        let parsed: SubPhase = serde_json::from_str(&json).unwrap();

        assert_eq!(sub.number, parsed.number);
        assert_eq!(sub.name, parsed.name);
        assert_eq!(sub.status, parsed.status);
    }

    #[test]
    fn test_phase_with_sub_phases_serialization() {
        let mut phase = Phase::new("05", "Auth", "AUTH DONE", 20, "reason", vec![]);
        phase.add_sub_phase("OAuth", "OAUTH DONE", 5, "reason");

        let json = serde_json::to_string(&phase).unwrap();
        let parsed: Phase = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.sub_phases.len(), 1);
        assert_eq!(parsed.sub_phases[0].number, "05.1");
    }
}
