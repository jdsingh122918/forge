//! Pattern learning and display commands — `forge learn` and `forge patterns`.

use anyhow::{Context, Result};

use super::super::PatternsCommands;

pub fn cmd_learn(project_dir: &std::path::Path, name: Option<&str>) -> Result<()> {
    use dialoguer::Input;
    use forge::init::is_initialized;
    use forge::patterns::{display_pattern, learn_pattern, save_pattern};

    // Check if project is initialized
    if !is_initialized(project_dir) {
        anyhow::bail!(
            "Project not initialized. Run 'forge init' first to create the .forge/ directory."
        );
    }

    // Determine pattern name
    let pattern_name = if let Some(n) = name {
        n.to_string()
    } else {
        let default_name = project_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("pattern")
            .to_string();

        Input::new()
            .with_prompt("Pattern name")
            .default(default_name)
            .interact_text()
            .context("Failed to read pattern name")?
    };

    println!("Learning pattern from current project...");

    // Learn the pattern
    let pattern = learn_pattern(project_dir, Some(&pattern_name))?;

    // Display the pattern
    println!();
    display_pattern(&pattern);
    println!();

    // Save the pattern
    let path = save_pattern(&pattern)?;
    println!("Pattern saved to: {}", path.display());

    Ok(())
}

pub fn cmd_patterns(command: Option<PatternsCommands>) -> Result<()> {
    use forge::patterns::{
        display_budget_suggestions, display_pattern, display_pattern_matches,
        display_patterns_list, display_type_statistics, get_pattern, list_patterns, match_patterns,
        suggest_budgets,
    };

    match command {
        None => {
            // List all patterns
            let patterns = list_patterns()?;
            display_patterns_list(&patterns);
        }
        Some(PatternsCommands::Show { name }) => {
            // Show specific pattern
            match get_pattern(&name)? {
                Some(pattern) => {
                    println!();
                    display_pattern(&pattern);

                    // Also show type statistics for this pattern
                    if !pattern.type_stats.is_empty() {
                        println!();
                        println!("Phase Type Breakdown:");
                        for (phase_type, stats) in &pattern.type_stats {
                            println!(
                                "  {}: {} phases, avg {:.1} iterations, {:.0}% success",
                                phase_type,
                                stats.count,
                                stats.avg_iterations,
                                stats.success_rate * 100.0
                            );
                        }
                    }
                    println!();
                }
                None => {
                    println!("Pattern '{}' not found.", name);
                    println!();
                    println!("Run 'forge patterns' to see available patterns.");
                }
            }
        }
        Some(PatternsCommands::Stats) => {
            // Show aggregate statistics across all patterns
            let patterns = list_patterns()?;
            if patterns.is_empty() {
                println!("No patterns found. Run 'forge learn' to create patterns.");
                return Ok(());
            }
            display_type_statistics(&patterns);
        }
        Some(PatternsCommands::Recommend { spec }) => {
            // Recommend patterns for a spec
            let spec_path = spec.unwrap_or_else(|| {
                let cwd = std::env::current_dir().unwrap_or_default();
                cwd.join(".forge").join("spec.md")
            });

            if !spec_path.exists() {
                println!("Spec file not found at: {}", spec_path.display());
                println!();
                println!("Provide a spec file with --spec or run 'forge interview' to create one.");
                return Ok(());
            }

            let spec_content =
                std::fs::read_to_string(&spec_path).context("Failed to read spec file")?;

            let patterns = list_patterns()?;
            if patterns.is_empty() {
                println!("No patterns found. Run 'forge learn' to create patterns first.");
                return Ok(());
            }

            let matches = match_patterns(&spec_content, &patterns);
            if matches.is_empty() {
                println!("No similar patterns found for this spec.");
                return Ok(());
            }

            display_pattern_matches(&matches);

            // Show budget suggestions based on top matches
            let top_patterns: Vec<_> = matches
                .iter()
                .filter(|m| m.score > 0.3)
                .take(3)
                .map(|m| m.pattern)
                .collect();

            if !top_patterns.is_empty() {
                println!("Based on similar patterns, here are budget recommendations:");
                println!();

                // Create hypothetical phases based on common phase types
                let demo_phases = vec![
                    forge::phase::Phase::new(
                        "01",
                        "Project scaffold",
                        "SCAFFOLD COMPLETE",
                        8,
                        "Setup",
                        vec![],
                    ),
                    forge::phase::Phase::new(
                        "02",
                        "Core implementation",
                        "CORE COMPLETE",
                        15,
                        "Build",
                        vec![],
                    ),
                    forge::phase::Phase::new("03", "Testing", "TESTS COMPLETE", 10, "Test", vec![]),
                ];

                let suggestions = suggest_budgets(&top_patterns, &demo_phases);
                display_budget_suggestions(&suggestions);
            }
        }
        Some(PatternsCommands::Compare { pattern1, pattern2 }) => {
            // Compare two patterns
            let p1 = get_pattern(&pattern1)?;
            let p2 = get_pattern(&pattern2)?;

            match (p1, p2) {
                (Some(p1), Some(p2)) => {
                    println!();
                    println!("Pattern Comparison: {} vs {}", p1.name, p2.name);
                    println!("{}", "=".repeat(50));
                    println!();

                    println!("{:<25} {:<15} {:<15}", "Metric", &p1.name, &p2.name);
                    println!(
                        "{:<25} {:<15} {:<15}",
                        "-".repeat(25),
                        "-".repeat(15),
                        "-".repeat(15)
                    );

                    println!(
                        "{:<25} {:<15} {:<15}",
                        "Total Phases", p1.total_phases, p2.total_phases
                    );
                    println!(
                        "{:<25} {:<15.0}% {:<15.0}%",
                        "Success Rate",
                        p1.success_rate * 100.0,
                        p2.success_rate * 100.0
                    );
                    println!(
                        "{:<25} {:<15} {:<15}",
                        "Tags",
                        p1.tags.join(", "),
                        p2.tags.join(", ")
                    );
                    println!();

                    // Compare type statistics
                    println!("Phase Type Comparison:");
                    let types = ["scaffold", "implement", "test", "refactor", "fix"];
                    for phase_type in types {
                        let s1 = p1.type_stats.get(phase_type);
                        let s2 = p2.type_stats.get(phase_type);

                        let count1 = s1.map(|s| s.count).unwrap_or(0);
                        let count2 = s2.map(|s| s.count).unwrap_or(0);
                        let avg1 = s1
                            .map(|s| format!("{:.1}", s.avg_iterations))
                            .unwrap_or("-".to_string());
                        let avg2 = s2
                            .map(|s| format!("{:.1}", s.avg_iterations))
                            .unwrap_or("-".to_string());

                        if count1 > 0 || count2 > 0 {
                            println!(
                                "  {:<12} count: {} vs {}, avg iter: {} vs {}",
                                phase_type, count1, count2, avg1, avg2
                            );
                        }
                    }
                    println!();
                }
                (None, _) => {
                    println!("Pattern '{}' not found.", pattern1);
                }
                (_, None) => {
                    println!("Pattern '{}' not found.", pattern2);
                }
            }
        }
    }

    Ok(())
}
