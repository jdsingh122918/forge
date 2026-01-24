//! Design document implementation module for forge.
//!
//! This module provides the `forge implement` command which takes a design
//! document and implements it end-to-end using TDD-first phase generation.
//!
//! The workflow:
//! 1. Parse and validate the design document
//! 2. Extract implementation spec and phases via Claude
//! 3. Generate spec.md and phases.json
//! 4. Interactive review and approval
//! 5. Execute phases using the standard orchestrator

pub mod extract;
pub mod spec_gen;
pub mod types;

pub use extract::{extract_design, validate_design_doc, DESIGN_DOC_EXTRACTION_PROMPT};
pub use spec_gen::generate_spec_markdown;
pub use types::{CodePattern, Complexity, Component, ExtractedDesign, ExtractedSpec};

use anyhow::{Context, Result};
use dialoguer::Input;
use std::path::Path;

use crate::generate::{create_phases_file, parse_review_action, ReviewAction};
use crate::init::get_forge_dir;
use crate::phase::{Phase, PhaseType};

/// Run the implement command.
///
/// This is the main entry point for `forge implement <design-doc>`.
///
/// # Arguments
/// * `project_dir` - The project root directory
/// * `design_doc` - Path to the design document
/// * `no_tdd` - Skip test phase generation
/// * `dry_run` - Generate spec and phases without executing
///
/// # Returns
/// `Ok(())` on success, or an error if something fails.
pub fn run_implement(
    project_dir: &Path,
    design_doc: &Path,
    no_tdd: bool,
    dry_run: bool,
) -> Result<()> {
    // 1. Validate and read design doc
    println!("Parsing design doc...");
    let design_content = validate_design_doc(design_doc)?;

    // 2. Extract structure via Claude
    println!("Extracting implementation details...");
    let extracted = extract_design(project_dir, &design_content, design_doc)?;

    // 3. Filter out test phases if --no-tdd and renumber
    let phases = filter_phases(extracted.phases, no_tdd);

    // 4. Generate spec.md
    println!("Generating implementation spec...");
    let spec_content = generate_spec_markdown(&extracted.spec);
    let forge_dir = get_forge_dir(project_dir);

    // Ensure .forge directory exists
    if !forge_dir.exists() {
        std::fs::create_dir_all(&forge_dir)
            .context("Failed to create .forge directory")?;
    }

    std::fs::write(forge_dir.join("spec.md"), &spec_content)
        .context("Failed to write spec.md")?;

    // 5. Save phases.json
    println!("Generating phases...");
    let phases_file = create_phases_file(phases.clone(), &spec_content);
    phases_file
        .save(&forge_dir.join("phases.json"))
        .context("Failed to write phases.json")?;

    // 6. Display phases
    display_phases_with_type(&phases);

    if dry_run {
        println!();
        println!("Dry run complete. Review:");
        println!("  Spec: .forge/spec.md");
        println!("  Phases: .forge/phases.json");
        return Ok(());
    }

    // 7. Interactive review
    loop {
        println!("[a]pprove  [e]dit phase  [r]egenerate  [q]uit");

        let input: String = Input::new()
            .with_prompt(">")
            .allow_empty(false)
            .interact_text()
            .context("Failed to read user input")?;

        match parse_review_action(&input)? {
            ReviewAction::Approve => {
                println!();
                println!("Phases approved. Run 'forge run' to start execution.");
                return Ok(());
            }
            ReviewAction::EditPhase(phase_num) => {
                println!(
                    "\nEditing phase {} - please provide feedback and regenerate.",
                    phase_num
                );
                println!("Phase editing is not yet fully implemented.");
                continue;
            }
            ReviewAction::Regenerate => {
                println!("\nRegenerating...");
                // Re-extract
                let extracted = extract_design(project_dir, &design_content, design_doc)?;
                let phases = filter_phases(extracted.phases, no_tdd);

                // Update spec and phases
                let spec_content = generate_spec_markdown(&extracted.spec);
                std::fs::write(forge_dir.join("spec.md"), &spec_content)
                    .context("Failed to write spec.md during regeneration")?;

                let phases_file = create_phases_file(phases.clone(), &spec_content);
                phases_file
                    .save(&forge_dir.join("phases.json"))
                    .context("Failed to write phases.json during regeneration")?;

                display_phases_with_type(&phases);
                continue;
            }
            ReviewAction::Quit => {
                println!("\nExiting without changes.");
                return Ok(());
            }
        }
    }
}

