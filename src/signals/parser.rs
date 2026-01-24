//! Signal parsing from Claude's output.
//!
//! Extracts progress signals from text using regex patterns for:
//! - `<progress>X%</progress>` or `<progress>X</progress>`
//! - `<blocker>description</blocker>`
//! - `<pivot>description</pivot>`

use super::types::{BlockerSignal, IterationSignals, PivotSignal, ProgressSignal};
use regex::Regex;
use std::sync::LazyLock;

// Compile regexes once using LazyLock
static PROGRESS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<progress>\s*(\d{1,3})%?\s*</progress>").unwrap());

static BLOCKER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<blocker>(.*?)</blocker>").unwrap());

static PIVOT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<pivot>(.*?)</pivot>").unwrap());

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
}
