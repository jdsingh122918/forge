use anyhow::Result;
use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SubPhaseSpawnSignal {
    pub name: String,
    pub budget: u32,
    #[serde(skip)]
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Default)]
pub struct IterationSignals {
    pub progress: Option<u8>,
    pub blockers: Vec<String>,
    pub pivots: Vec<String>,
    pub sub_phase_spawns: Vec<SubPhaseSpawnSignal>,
    pub done: bool,
}

pub struct SignalParser {
    verbose: bool,
    spawn_regex: Regex,
}

impl SignalParser {
    pub fn parse(&self, text: &str) -> IterationSignals {
        let mut signals = IterationSignals::default();

        for cap in self.spawn_regex.captures_iter(text) {
            if let Some(json_match) = cap.get(1) {
                let json_str = json_match.as_str().trim();
                if !json_str.is_empty() {
                    match serde_json::from_str::<SubPhaseSpawnSignal>(json_str) {
                        Ok(mut spawn_signal) => {
                            spawn_signal.timestamp = chrono::Utc::now();
                            // BUG: push first, then unwrap last() to log.
                            // last() cannot return None here, but unwrap() in
                            // production code is fragile and will panic if the
                            // vec is ever refactored to drain or filter.
                            signals.sub_phase_spawns.push(spawn_signal);

                            if self.verbose {
                                log::debug!(
                                    "Signal: spawn-subphase \"{}\" (budget: {})",
                                    signals.sub_phase_spawns.last().unwrap().name,
                                    signals.sub_phase_spawns.last().unwrap().budget
                                );
                            }
                        }
                        Err(e) => {
                            if self.verbose {
                                log::warn!("Failed to parse spawn-subphase JSON: {}", e);
                            }
                        }
                    }
                }
            }
        }

        signals
    }
}

pub fn extract_signals(text: &str) -> Result<IterationSignals> {
    let parser = SignalParser {
        verbose: false,
        spawn_regex: Regex::new(r"<spawn-subphase>(.*?)</spawn-subphase>")?,
    };
    Ok(parser.parse(text))
}
