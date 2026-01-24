use crate::audit::FileChangeSummary;
use crate::phase::Phase;
use crate::ui::OrchestratorUI;
use anyhow::Result;
use dialoguer::{Select, theme::ColorfulTheme};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateDecision {
    Approved,
    ApprovedAll, // User chose "yes to all"
    Rejected,
    Aborted,
}

pub struct ApprovalGate {
    pub auto_threshold: usize,
    pub skip_all: bool,
}

impl ApprovalGate {
    pub fn new(auto_threshold: usize, skip_all: bool) -> Self {
        Self {
            auto_threshold,
            skip_all,
        }
    }

    pub fn check_phase(
        &mut self,
        phase: &Phase,
        previous_changes: Option<&FileChangeSummary>,
        ui: &OrchestratorUI,
    ) -> Result<GateDecision> {
        // Display phase header
        ui.print_phase_header(&phase.number, &phase.name, &phase.promise, phase.budget);

        // Show previous changes if any
        if let Some(changes) = previous_changes {
            ui.print_previous_changes(changes);
        }

        // If --yes flag, auto-approve everything
        if self.skip_all {
            println!("  {} (--yes flag)", console::style("Auto-approved").dim());
            return Ok(GateDecision::Approved);
        }

        // Check threshold-based auto-approval
        if let Some(changes) = previous_changes {
            if changes.total_files() <= self.auto_threshold && changes.total_files() > 0 {
                println!(
                    "  {} (≤{} files changed)",
                    console::style("Auto-approved").dim(),
                    self.auto_threshold
                );
                return Ok(GateDecision::Approved);
            }
        }

        // Interactive prompt
        self.prompt_user()
    }

    fn prompt_user(&mut self) -> Result<GateDecision> {
        let options = &[
            "Yes, run this phase",
            "Yes, and auto-approve remaining phases (--yes)",
            "Skip this phase",
            "Abort orchestrator",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Proceed with this phase?")
            .items(options)
            .default(0)
            .interact()?;

        match selection {
            0 => Ok(GateDecision::Approved),
            1 => {
                self.skip_all = true;
                Ok(GateDecision::ApprovedAll)
            }
            2 => Ok(GateDecision::Rejected),
            3 => Ok(GateDecision::Aborted),
            _ => unreachable!(),
        }
    }
}
