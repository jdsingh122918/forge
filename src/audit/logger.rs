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
        if let Some(ref mut run) = self.current_run {
            if let Some(phase) = run.phases.last_mut() {
                f(phase);
                self.save_current()?;
            }
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
