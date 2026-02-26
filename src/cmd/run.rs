//! Sequential phase orchestration — `forge run` and `forge phase <N>`.

use anyhow::Result;
use std::path::PathBuf;

use super::super::Cli;

pub fn check_run_prerequisites(project_dir: &std::path::Path) -> Result<()> {
    use forge::init::{has_phases, is_initialized};

    if !is_initialized(project_dir) {
        anyhow::bail!(
            "Project not initialized. Run 'forge init' first, then 'forge generate' to create phases."
        );
    }
    if !has_phases(project_dir) {
        anyhow::bail!(
            "No phases found. Run 'forge generate' first to create phases from your spec."
        );
    }
    Ok(())
}

pub async fn run_orchestrator(
    cli: &Cli,
    project_dir: PathBuf,
    start_phase: Option<String>,
) -> Result<()> {
    use forge::audit::{AuditLogger, FileChangeSummary, PhaseAudit, PhaseOutcome, RunConfig};
    use forge::compaction::{
        CompactionManager, DEFAULT_MODEL_WINDOW_CHARS, extract_output_summary,
    };
    use forge::config::Config;
    use forge::forge_config::{ForgeToml, PermissionMode};
    use forge::gates::{ApprovalGate, GateDecision, IterationDecision, ProgressTracker};
    use forge::hooks::{HookAction, HookManager};
    use forge::init::get_forge_dir;
    use forge::orchestrator::{ClaudeRunner, IterationFeedback, PromptContext, StateManager};
    use forge::phase::load_phases_or_default;
    use forge::tracker::GitTracker;
    use forge::ui::OrchestratorUI;

    check_run_prerequisites(&project_dir)?;

    let config = Config::new(
        project_dir.clone(),
        cli.verbose,
        cli.auto_approve_threshold,
        cli.spec_file.clone(),
    )?;
    config.ensure_directories()?;

    // Initialize hook manager
    let mut hook_manager = HookManager::new(&project_dir, cli.verbose)?;

    // Merge hooks from forge.toml if it exists
    let forge_dir = get_forge_dir(&project_dir);
    if let Ok(toml) = ForgeToml::load_or_default(&forge_dir)
        && !toml.hooks.definitions.is_empty()
    {
        hook_manager.merge_config(toml.hooks.into_hooks_config());
    }

    // Report hook count if any
    let hook_count = hook_manager.hook_count();
    if hook_count > 0 && cli.verbose {
        println!("Loaded {} hook(s)", hook_count);
    }

    let state = StateManager::new(config.state_file.clone());
    let tracker = GitTracker::new(&config.project_dir)?;
    let runner = ClaudeRunner::new(config.clone());
    let mut audit = AuditLogger::new(&config.audit_dir);
    let mut gate = ApprovalGate::new(cli.auto_approve_threshold, cli.yes);

    // Determine starting phase
    let start = start_phase.unwrap_or_else(|| {
        state
            .get_last_completed_phase()
            .map(|p| format!("{:02}", p.parse::<u32>().unwrap_or(0) + 1))
            .unwrap_or_else(|| "01".to_string())
    });

    // Load phases from phases.json if it exists, otherwise use defaults
    let all_phases = load_phases_or_default(Some(&config.phases_file))?;

    // Apply permission modes from config to each phase
    let forge_toml = ForgeToml::load_or_default(&forge_dir)?;
    let phases: Vec<_> = all_phases
        .into_iter()
        .filter(|p| p.number.as_str() >= start.as_str())
        .map(|mut p| {
            // Get phase settings from config (includes pattern-matched overrides)
            let settings = forge_toml.phase_settings(&p.name);
            // Apply permission mode from config if phase doesn't have one explicitly set
            // (phases.json can override with explicit permission_mode)
            if p.permission_mode == PermissionMode::Standard {
                p.permission_mode = settings.permission_mode;
            }
            // Also apply budget from config if it differs and wasn't explicitly set in phases.json
            // (We don't override budget here since phases.json is the primary source)
            p
        })
        .collect();
    let ui = std::sync::Arc::new(OrchestratorUI::new(phases.len() as u64, cli.verbose));

    // Start audit run
    audit.start_run(RunConfig {
        auto_approve_threshold: cli.auto_approve_threshold,
        skip_permissions: config.skip_permissions,
        verbose: cli.verbose,
        spec_file: config.spec_file.clone(),
        project_dir: config.project_dir.clone(),
    })?;

    let mut previous_changes: Option<FileChangeSummary> = None;

    for phase in phases {
        // Run OnApproval hooks first (can auto-approve/reject)
        let approval_result = hook_manager
            .run_on_approval(&phase, previous_changes.as_ref())
            .await?;

        let decision = match approval_result.action {
            HookAction::Approve => {
                if cli.verbose {
                    println!(
                        "  {} (hook auto-approved)",
                        console::style("Auto-approved").dim()
                    );
                }
                GateDecision::Approved
            }
            HookAction::Reject => {
                if let Some(msg) = &approval_result.message {
                    println!("  Hook rejected: {}", msg);
                }
                GateDecision::Rejected
            }
            HookAction::Block => {
                if let Some(msg) = &approval_result.message {
                    println!("  Hook blocked: {}", msg);
                }
                audit.finish_run()?;
                return Ok(());
            }
            _ => {
                // No hook decision, use normal approval gate
                gate.check_phase(&phase, previous_changes.as_ref(), &ui)?
            }
        };

        match decision {
            GateDecision::Aborted => {
                println!("Orchestrator aborted by user");
                audit.finish_run()?;
                return Ok(());
            }
            GateDecision::Rejected => {
                println!("Phase {} skipped", phase.number);
                continue;
            }
            GateDecision::Approved | GateDecision::ApprovedAll => {}
        }

        // Run PrePhase hooks
        let pre_phase_result = hook_manager
            .run_pre_phase(&phase, previous_changes.as_ref())
            .await?;

        match pre_phase_result.action {
            HookAction::Block => {
                if let Some(msg) = &pre_phase_result.message {
                    println!("  PrePhase hook blocked: {}", msg);
                }
                audit.finish_run()?;
                return Ok(());
            }
            HookAction::Skip => {
                if let Some(msg) = &pre_phase_result.message {
                    println!("  PrePhase hook skipped phase: {}", msg);
                }
                continue;
            }
            _ => {}
        }

        ui.start_phase(&phase.number, &phase.name);
        state.save(&phase.number, 0, "started")?;

        let mut phase_audit = PhaseAudit::new(&phase.number, &phase.name, &phase.promise);

        // Take git snapshot before phase
        let snapshot_sha = tracker.snapshot_before(&phase.number)?;

        // Initialize progress tracker for autonomous mode
        let mut progress_tracker = ProgressTracker::new();

        // Initialize compaction manager for this phase
        let context_limit = cli
            .context_limit
            .clone()
            .unwrap_or_else(|| forge_toml.phase_settings(&phase.name).context_limit);
        let mut compaction_manager = CompactionManager::new(
            &phase.number,
            &phase.name,
            &phase.promise,
            &context_limit,
            DEFAULT_MODEL_WINDOW_CHARS,
        );

        // Track current prompt context (compaction summary if any)
        let mut current_prompt_context: Option<PromptContext> = None;

        // Session continuity: track active session ID for --resume across iterations
        let mut active_session_id: Option<String> = None;
        // Iteration feedback: track feedback to inject via --append-system-prompt
        let mut previous_feedback: Option<String> = None;

        // Check if session continuity and iteration feedback are enabled
        let session_continuity_enabled = forge_toml.claude.session_continuity;
        let iteration_feedback_enabled = forge_toml.claude.iteration_feedback;

        let mut completed = false;
        let mut phase_aborted = false;
        for iter in 1..=phase.budget {
            // === STRICT MODE: Per-iteration approval ===
            if phase.permission_mode == PermissionMode::Strict {
                let current_changes = tracker.compute_changes(&snapshot_sha)?;
                match gate.check_iteration(&phase, iter, Some(&current_changes), &ui)? {
                    IterationDecision::Continue => {}
                    IterationDecision::Skip => {
                        println!("  Iteration {} skipped by user", iter);
                        continue;
                    }
                    IterationDecision::StopPhase => {
                        println!("  Phase stopped by user at iteration {}", iter);
                        break;
                    }
                    IterationDecision::Abort => {
                        println!("  Orchestrator aborted by user");
                        phase_aborted = true;
                        break;
                    }
                }
            }

            // === AUTONOMOUS MODE: Check progress before continuing ===
            if phase.permission_mode == PermissionMode::Autonomous
                && iter > 1
                && !gate.check_autonomous_progress(&progress_tracker)
            {
                match gate.prompt_no_progress()? {
                    IterationDecision::Continue => {
                        // Reset stale counter and continue
                        progress_tracker.stale_iterations = 0;
                    }
                    IterationDecision::StopPhase => {
                        println!("  Phase stopped due to no progress");
                        break;
                    }
                    IterationDecision::Abort => {
                        println!("  Orchestrator aborted by user");
                        phase_aborted = true;
                        break;
                    }
                    IterationDecision::Skip => {
                        continue;
                    }
                }
            }

            // Run PreIteration hooks
            let pre_iter_result = hook_manager.run_pre_iteration(&phase, iter).await?;

            match pre_iter_result.action {
                HookAction::Block => {
                    if let Some(msg) = &pre_iter_result.message {
                        println!("  PreIteration hook blocked: {}", msg);
                    }
                    break;
                }
                HookAction::Skip => {
                    if let Some(msg) = &pre_iter_result.message {
                        println!("  PreIteration hook skipped iteration: {}", msg);
                    }
                    continue;
                }
                _ => {}
            }

            ui.start_iteration(iter, phase.budget);

            // Check if compaction is needed before this iteration
            if let Some(summary_text) = compaction_manager.compact_if_needed() {
                if cli.verbose {
                    println!("  Context compacted: {}", compaction_manager.status());
                }
                // Record compaction in audit
                if let Some(compaction) = compaction_manager.last_compaction() {
                    phase_audit.add_compaction_event(
                        compaction.iterations_summarized,
                        compaction.original_chars,
                        compaction.summary_chars,
                    );
                }
                current_prompt_context = Some(PromptContext::with_compaction(summary_text));
                // Reset session on compaction — the compacted context replaces history
                active_session_id = None;
                previous_feedback = None;
            }

            // Run iteration with optional compaction context, session resumption, and feedback
            let result = runner
                .run_iteration_with_context(
                    &phase,
                    iter,
                    Some(ui.clone()),
                    current_prompt_context.as_ref(),
                    if session_continuity_enabled {
                        active_session_id.as_deref()
                    } else {
                        None
                    },
                    if iteration_feedback_enabled {
                        previous_feedback.as_deref()
                    } else {
                        None
                    },
                )
                .await?;

            // Compute changes
            let changes = tracker.compute_changes(&snapshot_sha)?;
            ui.update_files(&changes);

            // === READONLY MODE: Validate no modifications ===
            if phase.permission_mode == PermissionMode::Readonly
                && let Err(e) = gate.validate_readonly_changes(&phase, &changes)
            {
                println!("  {} {}", console::style("Error:").red().bold(), e);
                ui.phase_failed(&phase.number, "readonly mode violation");
                phase_audit.finish(
                    PhaseOutcome::Error {
                        message: e.to_string(),
                    },
                    changes.clone(),
                );
                break;
            }

            // Show individual file changes
            for path in &changes.files_added {
                ui.show_file_change(path, forge::audit::ChangeType::Added);
            }
            for path in &changes.files_modified {
                ui.show_file_change(path, forge::audit::ChangeType::Modified);
            }

            // Update progress tracker for autonomous mode
            let progress_pct = result.signals.latest_progress();
            progress_tracker.update(&changes, progress_pct);

            // Capture session ID for --resume in next iteration
            if let Some(ref sid) = result.session.session_id {
                active_session_id = Some(sid.clone());
            }

            // Build iteration feedback for next iteration
            previous_feedback = IterationFeedback::new()
                .with_iteration_status(iter, phase.budget, result.promise_found)
                .with_git_changes(&changes)
                .with_signals(&result.signals)
                .build();

            // Record iteration in compaction manager
            let output_summary = extract_output_summary(&result.output, 100);
            compaction_manager.record_iteration(
                iter,
                result.session.prompt_chars,
                result.session.output_chars,
                &changes,
                &result.signals,
                &output_summary,
            );

            // Show context status in verbose mode
            if cli.verbose && iter > 1 {
                println!("  {}", compaction_manager.status());
            }

            // Run PostIteration hooks with signals
            let post_iter_result = hook_manager
                .run_post_iteration_with_signals(
                    &phase,
                    iter,
                    &changes,
                    result.promise_found,
                    Some(&result.output),
                    &result.signals,
                )
                .await?;

            // Handle blockers - pause and prompt user if there are unacknowledged blockers
            if result.signals.has_unacknowledged_blockers() {
                let blockers = result.signals.unacknowledged_blockers();
                for blocker in &blockers {
                    ui.show_blocker(&blocker.description);
                }

                // Auto-continue if --yes flag is set
                let continue_anyway = if cli.yes {
                    println!(
                        "  {} {} blocker(s) detected, auto-continuing (--yes flag)",
                        console::style("⚠").yellow(),
                        blockers.len()
                    );
                    true
                } else {
                    // Prompt user about blockers
                    use dialoguer::Confirm;
                    Confirm::new()
                        .with_prompt(format!(
                            "{} blocker(s) detected. Continue anyway?",
                            blockers.len()
                        ))
                        .default(true)
                        .interact()
                        .unwrap_or(true)
                };

                if !continue_anyway {
                    ui.phase_failed(&phase.number, "User stopped due to blockers");
                    phase_audit.finish(PhaseOutcome::UserAborted, changes.clone());
                    break;
                }
            }

            // PostIteration hook can override promise detection
            let should_complete = match post_iter_result.action {
                HookAction::Block => {
                    if let Some(msg) = &post_iter_result.message {
                        println!("  PostIteration hook blocked: {}", msg);
                    }
                    false
                }
                _ => result.promise_found,
            };

            if should_complete {
                ui.iteration_success(iter);
                phase_audit.finish(PhaseOutcome::Completed { iteration: iter }, changes.clone());
                state.save(&phase.number, iter, "completed")?;
                previous_changes = Some(changes);
                completed = true;
                break;
            } else {
                // Show progress if available
                if let Some(pct) = result.signals.latest_progress() {
                    ui.iteration_bar_message(iter, phase.budget, &format!("{}% done", pct));
                }
                ui.iteration_continue(iter);
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        // Handle phase abort (exit orchestrator entirely)
        if phase_aborted {
            audit.finish_run()?;
            return Ok(());
        }

        if !completed {
            let changes = tracker.compute_changes(&snapshot_sha)?;

            // Run OnFailure hooks
            let failure_result = hook_manager
                .run_on_failure(&phase, phase.budget, &changes)
                .await?;

            if let Some(msg) = &failure_result.message {
                println!("  OnFailure hook: {}", msg);
            }

            phase_audit.finish(PhaseOutcome::MaxIterationsReached, changes);
            state.save(&phase.number, phase.budget, "max_iterations")?;
            ui.phase_failed(&phase.number, "max iterations reached");
        } else {
            // Run PostPhase hooks
            let post_phase_result = hook_manager
                .run_post_phase(
                    &phase,
                    phase.budget,
                    previous_changes
                        .as_ref()
                        .unwrap_or(&FileChangeSummary::default()),
                    true,
                )
                .await?;

            if let Some(msg) = &post_phase_result.message
                && cli.verbose
            {
                println!("  PostPhase hook: {}", msg);
            }

            ui.phase_complete(&phase.number);
        }

        audit.add_phase(phase_audit)?;

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    let run_file = audit.finish_run()?;
    println!("Audit log saved to: {}", run_file.display());

    Ok(())
}

pub async fn run_single_phase(cli: &Cli, project_dir: PathBuf, phase_num: &str) -> Result<()> {
    use forge::init::get_forge_dir;
    use forge::phase::PhasesFile;

    check_run_prerequisites(&project_dir)?;

    // Load phases from phases.json
    let forge_dir = get_forge_dir(&project_dir);
    let phases_file = forge_dir.join("phases.json");
    let pf = PhasesFile::load(&phases_file)?;

    let phase = pf
        .phases
        .into_iter()
        .find(|p| p.number == phase_num)
        .ok_or_else(|| anyhow::anyhow!("Unknown phase: {}", phase_num))?;

    run_orchestrator(cli, project_dir, Some(phase.number)).await
}
