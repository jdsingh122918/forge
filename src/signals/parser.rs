//! Signal parsing from Claude's output.
//!
//! Extracts progress signals from text using regex patterns for:
//! - `<progress>X%</progress>` or `<progress>X</progress>`
//! - `<blocker>description</blocker>`
//! - `<pivot>description</pivot>`
//! - `<spawn-subphase>JSON</spawn-subphase>` for sub-phase spawning

use super::types::{BlockerSignal, IterationSignals, PivotSignal, ProgressSignal, SubPhaseSpawnSignal};
use regex::Regex;
use std::sync::LazyLock;

// Compile regexes once using LazyLock
static PROGRESS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<progress>\s*(\d{1,3})%?\s*</progress>").unwrap());

static BLOCKER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<blocker>(.*?)</blocker>").unwrap());

static PIVOT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<pivot>(.*?)</pivot>").unwrap());

// Regex for sub-phase spawn signal - captures multiline JSON content
static SPAWN_SUBPHASE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<spawn-subphase>\s*(.*?)\s*</spawn-subphase>").unwrap());

/// Parser for extracting signals from Claude's output.
pub struct SignalParser {
    /// Whether to log parsing details (verbose mode)
    verbose: bool,
}

impl SignalParser {
    /// Create a new signal parser.
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    /// Extract all signals from the given text.
    pub fn parse(&self, text: &str) -> IterationSignals {
        let mut signals = IterationSignals::new();

        // Extract progress signals
        for cap in PROGRESS_REGEX.captures_iter(text) {
            if let Some(value_match) = cap.get(1) {
                let raw_value = value_match.as_str();
                if let Ok(percentage) = raw_value.parse::<u8>() {
                    // Clamp to 100
                    let clamped = percentage.min(100);
                    signals.progress.push(ProgressSignal::new(clamped, raw_value));

                    if self.verbose {
                        eprintln!("  Signal: progress {}%", clamped);
                    }
                }
            }
        }

        // Extract blocker signals
        for cap in BLOCKER_REGEX.captures_iter(text) {
            if let Some(desc_match) = cap.get(1) {
                let description = desc_match.as_str().trim();
                if !description.is_empty() {
                    signals.blockers.push(BlockerSignal::new(description));

                    if self.verbose {
                        eprintln!("  Signal: blocker \"{}\"", description);
                    }
                }
            }
        }

        // Extract pivot signals
        for cap in PIVOT_REGEX.captures_iter(text) {
            if let Some(approach_match) = cap.get(1) {
                let new_approach = approach_match.as_str().trim();
                if !new_approach.is_empty() {
                    signals.pivots.push(PivotSignal::new(new_approach));

                    if self.verbose {
                        eprintln!("  Signal: pivot \"{}\"", new_approach);
                    }
                }
            }
        }

        // Extract sub-phase spawn signals
        for cap in SPAWN_SUBPHASE_REGEX.captures_iter(text) {
            if let Some(json_match) = cap.get(1) {
                let json_str = json_match.as_str().trim();
                if !json_str.is_empty() {
                    // Try to parse as JSON
                    match serde_json::from_str::<SubPhaseSpawnSignal>(json_str) {
                        Ok(mut spawn_signal) => {
                            spawn_signal.timestamp = chrono::Utc::now();
                            signals.sub_phase_spawns.push(spawn_signal);

                            if self.verbose {
                                eprintln!(
                                    "  Signal: spawn-subphase \"{}\" (budget: {})",
                                    signals.sub_phase_spawns.last().unwrap().name,
                                    signals.sub_phase_spawns.last().unwrap().budget
                                );
                            }
                        }
                        Err(e) => {
                            if self.verbose {
                                eprintln!("  Warning: Failed to parse spawn-subphase JSON: {}", e);
                            }
                        }
                    }
                }
            }
        }

        signals
    }
}

