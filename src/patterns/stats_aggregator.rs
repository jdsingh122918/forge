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
