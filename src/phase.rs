//! Phase definition and JSON loading for the forge orchestrator.
//!
//! This module provides:
//! - `Phase` struct representing a single implementation phase
//! - `PhasesFile` struct representing the full phases.json format
//! - Loading functions for JSON-based phase configuration
//! - Default IdCheck phases as a fallback

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::forge_config::PermissionMode;

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
    /// - Strict: Require approval for every iteration
    /// - Standard: Approve phase start, auto-continue iterations
    /// - Autonomous: Auto-approve if within budget and making progress
    /// - Readonly: Planning/research phases, no file modifications
    #[serde(default)]
    pub permission_mode: PermissionMode,
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

    /// Get phases starting from a given phase number.
    pub fn get_phases_from(&self, start: &str) -> Vec<&Phase> {
        self.phases
            .iter()
            .filter(|p| p.number.as_str() >= start)
            .collect()
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
            PermissionMode::Strict,
        );

        assert_eq!(phase.permission_mode, PermissionMode::Strict);
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
        assert_eq!(phase.permission_mode, PermissionMode::Strict);

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
}