/// Display phases with their type (test/implement) indicator.
fn display_phases_with_type(phases: &[Phase]) {
    println!();
    for phase in phases {
        let type_tag = match phase.phase_type {
            Some(PhaseType::Test) => "[test]",
            Some(PhaseType::Implement) => "[implement]",
            None => "",
        };

        println!(
            "Phase {}: {} (budget: {}) {}",
            phase.number, phase.name, phase.budget, type_tag
        );
    }
    println!();
}

/// Filter phases based on TDD setting and renumber them.
///
/// If `no_tdd` is true, test phases are filtered out.
/// The remaining phases are then renumbered sequentially.
fn filter_phases(phases: Vec<Phase>, no_tdd: bool) -> Vec<Phase> {
    let filtered = if no_tdd {
        phases
            .into_iter()
            .filter(|p| p.phase_type != Some(PhaseType::Test))
            .collect()
    } else {
        phases
    };
    renumber_phases(filtered)
}

/// Renumber phases sequentially after filtering.
///
/// When --no-tdd filters out test phases, we need to renumber
/// the remaining phases to be sequential (01, 02, 03...) and
/// update their dependencies accordingly.
fn renumber_phases(phases: Vec<Phase>) -> Vec<Phase> {
    if phases.is_empty() {
        return phases;
    }

    // Build a mapping from old number to new number
    let mut number_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (i, phase) in phases.iter().enumerate() {
        let new_number = format!("{:02}", i + 1);
        number_map.insert(phase.number.clone(), new_number);
    }

    // Apply the mapping
    phases
        .into_iter()
        .enumerate()
        .map(|(i, mut phase)| {
            let new_number = format!("{:02}", i + 1);

            // Update dependencies to use new numbers
            phase.depends_on = phase
                .depends_on
                .into_iter()
                .filter_map(|dep| number_map.get(&dep).cloned())
                .collect();

            phase.number = new_number;
            phase
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renumber_phases_empty() {
        let phases: Vec<Phase> = vec![];
        let result = renumber_phases(phases);
        assert!(result.is_empty());
    }

    #[test]
    fn test_renumber_phases_sequential() {
        let phases = vec![
            Phase::new("01", "First", "FIRST DONE", 5, "reason", vec![]),
            Phase::new("02", "Second", "SECOND DONE", 5, "reason", vec!["01".into()]),
        ];

        let result = renumber_phases(phases);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].number, "01");
        assert_eq!(result[1].number, "02");
        assert_eq!(result[1].depends_on, vec!["01"]);
    }

    #[test]
    fn test_renumber_phases_with_gaps() {
        // Simulates filtering where phases 01, 03, 05 remain after removing 02, 04
        let phases = vec![
            Phase::new("01", "First", "FIRST DONE", 5, "reason", vec![]),
            Phase::new("03", "Third", "THIRD DONE", 5, "reason", vec!["01".into()]),
            Phase::new("05", "Fifth", "FIFTH DONE", 5, "reason", vec!["03".into()]),
        ];

        let result = renumber_phases(phases);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].number, "01");
        assert_eq!(result[1].number, "02"); // Was 03, now 02
        assert_eq!(result[2].number, "03"); // Was 05, now 03

        // Dependencies should be updated too
        assert_eq!(result[1].depends_on, vec!["01"]);
        assert_eq!(result[2].depends_on, vec!["02"]); // Was 03, now 02
    }

    #[test]
    fn test_renumber_phases_removes_dangling_deps() {
        // If a dependency was filtered out, it should be removed
        let phases = vec![
            Phase::new("02", "Second", "SECOND DONE", 5, "reason", vec!["01".into()]),
            Phase::new("04", "Fourth", "FOURTH DONE", 5, "reason", vec!["02".into(), "03".into()]),
        ];

        let result = renumber_phases(phases);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].number, "01");
        assert_eq!(result[1].number, "02");

        // "01" dependency is removed (was filtered), "02" becomes "01"
        assert!(result[0].depends_on.is_empty()); // "01" was not in input
        assert_eq!(result[1].depends_on, vec!["01"]); // "02" -> "01", "03" removed
    }
}
