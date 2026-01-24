use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEntry {
    pub phase: String,
    pub iteration: u32,
    pub status: String,
    pub timestamp: DateTime<Utc>,
}

pub struct StateManager {
    state_file: std::path::PathBuf,
}

impl StateManager {
    pub fn new(state_file: std::path::PathBuf) -> Self {
        Self { state_file }
    }

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

    pub fn get_last_completed_phase(&self) -> Option<String> {
        if !self.state_file.exists() {
            return None;
        }

        let content = fs::read_to_string(&self.state_file).ok()?;

        content
            .lines()
            .filter(|line| line.contains("|completed|"))
            .last()
            .and_then(|line| line.split('|').next())
            .map(|s| s.to_string())
    }

    pub fn get_entries(&self) -> Result<Vec<StateEntry>> {
        if !self.state_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.state_file).context("Failed to read state file")?;

        let entries: Vec<StateEntry> = content
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 4 {
                    Some(StateEntry {
                        phase: parts[0].to_string(),
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

    pub fn reset(&self) -> Result<()> {
        if self.state_file.exists() {
            fs::remove_file(&self.state_file).context("Failed to remove state file")?;
        }
        Ok(())
    }
}
