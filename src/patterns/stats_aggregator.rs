//! Cross-pattern statistics aggregation.
//!
//! [`display_type_statistics`] renders a summary table showing average
//! iterations, success rates, and budget utilisation for each PhaseType.

use std::collections::HashMap;

use super::learning::{Pattern, PhaseStat, PhaseType, PhaseTypeStats};

/// Display statistics by phase type.
pub fn display_type_statistics(patterns: &[Pattern]) {
    if patterns.is_empty() {
        println!("No patterns to analyze.");
        return;
    }

    // Aggregate across all patterns
    let mut type_data: HashMap<PhaseType, Vec<&PhaseStat>> = HashMap::new();
    for pattern in patterns {
        for stat in &pattern.phase_stats {
            type_data.entry(stat.phase_type).or_default().push(stat);
        }
    }

    println!();
    println!(
        "Phase Type Statistics (across {} patterns):",
        patterns.len()
    );
    println!(
        "{:<12} {:<8} {:<10} {:<10} {:<10} {:<10}",
        "Type", "Count", "Avg Iter", "Min", "Max", "Success"
    );
    println!(
        "{:<12} {:<8} {:<10} {:<10} {:<10} {:<10}",
        "------------", "--------", "----------", "----------", "----------", "----------"
    );

    let types = [
        PhaseType::Scaffold,
        PhaseType::Implement,
        PhaseType::Test,
        PhaseType::Refactor,
        PhaseType::Fix,
    ];
    for phase_type in types {
        if let Some(phases) = type_data.get(&phase_type) {
            let stats = PhaseTypeStats::from_phases(phases);
            println!(
                "{:<12} {:<8} {:<10.1} {:<10} {:<10} {:<10.0}%",
                phase_type.as_str(),
                stats.count,
                stats.avg_iterations,
                stats.min_iterations,
                stats.max_iterations,
                stats.success_rate * 100.0
            );
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::learning::{Pattern, PhaseStat, PhaseType, PhaseTypeStats};

    // ----------------------------------------------------------------
    // Helpers
    // ----------------------------------------------------------------

    fn make_stat(name: &str, actual: u32, budget: u32, phase_type: PhaseType) -> PhaseStat {
        PhaseStat {
            name: name.to_string(),
            promise: "DONE".to_string(),
            actual_iterations: actual,
            original_budget: budget,
            phase_type,
            file_patterns: vec![],
            common_errors: vec![],
        }
    }

    fn make_pattern(name: &str, stats: Vec<PhaseStat>) -> Pattern {
        let mut p = Pattern::new(name);
        p.total_phases = stats.len();
        p.phase_stats = stats;
        p
    }

    // ----------------------------------------------------------------
    // display_type_statistics — smoke tests (does not panic)
    // ----------------------------------------------------------------

    #[test]
    fn display_type_statistics_does_not_panic_with_empty_slice() {
        // Should print "No patterns to analyze." and return without panicking.
        display_type_statistics(&[]);
    }

    #[test]
    fn display_type_statistics_does_not_panic_with_single_pattern() {
        let p = make_pattern(
            "single",
            vec![
                make_stat("Scaffold", 5, 10, PhaseType::Scaffold),
                make_stat("API impl", 8, 10, PhaseType::Implement),
            ],
        );
        display_type_statistics(&[p]);
    }

    #[test]
    fn display_type_statistics_does_not_panic_with_multiple_patterns() {
        let p1 = make_pattern(
            "proj-a",
            vec![
                make_stat("Setup", 3, 10, PhaseType::Scaffold),
                make_stat("Tests", 6, 10, PhaseType::Test),
            ],
        );
        let p2 = make_pattern(
            "proj-b",
            vec![
                make_stat("Bugfix", 4, 8, PhaseType::Fix),
                make_stat("Refactor", 9, 10, PhaseType::Refactor),
            ],
        );
        display_type_statistics(&[p1, p2]);
    }

    // ----------------------------------------------------------------
    // PhaseTypeStats::from_phases — the aggregation kernel
    // These tests verify the computation logic that display_type_statistics relies on.
    // ----------------------------------------------------------------

    #[test]
    fn aggregation_empty_input_produces_zero_stats() {
        let stats = PhaseTypeStats::from_phases(&[]);
        assert_eq!(stats.count, 0);
        assert_eq!(stats.avg_iterations, 0.0);
        assert_eq!(stats.min_iterations, 0);
        assert_eq!(stats.max_iterations, 0);
        assert_eq!(stats.avg_budget, 0.0);
        assert_eq!(stats.success_rate, 0.0);
    }

    #[test]
    fn aggregation_single_entry_all_fields_correct() {
        let s = make_stat("Phase A", 7, 10, PhaseType::Implement);
        let stats = PhaseTypeStats::from_phases(&[&s]);

        assert_eq!(stats.count, 1);
        assert_eq!(stats.avg_iterations, 7.0);
        assert_eq!(stats.min_iterations, 7);
        assert_eq!(stats.max_iterations, 7);
        assert_eq!(stats.avg_budget, 10.0);
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn aggregation_multiple_entries_averages_correctly() {
        // actual: 2, 4, 6; budget: 5, 10, 15; within budget: all three
        let s1 = make_stat("A", 2, 5, PhaseType::Scaffold);
        let s2 = make_stat("B", 4, 10, PhaseType::Scaffold);
        let s3 = make_stat("C", 6, 15, PhaseType::Scaffold);
        let stats = PhaseTypeStats::from_phases(&[&s1, &s2, &s3]);

        assert_eq!(stats.count, 3);
        // avg_iterations = (2+4+6)/3 = 4.0
        assert!((stats.avg_iterations - 4.0).abs() < 1e-9);
        assert_eq!(stats.min_iterations, 2);
        assert_eq!(stats.max_iterations, 6);
        // avg_budget = (5+10+15)/3 = 10.0
        assert!((stats.avg_budget - 10.0).abs() < 1e-9);
        // All within budget → success_rate = 1.0
        assert_eq!(stats.success_rate, 1.0);
    }

    #[test]
    fn aggregation_tracks_success_rate_accurately() {
        // 2 within budget, 1 exceeded → success_rate = 2/3
        let within1 = make_stat("W1", 5, 10, PhaseType::Fix);
        let within2 = make_stat("W2", 9, 10, PhaseType::Fix);
        let exceeded = make_stat("E1", 15, 10, PhaseType::Fix);
        let stats = PhaseTypeStats::from_phases(&[&within1, &within2, &exceeded]);

        assert!((stats.success_rate - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn aggregation_min_and_max_correct_with_outlier() {
        // One outlier at 100; normal phases at 5 and 6.
        let normal1 = make_stat("N1", 5, 20, PhaseType::Refactor);
        let normal2 = make_stat("N2", 6, 20, PhaseType::Refactor);
        let outlier = make_stat("O1", 100, 20, PhaseType::Refactor);
        let stats = PhaseTypeStats::from_phases(&[&normal1, &normal2, &outlier]);

        assert_eq!(stats.min_iterations, 5);
        assert_eq!(stats.max_iterations, 100);
        // avg = (5+6+100)/3 ≈ 37.0
        assert!((stats.avg_iterations - 111.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn aggregation_cross_pattern_collection_matches_manual_calculation() {
        // Simulate what display_type_statistics does internally:
        // collect PhaseStat refs across two patterns and feed to from_phases.
        let p1 = make_pattern(
            "proj-x",
            vec![
                make_stat("Implement A", 4, 10, PhaseType::Implement),
                make_stat("Implement B", 8, 10, PhaseType::Implement),
            ],
        );
        let p2 = make_pattern(
            "proj-y",
            vec![make_stat("Implement C", 6, 10, PhaseType::Implement)],
        );

        // Replicate the aggregation logic from display_type_statistics
        let mut collected: Vec<&PhaseStat> = vec![];
        for p in &[&p1, &p2] {
            for stat in &p.phase_stats {
                if stat.phase_type == PhaseType::Implement {
                    collected.push(stat);
                }
            }
        }

        let stats = PhaseTypeStats::from_phases(&collected);
        assert_eq!(stats.count, 3);
        // avg = (4+8+6)/3 = 6.0
        assert!((stats.avg_iterations - 6.0).abs() < 1e-9);
        // All within budget → success_rate = 1.0
        assert_eq!(stats.success_rate, 1.0);
    }
}
