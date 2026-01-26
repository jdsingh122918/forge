//! Swarm executor for orchestrating Claude Code swarm execution.
//!
//! The SwarmExecutor is the bridge between Forge and Claude Code swarms. It:
//! - Starts a CallbackServer to receive progress updates
//! - Builds the orchestration prompt using the SwarmContext
//! - Spawns a Claude Code process with the prompt
//! - Monitors execution via HTTP callbacks and stdout
//! - Parses the `<swarm_complete>` signal to determine success/failure
//! - Returns a SwarmResult with execution details
//!
//! ## Usage
//!
//! ```no_run
//! use forge::swarm::{SwarmContext, PhaseInfo, SwarmExecutor, SwarmConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20);
//! let config = SwarmConfig::default();
//! let executor = SwarmExecutor::new(config);
//!
//! let context = SwarmContext::new(phase, "", PathBuf::from("/project"));
//! let result = executor.execute(context).await?;
//!
//! if result.success {
//!     println!("Swarm completed successfully!");
//! }
//! # Ok(())
//! # }
//! ```

use crate::swarm::callback::{CallbackServer, SwarmEvent, TaskComplete, TaskStatus};
use crate::swarm::context::SwarmContext;
use crate::swarm::prompts::{build_orchestration_prompt, parse_swarm_completion, SwarmCompletionResult};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Default timeout for swarm execution (30 minutes).
const DEFAULT_TIMEOUT_SECS: u64 = 1800;

/// Default polling interval for checking events (100ms).
const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

/// Default Claude command.
const DEFAULT_CLAUDE_CMD: &str = "claude";

/// Configuration for the swarm executor.
#[derive(Debug, Clone)]
pub struct SwarmConfig {
    /// Claude CLI command (default: "claude").
    pub claude_cmd: String,
    /// Working directory for execution.
    pub working_dir: Option<PathBuf>,
    /// Timeout for swarm execution.
    pub timeout: Duration,
    /// Polling interval for event checking.
    pub poll_interval: Duration,
    /// Skip permission prompts in Claude.
    pub skip_permissions: bool,
    /// Verbose output.
    pub verbose: bool,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            claude_cmd: DEFAULT_CLAUDE_CMD.to_string(),
            working_dir: None,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            poll_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
            skip_permissions: true,
            verbose: false,
        }
    }
}

impl SwarmConfig {
    /// Create a new SwarmConfig with custom Claude command.
    pub fn with_claude_cmd(mut self, cmd: &str) -> Self {
        self.claude_cmd = cmd.to_string();
        self
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Enable or disable verbose output.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Enable or disable permission skipping.
    pub fn with_skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }
}

/// Result of swarm execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResult {
    /// Whether the swarm execution succeeded.
    pub success: bool,
    /// Phase number that was executed.
    pub phase: String,
    /// Tasks that completed successfully.
    pub tasks_completed: Vec<String>,
    /// Tasks that failed.
    pub tasks_failed: Vec<String>,
    /// Review results (if any).
    pub reviews: Vec<ReviewOutcome>,
    /// Files changed during execution.
    pub files_changed: Vec<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Total execution duration.
    pub duration: Duration,
    /// Number of progress events received.
    pub progress_events: usize,
    /// Exit code of Claude process.
    pub exit_code: i32,
}

/// Outcome of a review specialist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewOutcome {
    /// Specialist name.
    pub specialist: String,
    /// Review verdict (pass, warn, fail).
    pub verdict: String,
}

impl SwarmResult {
    /// Create a successful result.
    fn success(
        phase: String,
        tasks_completed: Vec<String>,
        files_changed: Vec<String>,
        reviews: Vec<ReviewOutcome>,
        duration: Duration,
        progress_events: usize,
        exit_code: i32,
    ) -> Self {
        Self {
            success: true,
            phase,
            tasks_completed,
            tasks_failed: Vec::new(),
            reviews,
            files_changed,
            error: None,
            duration,
            progress_events,
            exit_code,
        }
    }

