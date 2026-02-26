//! Project initialization, interview, generate, and implement commands.

use anyhow::Result;

pub fn cmd_init(project_dir: &std::path::Path, from_pattern: Option<&str>) -> Result<()> {
    use forge::init::{init_project, is_initialized};

    let was_initialized = is_initialized(project_dir);

    let result = init_project(project_dir, from_pattern)?;

    if result.created {
        println!(
            "Initialized forge project at {}",
            result.forge_dir.display()
        );
        println!();
        println!("Created directory structure:");
        println!("  .forge/");
        println!("  ├── spec.md       # Generated spec (use `forge interview`)");
        println!("  ├── phases.json   # Generated phases (use `forge generate`)");
        println!("  ├── state         # Execution state");
        println!("  ├── audit/runs/   # Audit trail");
        println!("  ├── prompts/      # Custom prompt overrides");
        println!("  └── skills/       # Reusable prompt fragments (use `forge skills`)");
        println!();
        println!("Next steps:");
        println!("  1. Run `forge interview` to create your spec");
        println!("  2. Run `forge generate` to create phases from the spec");
        println!("  3. Run `forge run` to start execution");
    } else if was_initialized {
        println!(
            "Forge project already initialized at {}",
            result.forge_dir.display()
        );
        println!("Directory structure verified.");
    } else {
        println!(
            "Completed forge initialization at {}",
            result.forge_dir.display()
        );
    }

    Ok(())
}

pub fn cmd_interview(project_dir: &std::path::Path) -> Result<()> {
    use forge::interview::run_interview;
    run_interview(project_dir)
}

pub fn cmd_generate(
    project_dir: &std::path::Path,
    spec_file: Option<&std::path::Path>,
    auto_approve: bool,
) -> Result<()> {
    use forge::generate::run_generate;
    run_generate(project_dir, spec_file, auto_approve)
}

pub fn cmd_implement(
    project_dir: &std::path::Path,
    design_doc: &std::path::Path,
    no_tdd: bool,
    dry_run: bool,
) -> Result<()> {
    use forge::implement::run_implement;
    run_implement(project_dir, design_doc, no_tdd, dry_run)
}
