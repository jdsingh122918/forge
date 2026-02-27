use super::{AuditRun, PhaseAudit, RunConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct AuditLogger {
    audit_dir: PathBuf,
    current_run: Option<AuditRun>,
    current_run_file: PathBuf,
}

impl AuditLogger {
    pub fn new(audit_dir: &Path) -> Self {
        let current_run_file = audit_dir.join("current-run.json");
        Self {
            audit_dir: audit_dir.to_path_buf(),
            current_run: None,
            current_run_file,
        }
    }

    pub fn start_run(&mut self, config: RunConfig) -> Result<()> {
        let run = AuditRun::new(config);
        self.current_run = Some(run);
        self.save_current()?;
        Ok(())
    }

    /// Add a phase audit record to the current run.
    ///
    /// Returns an error if no run is currently active (i.e., `start_run` was never called
    /// or `finish_run` has already been called). This prevents silent data loss when callers
    /// forget to start a run before logging phase data.
    pub fn add_phase(&mut self, phase: PhaseAudit) -> Result<()> {
        let run = self
            .current_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("add_phase called with no active run"))?;
        run.phases.push(phase);
        self.save_current()
    }

    /// Apply a mutation to the last phase in the current run.
    ///
    /// Returns an error if no run is currently active, or if the current run has no phases
    /// yet. Both conditions indicate a programming error — the caller must ensure a run and
    /// at least one phase exist before updating.
    pub fn update_last_phase<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut PhaseAudit),
    {
        let run = self
            .current_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("update_last_phase called with no active run"))?;
        let phase = run
            .phases
            .last_mut()
            .ok_or_else(|| anyhow::anyhow!("update_last_phase called with no phases in run"))?;
        f(phase);
        self.save_current()
    }

    pub fn finish_run(&mut self) -> Result<PathBuf> {
        let run = self
            .current_run
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No current run to finish"))?;

        run.finish();

        // Save to runs directory
        let filename = format!(
            "{}_{}.json",
            run.started_at.format("%Y-%m-%dT%H-%M-%S"),
            &run.run_id.to_string()[..8]
        );
        let run_file = self.audit_dir.join("runs").join(&filename);

        let json = serde_json::to_string_pretty(&run).context("Failed to serialize audit run")?;
        fs::write(&run_file, json).context("Failed to write audit run file")?;

        // Remove current run file — propagate errors instead of silently discarding them
        if self.current_run_file.exists() {
            fs::remove_file(&self.current_run_file)
                .context("Failed to remove current-run.json after finishing run")?;
        }

        self.current_run = None;
        Ok(run_file)
    }

    pub fn save_current(&self) -> Result<()> {
        if let Some(ref run) = self.current_run {
            let json =
                serde_json::to_string_pretty(&run).context("Failed to serialize current run")?;
            fs::write(&self.current_run_file, json).context("Failed to write current run file")?;
        }
        Ok(())
    }

    pub fn load_current(&mut self) -> Result<bool> {
        if self.current_run_file.exists() {
            let content = fs::read_to_string(&self.current_run_file)
                .context("Failed to read current run file")?;
            let run: AuditRun =
                serde_json::from_str(&content).context("Failed to parse current run file")?;
            self.current_run = Some(run);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn current_run(&self) -> Option<&AuditRun> {
        self.current_run.as_ref()
    }

    pub fn list_runs(&self) -> Result<Vec<PathBuf>> {
        let runs_dir = self.audit_dir.join("runs");
        if !runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut runs: Vec<PathBuf> = fs::read_dir(&runs_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .collect();

        runs.sort();
        runs.reverse(); // Most recent first
        Ok(runs)
    }

    pub fn load_run(&self, path: &Path) -> Result<AuditRun> {
        let content = fs::read_to_string(path).context("Failed to read audit run file")?;
        let run: AuditRun =
            serde_json::from_str(&content).context("Failed to parse audit run file")?;
        Ok(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::PhaseOutcome;
    use tempfile::TempDir;

    /// Create a temporary audit directory with the expected `runs/` subdirectory, and
    /// return both an `AuditLogger` pointed at it and the `TempDir` guard (which must be
    /// kept alive for the duration of the test so the directory is not deleted early).
    fn setup_logger() -> (AuditLogger, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let runs_dir = dir.path().join("runs");
        std::fs::create_dir_all(&runs_dir).expect("failed to create runs dir");
        let logger = AuditLogger::new(dir.path());
        (logger, dir)
    }

    /// Build a minimal `RunConfig` suitable for unit tests.
    fn make_run_config() -> RunConfig {
        RunConfig {
            auto_approve_threshold: 3,
            skip_permissions: true,
            verbose: false,
            spec_file: PathBuf::from("spec.md"),
            project_dir: PathBuf::from("."),
        }
    }

    // -------------------------------------------------------------------------
    // Issue 1 — CRITICAL: silent Ok(()) when no run is active
    // -------------------------------------------------------------------------

    #[test]
    fn test_add_phase_without_active_run_returns_err() {
        let (mut logger, _dir) = setup_logger();
        let result = logger.add_phase(PhaseAudit::new("01", "Orphan", "DONE"));
        assert!(
            result.is_err(),
            "add_phase with no active run must return Err"
        );
    }

    #[test]
    fn test_update_last_phase_with_no_phases_returns_err() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        let result = logger.update_last_phase(|p| {
            p.description = "x".to_string();
        });
        assert!(
            result.is_err(),
            "update_last_phase with empty phases must return Err"
        );
    }

    #[test]
    fn test_update_last_phase_without_active_run_returns_err() {
        let (mut logger, _dir) = setup_logger();
        // No start_run call — current_run is None.
        let result = logger.update_last_phase(|p| {
            p.description = "should fail".to_string();
        });
        assert!(
            result.is_err(),
            "update_last_phase with no active run must return Err"
        );
    }

    // -------------------------------------------------------------------------
    // Issue 3 — HIGH: test_run_file_is_valid_json assertions too weak
    // -------------------------------------------------------------------------

    #[test]
    fn test_run_file_is_valid_json() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "Bootstrap", "DONE"))
            .unwrap();
        let run_path = logger.finish_run().unwrap();

        let content = std::fs::read_to_string(&run_path).expect("run file must exist");
        let value: serde_json::Value =
            serde_json::from_str(&content).expect("run file must be valid JSON");

        // run_id must be a 36-character UUID string (8-4-4-4-12 = 36 chars with hyphens)
        let run_id = value
            .get("run_id")
            .expect("run_id field must be present")
            .as_str()
            .expect("run_id must be a string");
        assert_eq!(
            run_id.len(),
            36,
            "run_id must be a UUID string of length 36, got: {run_id}"
        );

        // phases must be a JSON array with at least one element
        let phases = value
            .get("phases")
            .expect("phases field must be present")
            .as_array()
            .expect("phases must be a JSON array");
        assert!(
            !phases.is_empty(),
            "phases array must contain at least 1 element"
        );

        // ended_at must be present and not null (set by finish_run via run.finish())
        let ended_at = value
            .get("ended_at")
            .expect("ended_at field must be present");
        assert!(
            !ended_at.is_null(),
            "ended_at must not be null after finish_run"
        );
    }

    // -------------------------------------------------------------------------
    // Issue 4 — HIGH: test_multiple_phases_persisted only checks in-memory state
    // -------------------------------------------------------------------------

    #[test]
    fn test_multiple_phases_persisted() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "Phase One", "DONE"))
            .unwrap();
        logger
            .add_phase(PhaseAudit::new("02", "Phase Two", "DONE"))
            .unwrap();
        logger
            .add_phase(PhaseAudit::new("03", "Phase Three", "DONE"))
            .unwrap();

        // In-memory check
        let in_memory_count = logger
            .current_run()
            .expect("run must be active")
            .phases
            .len();
        assert_eq!(in_memory_count, 3, "in-memory phase count must be 3");

        // Disk persistence check: a second logger at the same path must load all 3 phases.
        // We keep `dir` alive so the temp directory is not deleted before we read from it.
        let mut second_logger = AuditLogger::new(dir.path());
        let loaded = second_logger
            .load_current()
            .expect("load_current must succeed");
        assert!(
            loaded,
            "load_current must return true when a run file exists"
        );
        let disk_count = second_logger
            .current_run()
            .expect("loaded run must be present")
            .phases
            .len();
        assert_eq!(
            disk_count, 3,
            "disk-persisted phase count must also be 3 — save_current() may be broken"
        );
    }

    // -------------------------------------------------------------------------
    // Issue 5 — MEDIUM: test_update_last_phase_modifies_phase only verifies in-memory
    // -------------------------------------------------------------------------

    #[test]
    fn test_update_last_phase_modifies_phase() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "Initial Description", "DONE"))
            .unwrap();

        // Mutate the last phase
        logger
            .update_last_phase(|p| {
                p.description = "Updated Description".to_string();
                p.outcome = PhaseOutcome::Completed { iteration: 2 };
            })
            .unwrap();

        // In-memory check
        let in_memory_desc = logger
            .current_run()
            .expect("run must be active")
            .phases
            .last()
            .expect("must have at least one phase")
            .description
            .clone();
        assert_eq!(in_memory_desc, "Updated Description");

        // Disk persistence check: reload from disk and verify the mutation was persisted.
        let mut second_logger = AuditLogger::new(dir.path());
        second_logger
            .load_current()
            .expect("load_current must succeed");
        let disk_phase = second_logger
            .current_run()
            .expect("loaded run must be present")
            .phases
            .last()
            .expect("loaded run must have phases");

        assert_eq!(
            disk_phase.description, "Updated Description",
            "update_last_phase mutation must be persisted to disk"
        );
        assert_eq!(
            disk_phase.outcome,
            PhaseOutcome::Completed { iteration: 2 },
            "outcome mutation must also be persisted to disk"
        );
    }

    // -------------------------------------------------------------------------
    // Additional happy-path coverage
    // -------------------------------------------------------------------------

    #[test]
    fn test_start_run_creates_current_run_file() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        assert!(
            dir.path().join("current-run.json").exists(),
            "current-run.json must exist after start_run"
        );
    }

    #[test]
    fn test_finish_run_removes_current_run_file() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger.finish_run().unwrap();
        assert!(
            !dir.path().join("current-run.json").exists(),
            "current-run.json must be removed after finish_run"
        );
    }
}