    /// Create a failed result.
    fn failure(
        phase: String,
        error: String,
        tasks_completed: Vec<String>,
        tasks_failed: Vec<String>,
        duration: Duration,
        progress_events: usize,
        exit_code: i32,
    ) -> Self {
        Self {
            success: false,
            phase,
            tasks_completed,
            tasks_failed,
            reviews: Vec::new(),
            files_changed: Vec::new(),
            error: Some(error),
            duration,
            progress_events,
            exit_code,
        }
    }
}

/// Internal events during swarm execution.
#[derive(Debug)]
enum ExecutionEvent {
    /// Stdout reading completed (all output captured).
    StdoutComplete { output: String },
    /// Callback event received.
    CallbackReceived(SwarmEvent),
    /// Timeout reached.
    Timeout,
}

/// The swarm executor orchestrates Claude Code swarm execution.
///
/// It manages the lifecycle of a swarm execution:
/// 1. Starting the callback server
/// 2. Building and sending the orchestration prompt
/// 3. Monitoring progress via callbacks and stdout
/// 4. Parsing the completion signal
/// 5. Returning the result
pub struct SwarmExecutor {
    config: SwarmConfig,
}

impl SwarmExecutor {
    /// Create a new SwarmExecutor with the given configuration.
    pub fn new(config: SwarmConfig) -> Self {
        Self { config }
    }

