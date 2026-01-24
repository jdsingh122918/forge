use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEntry {
    pub phase: String,
    /// Sub-phase number if this is a sub-phase entry (e.g., "05.1")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_phase: Option<String>,
    pub iteration: u32,
    pub status: String,
    pub timestamp: DateTime<Utc>,
}

impl StateEntry {
    /// Check if this is a sub-phase entry.
    pub fn is_sub_phase(&self) -> bool {
        self.sub_phase.is_some()
    }

    /// Get the full phase/sub-phase identifier.
    pub fn full_phase_id(&self) -> String {
        self.sub_phase.clone().unwrap_or_else(|| self.phase.clone())
    }

    /// Get the parent phase number for sub-phase entries.
    pub fn parent_phase(&self) -> &str {
        &self.phase
    }
}

pub struct StateManager {
    state_file: std::path::PathBuf,
}

impl StateManager {
    pub fn new(state_file: std::path::PathBuf) -> Self {
        Self { state_file }
    }

    /// Save a phase state entry (legacy format, backward compatible).
    pub fn save(&self, phase: &str, iteration: u32, status: &str) -> Result<()> {
        let entry = format!(
            "{}|{}|{}|{}\n",
            phase,
            iteration,
            status,
            Utc::now().to_rfc3339()
        );

        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.state_file)
            .context("Failed to open state file")?
            .write_all(entry.as_bytes())
            .context("Failed to write state entry")?;

        Ok(())
    }

    /// Save a sub-phase state entry with parent phase reference.
    /// Format: phase|sub_phase|iteration|status|timestamp
    pub fn save_sub_phase(
        &self,
        parent_phase: &str,
        sub_phase: &str,
        iteration: u32,
        status: &str,
    ) -> Result<()> {
        let entry = format!(
            "{}|{}|{}|{}|{}\n",
            parent_phase,
            sub_phase,
            iteration,
            status,
            Utc::now().to_rfc3339()
        );

        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.state_file)
            .context("Failed to open state file")?
            .write_all(entry.as_bytes())
            .context("Failed to write sub-phase state entry")?;

        Ok(())
    }

    /// Get the last completed top-level phase (not sub-phases).
    pub fn get_last_completed_phase(&self) -> Option<String> {
        if !self.state_file.exists() {
            return None;
        }

        let content = fs::read_to_string(&self.state_file).ok()?;

        content
            .lines()
            .filter(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                // Old format has 4 parts, new sub-phase format has 5 parts
                // Only consider old format (top-level phases) for this query
                parts.len() == 4 && line.contains("|completed|")
            })
            .next_back()
            .and_then(|line| line.split('|').next())
            .map(|s| s.to_string())
    }

    /// Get the last completed phase or sub-phase.
    pub fn get_last_completed_any(&self) -> Option<String> {
        if !self.state_file.exists() {
            return None;
        }

        let content = fs::read_to_string(&self.state_file).ok()?;

        content
            .lines()
            .rfind(|line| line.contains("|completed|"))
            .and_then(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() == 5 {
                    // Sub-phase format: phase|sub_phase|iteration|status|timestamp
                    Some(parts[1].to_string()) // Return sub-phase number
                } else if parts.len() >= 4 {
                    // Old format: phase|iteration|status|timestamp
                    Some(parts[0].to_string())
                } else {
                    None
                }
            })
    }

    /// Get all state entries including sub-phase entries.
    pub fn get_entries(&self) -> Result<Vec<StateEntry>> {
        if !self.state_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.state_file).context("Failed to read state file")?;

        let entries: Vec<StateEntry> = content
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() == 5 {
                    // New sub-phase format: phase|sub_phase|iteration|status|timestamp
                    Some(StateEntry {
                        phase: parts[0].to_string(),
                        sub_phase: Some(parts[1].to_string()),
                        iteration: parts[2].parse().unwrap_or(0),
                        status: parts[3].to_string(),
                        timestamp: DateTime::parse_from_rfc3339(parts[4])
                            .ok()?
                            .with_timezone(&Utc),
                    })
                } else if parts.len() >= 4 {
                    // Old format: phase|iteration|status|timestamp
                    Some(StateEntry {
                        phase: parts[0].to_string(),
                        sub_phase: None,
                        iteration: parts[1].parse().unwrap_or(0),
                        status: parts[2].to_string(),
                        timestamp: DateTime::parse_from_rfc3339(parts[3])
                            .ok()?
                            .with_timezone(&Utc),
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(entries)
    }

    /// Get entries for a specific phase including its sub-phases.
    pub fn get_phase_entries(&self, phase: &str) -> Result<Vec<StateEntry>> {
        let entries = self.get_entries()?;
        Ok(entries.into_iter().filter(|e| e.phase == phase).collect())
    }

    /// Get entries for a specific sub-phase.
    pub fn get_sub_phase_entries(&self, sub_phase: &str) -> Result<Vec<StateEntry>> {
        let entries = self.get_entries()?;
        Ok(entries
            .into_iter()
            .filter(|e| e.sub_phase.as_deref() == Some(sub_phase))
            .collect())
    }

    /// Check if a phase has any sub-phase entries.
    pub fn has_sub_phase_entries(&self, phase: &str) -> Result<bool> {
        let entries = self.get_phase_entries(phase)?;
        Ok(entries.iter().any(|e| e.is_sub_phase()))
    }

    /// Get all completed sub-phases for a parent phase.
    pub fn get_completed_sub_phases(&self, parent_phase: &str) -> Result<Vec<String>> {
        let entries = self.get_phase_entries(parent_phase)?;
        Ok(entries
            .into_iter()
            .filter(|e| e.is_sub_phase() && e.status == "completed")
            .filter_map(|e| e.sub_phase)
            .collect())
    }

    /// Check if all sub-phases of a parent are complete.
    pub fn all_sub_phases_complete(
        &self,
        parent_phase: &str,
        expected_count: usize,
    ) -> Result<bool> {
        let completed = self.get_completed_sub_phases(parent_phase)?;
        Ok(completed.len() >= expected_count)
    }

    pub fn reset(&self) -> Result<()> {
        if self.state_file.exists() {
            fs::remove_file(&self.state_file).context("Failed to remove state file")?;
        }
        Ok(())
    }
}