/// Convenience function to extract signals without creating a parser.
pub fn extract_signals(text: &str) -> IterationSignals {
    SignalParser::new(false).parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_progress_with_percent() {
        let signals = extract_signals("Working on it... <progress>50%</progress> done so far.");
        assert_eq!(signals.progress.len(), 1);
        assert_eq!(signals.progress[0].percentage, 50);
        assert_eq!(signals.progress[0].raw_value, "50");
    }

    #[test]
    fn test_parse_progress_without_percent() {
        let signals = extract_signals("<progress>75</progress>");
        assert_eq!(signals.progress.len(), 1);
        assert_eq!(signals.progress[0].percentage, 75);
    }

    #[test]
    fn test_parse_progress_with_whitespace() {
        let signals = extract_signals("<progress>  25%  </progress>");
        assert_eq!(signals.progress.len(), 1);
        assert_eq!(signals.progress[0].percentage, 25);
    }

    #[test]
    fn test_parse_progress_clamps_to_100() {
        let signals = extract_signals("<progress>150%</progress>");
        assert_eq!(signals.progress.len(), 1);
        assert_eq!(signals.progress[0].percentage, 100);
    }

    #[test]
    fn test_parse_multiple_progress() {
        let signals =
            extract_signals("<progress>25%</progress> then <progress>50%</progress> finally done");
        assert_eq!(signals.progress.len(), 2);
        assert_eq!(signals.progress[0].percentage, 25);
        assert_eq!(signals.progress[1].percentage, 50);
    }

    #[test]
    fn test_parse_blocker() {
        let signals = extract_signals("<blocker>Need API key from user</blocker>");
        assert_eq!(signals.blockers.len(), 1);
        assert_eq!(signals.blockers[0].description, "Need API key from user");
    }

    #[test]
    fn test_parse_blocker_with_whitespace() {
        let signals = extract_signals("<blocker>  Need clarification  </blocker>");
        assert_eq!(signals.blockers.len(), 1);
        assert_eq!(signals.blockers[0].description, "Need clarification");
    }

    #[test]
    fn test_parse_pivot() {
        let signals = extract_signals("<pivot>Using REST API instead of GraphQL</pivot>");
        assert_eq!(signals.pivots.len(), 1);
        assert_eq!(
            signals.pivots[0].new_approach,
            "Using REST API instead of GraphQL"
        );
    }

    #[test]
    fn test_parse_multiple_signals() {
        let text = r#"
            Starting work...
            <progress>25%</progress>
            Found an issue: <blocker>Missing configuration file</blocker>
            <pivot>Will create default config instead</pivot>
            <progress>50%</progress>
            Almost done...
            <progress>100%</progress>
        "#;

        let signals = extract_signals(text);

        assert_eq!(signals.progress.len(), 3);
        assert_eq!(signals.progress[0].percentage, 25);
        assert_eq!(signals.progress[1].percentage, 50);
        assert_eq!(signals.progress[2].percentage, 100);

        assert_eq!(signals.blockers.len(), 1);
        assert_eq!(
            signals.blockers[0].description,
            "Missing configuration file"
        );

        assert_eq!(signals.pivots.len(), 1);
        assert_eq!(
            signals.pivots[0].new_approach,
            "Will create default config instead"
        );
    }

    #[test]
    fn test_parse_no_signals() {
        let signals = extract_signals("Just regular text without any signals.");
        assert!(!signals.has_signals());
        assert!(signals.progress.is_empty());
        assert!(signals.blockers.is_empty());
        assert!(signals.pivots.is_empty());
    }

    #[test]
    fn test_parse_empty_tags_ignored() {
        let signals = extract_signals("<blocker></blocker> <pivot>  </pivot>");
        assert!(signals.blockers.is_empty());
        assert!(signals.pivots.is_empty());
    }

    #[test]
    fn test_parse_mixed_with_promise() {
        // Signals should work alongside promise tags
        let text = r#"
            <progress>100%</progress>
            All done!
            <promise>PHASE COMPLETE</promise>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.progress.len(), 1);
        assert!(signals.progress[0].is_complete());
    }

    #[test]
    fn test_signal_parser_verbose() {
        let parser = SignalParser::new(true);
        let signals = parser.parse("<progress>50%</progress>");
        assert_eq!(signals.progress.len(), 1);
    }

    #[test]
    fn test_parse_spawn_subphase() {
        let text = r#"
            <spawn-subphase>
            {
                "name": "OAuth setup",
                "promise": "OAUTH DONE",
                "budget": 5,
                "reasoning": "OAuth is complex"
            }
            </spawn-subphase>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.sub_phase_spawns.len(), 1);
        assert_eq!(signals.sub_phase_spawns[0].name, "OAuth setup");
        assert_eq!(signals.sub_phase_spawns[0].promise, "OAUTH DONE");
        assert_eq!(signals.sub_phase_spawns[0].budget, 5);
        assert_eq!(signals.sub_phase_spawns[0].reasoning, "OAuth is complex");
    }

    #[test]
    fn test_parse_spawn_subphase_minimal() {
        let text = r#"
            <spawn-subphase>{"name": "Task", "promise": "DONE", "budget": 3}</spawn-subphase>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.sub_phase_spawns.len(), 1);
        assert_eq!(signals.sub_phase_spawns[0].name, "Task");
        assert_eq!(signals.sub_phase_spawns[0].budget, 3);
        assert!(signals.sub_phase_spawns[0].reasoning.is_empty());
    }

    #[test]
    fn test_parse_multiple_spawn_subphases() {
        let text = r#"
            <spawn-subphase>{"name": "First", "promise": "FIRST DONE", "budget": 3}</spawn-subphase>
            <spawn-subphase>{"name": "Second", "promise": "SECOND DONE", "budget": 4}</spawn-subphase>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.sub_phase_spawns.len(), 2);
        assert_eq!(signals.sub_phase_spawns[0].name, "First");
        assert_eq!(signals.sub_phase_spawns[1].name, "Second");
    }

    #[test]
    fn test_parse_spawn_subphase_with_other_signals() {
        let text = r#"
            <progress>50%</progress>
            <spawn-subphase>{"name": "Task", "promise": "DONE", "budget": 3}</spawn-subphase>
            <blocker>Need OAuth credentials</blocker>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.progress.len(), 1);
        assert_eq!(signals.sub_phase_spawns.len(), 1);
        assert_eq!(signals.blockers.len(), 1);
        assert!(signals.has_sub_phase_spawns());
    }

    #[test]
    fn test_parse_spawn_subphase_invalid_json() {
        let text = r#"
            <spawn-subphase>{ invalid json }</spawn-subphase>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.sub_phase_spawns.len(), 0);
    }

    #[test]
    fn test_parse_spawn_subphase_with_skills() {
        let text = r#"
            <spawn-subphase>
            {
                "name": "API setup",
                "promise": "API DONE",
                "budget": 8,
                "reasoning": "API needs dedicated focus",
                "skills": ["api-design", "rust-conventions"]
            }
            </spawn-subphase>
        "#;

        let signals = extract_signals(text);
        assert_eq!(signals.sub_phase_spawns.len(), 1);
        assert_eq!(signals.sub_phase_spawns[0].skills.len(), 2);
        assert_eq!(signals.sub_phase_spawns[0].skills[0], "api-design");
    }
}