    /// Create a SwarmExecutor with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SwarmConfig::default())
    }

    /// Execute a swarm for the given context.
    ///
    /// This is the main entry point for swarm execution. It:
    /// 1. Starts the callback server
    /// 2. Builds the orchestration prompt
    /// 3. Spawns Claude Code with the prompt
    /// 4. Monitors execution until completion or timeout
    /// 5. Returns the result
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The callback server fails to start
    /// - The Claude process fails to spawn
    /// - The execution times out
    pub async fn execute(&self, mut context: SwarmContext) -> Result<SwarmResult> {
        let start = Instant::now();
        let phase_number = context.phase.number.clone();

        // 1. Start callback server
        let mut callback_server = CallbackServer::new();
        let callback_url = callback_server
            .start()
            .await
            .context("Failed to start callback server")?;

        if self.config.verbose {
            eprintln!("[swarm] Callback server started at {}", callback_url);
        }

        // Update context with actual callback URL
        context.callback_url = callback_url.clone();

        // 2. Build orchestration prompt
        let prompt = build_orchestration_prompt(&context);

        if self.config.verbose {
            eprintln!("[swarm] Generated prompt ({} chars)", prompt.len());
        }

        // 3. Determine working directory
        let working_dir = self
            .config
            .working_dir
            .clone()
            .unwrap_or_else(|| context.working_dir.clone());

        // 4. Spawn Claude process and monitor execution
        let result = self
            .run_claude_process(&prompt, &working_dir, &mut callback_server, start)
            .await;

        // 5. Cleanup callback server
        if let Err(e) = callback_server.stop().await {
            eprintln!("[swarm] Warning: Failed to stop callback server: {}", e);
        }

        // Return result with phase info
        match result {
            Ok(mut res) => {
                res.phase = phase_number;
                Ok(res)
            }
            Err(e) => {
                let duration = start.elapsed();
                Ok(SwarmResult::failure(
                    phase_number,
                    e.to_string(),
                    Vec::new(),
                    Vec::new(),
                    duration,
                    0,
                    -1,
                ))
            }
        }
    }

    /// Run the Claude process and monitor its execution.
    async fn run_claude_process(
        &self,
        prompt: &str,
        working_dir: &PathBuf,
        callback_server: &mut CallbackServer,
        start: Instant,
    ) -> Result<SwarmResult> {
        // Build command
        let mut cmd = Command::new(&self.config.claude_cmd);
        cmd.arg("--print");
        cmd.arg("--output-format").arg("stream-json");

        if self.config.skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }

        // Note: stderr is set to inherit so Claude errors are visible in terminal.
        // Using piped() without reading could cause the process to block if buffer fills.
        cmd.current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        if self.config.verbose {
            eprintln!("[swarm] Spawning Claude process in {:?}", working_dir);
        }

        // Spawn process
        let mut child = cmd.spawn().context("Failed to spawn Claude process")?;

        let child_pid = child.id().unwrap_or(0);
        if self.config.verbose {
            eprintln!("[swarm] Process spawned (PID: {})", child_pid);
        }

        // Write prompt to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .context("Failed to write prompt to stdin")?;
            stdin
                .shutdown()
                .await
                .context("Failed to close stdin")?;
        }

        // Set up stdout reader
        let stdout = child.stdout.take().context("Failed to get stdout")?;
        let reader = BufReader::new(stdout);

        // Create channels for coordinating events
        let (event_tx, mut event_rx) = mpsc::channel::<ExecutionEvent>(100);

        // Spawn task to read stdout
        let stdout_tx = event_tx.clone();
        let verbose = self.config.verbose;
        let stdout_task = tokio::spawn(async move {
            Self::read_stdout(reader, stdout_tx, verbose).await
        });

        // Spawn task to poll callback events
        let callback_tx = event_tx.clone();
        let poll_interval = self.config.poll_interval;
        let callback_state = callback_server.state_clone();
        let callback_task = tokio::spawn(async move {
            Self::poll_callbacks(callback_state, callback_tx, poll_interval).await
        });

        // Spawn timeout task
        let timeout = self.config.timeout;
        let timeout_tx = event_tx;
        let timeout_task = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let _ = timeout_tx.send(ExecutionEvent::Timeout).await;
        });

        // Collect results
        let mut accumulated_output = String::new();
        let mut callback_events: Vec<SwarmEvent> = Vec::new();
        let mut exit_code = -1;
        let mut timed_out = false;
        let mut stdout_complete = false;
        let mut process_exited = false;

        // Wait for both stdout to close AND process to exit (or timeout)
        loop {
            tokio::select! {
                // Check for events from our monitoring tasks
                Some(event) = event_rx.recv() => {
                    match event {
                        ExecutionEvent::StdoutComplete { output } => {
                            accumulated_output = output;
                            stdout_complete = true;
                            // If process already exited, we're done
                            if process_exited {
                                break;
                            }
                        }
                        ExecutionEvent::CallbackReceived(cb_event) => {
                            callback_events.push(cb_event);
                        }
                        ExecutionEvent::Timeout => {
                            timed_out = true;
                            // Kill the process
                            let _ = child.kill().await;
                            break;
                        }
                    }
                }
                // Wait for the process to exit
                result = child.wait(), if !process_exited => {
                    match result {
                        Ok(status) => {
                            exit_code = status.code().unwrap_or(-1);
                            process_exited = true;
                            // If stdout already complete, we're done
                            if stdout_complete {
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("[swarm] Warning: Error waiting for process: {}", e);
                            process_exited = true;
                            if stdout_complete {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Abort monitoring tasks
        stdout_task.abort();
        callback_task.abort();
        timeout_task.abort();

        // Get any remaining callback events
        let remaining_events = callback_server.drain_events().await;
        callback_events.extend(remaining_events);

        let duration = start.elapsed();
        let progress_events = callback_events.len();

        if self.config.verbose {
            eprintln!(
                "[swarm] Execution completed in {:?} (exit: {}, events: {})",
                duration, exit_code, progress_events
            );
        }

        // Handle timeout
        if timed_out {
            return Ok(SwarmResult::failure(
                String::new(),
                format!("Swarm execution timed out after {:?}", self.config.timeout),
                extract_completed_tasks(&callback_events),
                Vec::new(),
                duration,
                progress_events,
                exit_code,
            ));
        }

        // Parse completion signal from output
        if let Some(completion) = parse_swarm_completion(&accumulated_output) {
            return Ok(self.build_result_from_completion(
                completion,
                duration,
                progress_events,
                exit_code,
            ));
        }

        // No completion signal found - check if we have callback data
        let tasks_completed = extract_completed_tasks(&callback_events);
        let files_changed = extract_files_changed(&callback_events);

        if exit_code == 0 && !tasks_completed.is_empty() {
            // Process exited successfully and we have completed tasks
            Ok(SwarmResult::success(
                String::new(),
                tasks_completed,
                files_changed,
                Vec::new(),
                duration,
                progress_events,
                exit_code,
            ))
        } else if exit_code == 0 {
            // Process exited successfully but no completion signal
            Ok(SwarmResult::failure(
                String::new(),
                "No completion signal received from swarm".to_string(),
                tasks_completed,
                Vec::new(),
                duration,
                progress_events,
                exit_code,
            ))
        } else {
            // Process failed
            Ok(SwarmResult::failure(
                String::new(),
                format!("Claude process exited with code {}", exit_code),
                tasks_completed,
                Vec::new(),
                duration,
                progress_events,
                exit_code,
            ))
        }
    }

    /// Read stdout from the Claude process.
    async fn read_stdout(
        reader: BufReader<tokio::process::ChildStdout>,
        tx: mpsc::Sender<ExecutionEvent>,
        verbose: bool,
    ) {
        let mut lines = reader.lines();
        let mut accumulated = String::new();

        while let Ok(Some(line)) = lines.next_line().await {
            if verbose
                && !line.is_empty()
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&line)
                && let Some(msg_type) = json.get("type").and_then(|t| t.as_str())
            {
                eprintln!("[swarm] Event: {}", msg_type);
            }
            accumulated.push_str(&line);
            accumulated.push('\n');
        }

        let _ = tx.send(ExecutionEvent::StdoutComplete {
            output: accumulated,
        }).await;
    }

    /// Poll the callback server for events.
    async fn poll_callbacks(
        state: std::sync::Arc<tokio::sync::RwLock<super::callback::ServerState>>,
        tx: mpsc::Sender<ExecutionEvent>,
        interval: Duration,
    ) {
        let mut ticker = tokio::time::interval(interval);
        let mut last_count = 0;

        loop {
            ticker.tick().await;

            let events = {
                let state = state.read().await;
                if state.events.len() > last_count {
                    let new_events: Vec<_> = state.events.iter().skip(last_count).cloned().collect();
                    last_count = state.events.len();
                    new_events
                } else {
                    Vec::new()
                }
            };

            for event in events {
                if tx.send(ExecutionEvent::CallbackReceived(event)).await.is_err() {
                    return;
                }
            }
        }
    }

    /// Build a SwarmResult from a parsed completion signal.
    fn build_result_from_completion(
        &self,
        completion: SwarmCompletionResult,
        duration: Duration,
        progress_events: usize,
        exit_code: i32,
    ) -> SwarmResult {
        let reviews = completion
            .reviews
            .into_iter()
            .map(|r| ReviewOutcome {
                specialist: r.specialist,
                verdict: r.verdict,
            })
            .collect();

        if completion.success {
            SwarmResult::success(
                completion.phase,
                completion.tasks_completed,
                completion.files_changed,
                reviews,
                duration,
                progress_events,
                exit_code,
            )
        } else {
            SwarmResult {
                success: false,
                phase: completion.phase,
                tasks_completed: completion.tasks_completed,
                tasks_failed: completion.tasks_failed,
                reviews,
                files_changed: completion.files_changed,
                error: completion.error,
                duration,
                progress_events,
                exit_code,
            }
        }
    }
}

/// Extract completed task IDs from callback events.
fn extract_completed_tasks(events: &[SwarmEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| {
            if let SwarmEvent::Complete(TaskComplete {
                task,
                status: TaskStatus::Success,
                ..
            }) = e
            {
                Some(task.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Extract files changed from callback events.
fn extract_files_changed(events: &[SwarmEvent]) -> Vec<String> {
    let mut files: Vec<String> = events
        .iter()
        .filter_map(|e| {
            if let SwarmEvent::Complete(TaskComplete { files_changed, .. }) = e {
                Some(files_changed.clone())
            } else {
                None
            }
        })
        .flatten()
        .collect();

    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::context::PhaseInfo;

    // =========================================
    // SwarmConfig tests
    // =========================================

    #[test]
    fn test_swarm_config_default() {
        let config = SwarmConfig::default();

        assert_eq!(config.claude_cmd, "claude");
        assert!(config.working_dir.is_none());
        assert_eq!(config.timeout, Duration::from_secs(1800));
        assert!(config.skip_permissions);
        assert!(!config.verbose);
    }

    #[test]
    fn test_swarm_config_builder() {
        let config = SwarmConfig::default()
            .with_claude_cmd("custom-claude")
            .with_working_dir(PathBuf::from("/custom/dir"))
            .with_timeout(Duration::from_secs(60))
            .with_verbose(true)
            .with_skip_permissions(false);

        assert_eq!(config.claude_cmd, "custom-claude");
        assert_eq!(config.working_dir, Some(PathBuf::from("/custom/dir")));
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert!(config.verbose);
        assert!(!config.skip_permissions);
    }

    // =========================================
    // SwarmResult tests
    // =========================================

    #[test]
    fn test_swarm_result_success() {
        let result = SwarmResult::success(
            "05".to_string(),
            vec!["task-1".to_string(), "task-2".to_string()],
            vec!["src/auth.rs".to_string()],
            vec![ReviewOutcome {
                specialist: "security".to_string(),
                verdict: "pass".to_string(),
            }],
            Duration::from_secs(120),
            10,
            0,
        );

        assert!(result.success);
        assert_eq!(result.phase, "05");
        assert_eq!(result.tasks_completed.len(), 2);
        assert!(result.tasks_failed.is_empty());
        assert_eq!(result.reviews.len(), 1);
        assert_eq!(result.files_changed, vec!["src/auth.rs"]);
        assert!(result.error.is_none());
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_swarm_result_failure() {
        let result = SwarmResult::failure(
            "05".to_string(),
            "Task failed".to_string(),
            vec!["task-1".to_string()],
            vec!["task-2".to_string()],
            Duration::from_secs(60),
            5,
            1,
        );

        assert!(!result.success);
        assert_eq!(result.phase, "05");
        assert_eq!(result.tasks_completed, vec!["task-1"]);
        assert_eq!(result.tasks_failed, vec!["task-2"]);
        assert_eq!(result.error, Some("Task failed".to_string()));
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_swarm_result_serialization() {
        let result = SwarmResult::success(
            "05".to_string(),
            vec!["task-1".to_string()],
            vec!["file.rs".to_string()],
            vec![],
            Duration::from_secs(30),
            3,
            0,
        );

        let json = serde_json::to_string(&result).unwrap();
        let parsed: SwarmResult = serde_json::from_str(&json).unwrap();

        assert_eq!(result.success, parsed.success);
        assert_eq!(result.phase, parsed.phase);
        assert_eq!(result.tasks_completed, parsed.tasks_completed);
    }

    // =========================================
    // SwarmExecutor tests
    // =========================================

    #[test]
    fn test_swarm_executor_new() {
        let config = SwarmConfig::default();
        let _executor = SwarmExecutor::new(config);
        // Just verify it can be created
    }

    #[test]
    fn test_swarm_executor_with_defaults() {
        let executor = SwarmExecutor::with_defaults();
        assert_eq!(executor.config.claude_cmd, "claude");
    }

    // =========================================
    // Helper function tests
    // =========================================

    #[test]
    fn test_extract_completed_tasks() {
        use crate::swarm::callback::{ProgressUpdate, TaskComplete, TaskStatus};

        let events = vec![
            SwarmEvent::Progress(ProgressUpdate {
                task: "task-1".to_string(),
                status: "running".to_string(),
                percent: Some(50),
                metadata: None,
            }),
            SwarmEvent::Complete(TaskComplete {
                task: "task-1".to_string(),
                status: TaskStatus::Success,
                summary: Some("Done".to_string()),
                error: None,
                files_changed: vec!["a.rs".to_string()],
            }),
            SwarmEvent::Complete(TaskComplete {
                task: "task-2".to_string(),
                status: TaskStatus::Failed,
                summary: None,
                error: Some("Failed".to_string()),
                files_changed: vec![],
            }),
            SwarmEvent::Complete(TaskComplete {
                task: "task-3".to_string(),
                status: TaskStatus::Success,
                summary: Some("Also done".to_string()),
                error: None,
                files_changed: vec!["b.rs".to_string()],
            }),
        ];

        let completed = extract_completed_tasks(&events);
        assert_eq!(completed, vec!["task-1", "task-3"]);
    }

    #[test]
    fn test_extract_files_changed() {
        use crate::swarm::callback::{TaskComplete, TaskStatus};

        let events = vec![
            SwarmEvent::Complete(TaskComplete {
                task: "task-1".to_string(),
                status: TaskStatus::Success,
                summary: None,
                error: None,
                files_changed: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            }),
            SwarmEvent::Complete(TaskComplete {
                task: "task-2".to_string(),
                status: TaskStatus::Success,
                summary: None,
                error: None,
                files_changed: vec!["src/b.rs".to_string(), "src/c.rs".to_string()],
            }),
        ];

        let files = extract_files_changed(&events);
        // Should be sorted and deduplicated
        assert_eq!(files, vec!["src/a.rs", "src/b.rs", "src/c.rs"]);
    }

    #[test]
    fn test_extract_files_changed_empty() {
        let events: Vec<SwarmEvent> = vec![];
        let files = extract_files_changed(&events);
        assert!(files.is_empty());
    }

    // =========================================
    // ReviewOutcome tests
    // =========================================

    #[test]
    fn test_review_outcome_serialization() {
        let outcome = ReviewOutcome {
            specialist: "security-sentinel".to_string(),
            verdict: "pass".to_string(),
        };

        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("security-sentinel"));
        assert!(json.contains("pass"));

        let parsed: ReviewOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.specialist, "security-sentinel");
        assert_eq!(parsed.verdict, "pass");
    }

    // =========================================
    // Integration-style tests (no actual Claude)
    // =========================================

    #[test]
    fn test_build_result_from_completion_success() {
        use crate::swarm::prompts::{ReviewResult, SwarmCompletionResult};

        let executor = SwarmExecutor::with_defaults();
        let completion = SwarmCompletionResult {
            success: true,
            phase: "05".to_string(),
            tasks_completed: vec!["t1".to_string(), "t2".to_string()],
            tasks_failed: vec![],
            reviews: vec![ReviewResult {
                specialist: "security".to_string(),
                verdict: "pass".to_string(),
            }],
            files_changed: vec!["src/main.rs".to_string()],
            error: None,
        };

        let result = executor.build_result_from_completion(
            completion,
            Duration::from_secs(100),
            15,
            0,
        );

        assert!(result.success);
        assert_eq!(result.phase, "05");
        assert_eq!(result.tasks_completed, vec!["t1", "t2"]);
        assert_eq!(result.reviews.len(), 1);
        assert_eq!(result.reviews[0].specialist, "security");
        assert_eq!(result.files_changed, vec!["src/main.rs"]);
        assert_eq!(result.progress_events, 15);
    }

    #[test]
    fn test_build_result_from_completion_failure() {
        use crate::swarm::prompts::{ReviewResult, SwarmCompletionResult};

        let executor = SwarmExecutor::with_defaults();
        let completion = SwarmCompletionResult {
            success: false,
            phase: "05".to_string(),
            tasks_completed: vec!["t1".to_string()],
            tasks_failed: vec!["t2".to_string()],
            reviews: vec![ReviewResult {
                specialist: "security".to_string(),
                verdict: "fail".to_string(),
            }],
            files_changed: vec![],
            error: Some("Security review failed".to_string()),
        };

        let result = executor.build_result_from_completion(
            completion,
            Duration::from_secs(50),
            8,
            1,
        );

        assert!(!result.success);
        assert_eq!(result.phase, "05");
        assert_eq!(result.tasks_completed, vec!["t1"]);
        assert_eq!(result.tasks_failed, vec!["t2"]);
        assert_eq!(result.error, Some("Security review failed".to_string()));
    }

    #[test]
    fn test_swarm_context_integration() {
        // Test that SwarmContext works well with SwarmExecutor
        let phase = PhaseInfo::new("05", "OAuth Integration", "OAUTH COMPLETE", 20);
        let context = SwarmContext::new(phase, "http://localhost:8080", PathBuf::from("/project"));

        assert_eq!(context.phase.number, "05");
        assert_eq!(context.callback_url, "http://localhost:8080");

        // Verify prompt generation works
        let prompt = build_orchestration_prompt(&context);
        assert!(prompt.contains("phase 05"));
        assert!(prompt.contains("OAuth Integration"));
    }

    // =========================================
    // Error recovery tests
    // =========================================

    #[test]
    fn test_swarm_config_timeout_settings() {
        // Test custom timeout configuration
        let custom_timeout = Duration::from_secs(300);
        let config = SwarmConfig::default().with_timeout(custom_timeout);

        assert_eq!(config.timeout, custom_timeout);
        assert_eq!(config.timeout.as_secs(), 300);
    }

    #[test]
    fn test_swarm_config_default_has_reasonable_timeout() {
        // Verify default timeout exists and is reasonable (> 0)
        let config = SwarmConfig::default();

        assert!(config.timeout.as_secs() > 0);
        // Default should be 30 minutes (1800 seconds)
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    }

    #[test]
    fn test_swarm_config_verbose_setting() {
        // Test verbose configuration for both true and false
        let config_verbose = SwarmConfig::default().with_verbose(true);
        assert!(config_verbose.verbose);

        let config_quiet = SwarmConfig::default().with_verbose(false);
        assert!(!config_quiet.verbose);

        // Default should be false
        let config_default = SwarmConfig::default();
        assert!(!config_default.verbose);
    }

    #[test]
    fn test_swarm_config_serialization() {
        // SwarmConfig doesn't derive Serialize/Deserialize, but we can test
        // that values survive through clone (which is used in executor flow)
        let config = SwarmConfig::default()
            .with_timeout(Duration::from_secs(600))
            .with_verbose(true)
            .with_claude_cmd("test-claude");

        // Verify all settings are preserved
        assert_eq!(config.timeout, Duration::from_secs(600));
        assert!(config.verbose);
        assert_eq!(config.claude_cmd, "test-claude");

        // Clone should preserve all values (simulates config passing through executor)
        let cloned = config.clone();
        assert_eq!(cloned.timeout, Duration::from_secs(600));
        assert!(cloned.verbose);
        assert_eq!(cloned.claude_cmd, "test-claude");
    }

    #[test]
    fn test_swarm_result_success_construction() {
        // Test creating a successful SwarmResult with all fields populated
        let result = SwarmResult::success(
            "10".to_string(),
            vec!["task-a".to_string(), "task-b".to_string(), "task-c".to_string()],
            vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            vec![
                ReviewOutcome {
                    specialist: "security-sentinel".to_string(),
                    verdict: "pass".to_string(),
                },
                ReviewOutcome {
                    specialist: "performance-analyst".to_string(),
                    verdict: "pass".to_string(),
                },
            ],
            Duration::from_secs(180),
            25,
            0,
        );

        // Verify success state
        assert!(result.success);
        assert_eq!(result.phase, "10");

        // Verify tasks
        assert_eq!(result.tasks_completed.len(), 3);
        assert!(result.tasks_completed.contains(&"task-a".to_string()));
        assert!(result.tasks_completed.contains(&"task-b".to_string()));
        assert!(result.tasks_completed.contains(&"task-c".to_string()));
        assert!(result.tasks_failed.is_empty());

        // Verify reviews
        assert_eq!(result.reviews.len(), 2);
        assert_eq!(result.reviews[0].specialist, "security-sentinel");
        assert_eq!(result.reviews[1].verdict, "pass");

        // Verify files changed
        assert_eq!(result.files_changed.len(), 2);

        // Verify no error
        assert!(result.error.is_none());

        // Verify metrics
        assert_eq!(result.duration, Duration::from_secs(180));
        assert_eq!(result.progress_events, 25);
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_swarm_result_failure_construction() {
        // Test creating a failed SwarmResult
        let result = SwarmResult::failure(
            "07".to_string(),
            "Connection timeout to external service".to_string(),
            vec!["task-1".to_string()],
            vec!["task-2".to_string(), "task-3".to_string()],
            Duration::from_secs(45),
            12,
            1,
        );

        // Verify failure state
        assert!(!result.success);
        assert_eq!(result.phase, "07");

        // Verify error is set
        assert!(result.error.is_some());
        assert_eq!(
            result.error.as_ref().unwrap(),
            "Connection timeout to external service"
        );

        // Verify tasks
        assert_eq!(result.tasks_completed.len(), 1);
        assert_eq!(result.tasks_failed.len(), 2);
        assert!(result.tasks_failed.contains(&"task-2".to_string()));
        assert!(result.tasks_failed.contains(&"task-3".to_string()));

        // Verify reviews and files are empty for failure
        assert!(result.reviews.is_empty());
        assert!(result.files_changed.is_empty());

        // Verify metrics
        assert_eq!(result.duration, Duration::from_secs(45));
        assert_eq!(result.progress_events, 12);
        assert_eq!(result.exit_code, 1);
    }
}
