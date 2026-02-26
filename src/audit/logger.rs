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

    pub fn add_phase(&mut self, phase: PhaseAudit) -> Result<()> {
        if let Some(ref mut run) = self.current_run {
            run.phases.push(phase);
            self.save_current()?;
        }
        Ok(())
    }

    pub fn update_last_phase<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut PhaseAudit),
    {
        if let Some(ref mut run) = self.current_run
            && let Some(phase) = run.phases.last_mut()
        {
            f(phase);
            self.save_current()?;
        }
        Ok(())
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

        // Remove current run file
        if self.current_run_file.exists() {
            fs::remove_file(&self.current_run_file).ok();
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
    use tempfile::tempdir;

    fn make_run_config() -> RunConfig {
        RunConfig {
            auto_approve_threshold: 5,
            skip_permissions: true,
            verbose: false,
            spec_file: std::path::PathBuf::from("spec.md"),
            project_dir: std::path::PathBuf::from("."),
        }
    }

    fn make_phase_audit() -> PhaseAudit {
        PhaseAudit::new("01", "Test phase", "<promise>DONE</promise>")
    }

    fn setup_logger() -> (AuditLogger, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("runs")).unwrap();
        let logger = AuditLogger::new(dir.path());
        (logger, dir)
    }

    #[test]
    fn test_start_run_creates_current_run_file() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        assert!(dir.path().join("current-run.json").exists());
    }

    #[test]
    fn test_add_phase_updates_current_run() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger.add_phase(make_phase_audit()).unwrap();
        let current_run = logger.current_run().unwrap();
        assert_eq!(current_run.phases.len(), 1);
    }

    #[test]
    fn test_finish_run_writes_to_runs_dir() {
        let (mut logger, dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        let run_file = logger.finish_run().unwrap();
        assert!(!dir.path().join("current-run.json").exists());
        assert!(run_file.exists());
        assert!(run_file.to_str().unwrap().contains("runs"));
    }

    #[test]
    fn test_finish_run_without_start_errors() {
        let (mut logger, _dir) = setup_logger();
        let result = logger.finish_run();
        assert!(result.is_err());
    }

    #[test]
    fn test_load_current_recovers_in_progress_run() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("runs")).unwrap();

        {
            let mut logger = AuditLogger::new(dir.path());
            logger.start_run(make_run_config()).unwrap();
            logger.add_phase(make_phase_audit()).unwrap();
            // Simulate crash — do NOT call finish_run
        }

        let mut logger2 = AuditLogger::new(dir.path());
        let loaded = logger2.load_current().unwrap();
        assert!(loaded);
        let run = logger2.current_run().unwrap();
        assert_eq!(run.phases.len(), 1);
    }

    #[test]
    fn test_load_current_returns_false_when_no_file() {
        let (mut logger, _dir) = setup_logger();
        let loaded = logger.load_current().unwrap();
        assert!(!loaded);
    }

    #[test]
    fn test_list_runs_empty_directory() {
        let (logger, _dir) = setup_logger();
        let runs = logger.list_runs().unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn test_list_runs_after_finish() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger.finish_run().unwrap();
        let runs = logger.list_runs().unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn test_run_file_is_valid_json() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "Alpha", "ALPHA_DONE"))
            .unwrap();
        let run_file = logger.finish_run().unwrap();
        let content = std::fs::read_to_string(&run_file).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .expect("finished run file should be valid JSON");
        assert!(parsed.get("run_id").is_some());
        assert!(parsed.get("phases").is_some());
    }

    #[test]
    fn test_update_last_phase_modifies_phase() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "First", "DONE"))
            .unwrap();
        logger
            .update_last_phase(|p| {
                p.description = "Updated description".to_string();
            })
            .unwrap();
        let run = logger.current_run().unwrap();
        assert_eq!(run.phases[0].description, "Updated description");
    }

    #[test]
    fn test_multiple_phases_persisted() {
        let (mut logger, _dir) = setup_logger();
        logger.start_run(make_run_config()).unwrap();
        logger
            .add_phase(PhaseAudit::new("01", "First", "DONE"))
            .unwrap();
        logger
            .add_phase(PhaseAudit::new("02", "Second", "DONE2"))
            .unwrap();
        logger
            .add_phase(PhaseAudit::new("03", "Third", "DONE3"))
            .unwrap();
        let run = logger.current_run().unwrap();
        assert_eq!(run.phases.len(), 3);
        assert_eq!(run.phases[1].phase_number, "02");
    }
}
