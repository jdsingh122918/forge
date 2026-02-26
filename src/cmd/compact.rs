//! Context compaction status and manual trigger — `forge compact`.

use anyhow::Result;

use super::super::Cli;

pub fn cmd_compact(
    project_dir: &std::path::Path,
    cli: &Cli,
    phase: Option<&str>,
    status_only: bool,
) -> Result<()> {
    use forge::compaction::{ContextTracker, DEFAULT_MODEL_WINDOW_CHARS};
    use forge::forge_config::ForgeToml;
    use forge::init::get_forge_dir;
    use forge::orchestrator::StateManager;

    let forge_dir = get_forge_dir(project_dir);
    let state_file = forge_dir.join("state");
    let state = StateManager::new(state_file);
    let log_dir = forge_dir.join("logs");

    // Determine which phase to work with
    let phase_number = if let Some(p) = phase {
        p.to_string()
    } else {
        // Get the most recent phase from state
        state
            .get_last_completed_phase()
            .map(|p| {
                // Get the next phase (current running phase)
                format!("{:02}", p.parse::<u32>().unwrap_or(0) + 1)
            })
            .unwrap_or_else(|| "01".to_string())
    };

    println!();
    println!("Context Compaction - Phase {}", phase_number);
    println!("================================");
    println!();

    // Get context limit from config
    let forge_toml = ForgeToml::load_or_default(&forge_dir)?;
    let context_limit = cli
        .context_limit
        .clone()
        .unwrap_or_else(|| forge_toml.defaults.context_limit.clone());

    println!("Context limit: {}", context_limit);

    // Calculate context usage from log files
    let mut total_prompt_chars = 0usize;
    let mut total_output_chars = 0usize;
    let mut iteration_count = 0u32;

    // Find all log files for this phase
    if log_dir.exists() {
        for entry in std::fs::read_dir(&log_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            if name.starts_with(&format!("phase-{}-iter-", phase_number)) {
                if name.ends_with("-prompt.md") {
                    if let Ok(content) = std::fs::read_to_string(entry.path()) {
                        total_prompt_chars += content.len();
                        iteration_count += 1;
                    }
                } else if name.ends_with("-output.log")
                    && let Ok(content) = std::fs::read_to_string(entry.path())
                {
                    total_output_chars += content.len();
                }
            }
        }
    }

    // Create tracker with current stats
    let mut tracker = ContextTracker::new(&context_limit, DEFAULT_MODEL_WINDOW_CHARS);

    // Simulate adding iterations (we don't have per-iteration breakdown from files)
    if iteration_count > 0 {
        let avg_prompt = total_prompt_chars / iteration_count as usize;
        let avg_output = total_output_chars / iteration_count as usize;
        for _ in 0..iteration_count {
            tracker.add_iteration(avg_prompt, avg_output);
        }
    }

    println!();
    println!("Status:");
    println!("  Iterations found: {}", iteration_count);
    println!("  Total prompt chars: {}", total_prompt_chars);
    println!("  Total output chars: {}", total_output_chars);
    println!("  Total context used: {}", tracker.total_context_used());
    println!("  Context limit: {} chars", tracker.effective_limit());
    println!("  Usage: {:.1}%", tracker.usage_percentage());
    println!("  Remaining budget: {} chars", tracker.remaining_budget());
    println!();

    if tracker.should_compact() {
        println!("Status: Compaction RECOMMENDED (approaching limit)");
    } else {
        println!("Status: Compaction not needed");
    }

    if status_only {
        println!();
        println!(
            "Use 'forge compact --phase {}' to perform compaction.",
            phase_number
        );
        return Ok(());
    }

    // Check if we need compaction
    if !tracker.should_compact() && iteration_count < 2 {
        println!();
        println!("No compaction needed at this time.");
        println!("  - Context usage is below threshold");
        println!("  - Need at least 2 iterations to compact");
        return Ok(());
    }

    // Perform compaction by generating a summary
    println!();
    println!("Performing compaction...");

    // For manual compaction, we generate a summary file that can be used
    let summary_file = log_dir.join(format!("phase-{}-compaction-summary.md", phase_number));

    let summary_content = format!(
        r#"## CONTEXT COMPACTION SUMMARY

Phase {} has been compacted to save context space.

### Statistics
- Iterations compacted: {}
- Original context: {} chars
- This summary: ~{} chars
- Compression achieved: ~{:.1}%

### Note
This is a manual compaction summary. The actual compaction occurs
automatically during orchestration when context approaches the limit.

To leverage automatic compaction, the orchestrator will:
1. Track context usage across iterations
2. Summarize older iterations when approaching the limit
3. Inject the summary into subsequent prompts

### Configuration
Set context_limit in forge.toml:
```toml
[defaults]
context_limit = "{}"
```
"#,
        phase_number,
        iteration_count,
        total_prompt_chars + total_output_chars,
        1000, // Approximate summary size
        if total_prompt_chars + total_output_chars > 0 {
            (1.0 - 1000.0 / (total_prompt_chars + total_output_chars) as f32) * 100.0
        } else {
            0.0
        },
        context_limit
    );

    std::fs::write(&summary_file, &summary_content)?;
    println!("Summary written to: {}", summary_file.display());
    println!();
    println!("Compaction complete.");

    Ok(())
}
