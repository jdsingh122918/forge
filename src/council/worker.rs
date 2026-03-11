use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::sleep;

use crate::audit::TokenUsage;
use crate::council::config::WorkerConfig;
use crate::council::prompts::review_prompt;
use crate::council::types::{ReviewResult, ReviewScores, ReviewVerdict, WorkerResult};
use crate::phase::Phase;
use crate::signals::extract_signals;
use crate::stream::{ContentBlock, StreamEvent};

#[async_trait]
pub trait Worker: Send + Sync {
    fn name(&self) -> &str;

    async fn execute(
        &self,
        phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult>;

    async fn review(
        &self,
        phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult>;
}

#[derive(Debug, Clone)]
pub struct ClaudeWorker {
    command: String,
    flags: Vec<String>,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeReviewPayload {
    candidate_label: String,
    verdict: String,
    request_changes_reason: Option<String>,
    scores: ReviewScores,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    summary: String,
}

#[derive(Debug)]
struct ParsedClaudeOutput {
    combined_output: String,
    is_error: bool,
    token_usage: Option<TokenUsage>,
}

impl ClaudeWorker {
    pub fn new(config: &WorkerConfig) -> Self {
        let name = std::path::Path::new(&config.cmd)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("claude")
            .to_string();

        Self {
            command: config.cmd.clone(),
            flags: config.flags.clone(),
            name,
        }
    }

    pub fn build_execute_args(
        &self,
        _phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Vec<String> {
        let mut args = self.base_args();
        args.push("--cwd".to_string());
        args.push(worktree_path.display().to_string());
        args.push("-p".to_string());
        args.push(prompt.to_string());
        args
    }

    pub fn parse_execute_output(&self, raw: &str) -> Result<WorkerResult> {
        let parsed = self.parse_stream_output(raw)?;
        let signals = extract_signals(raw);

        Ok(WorkerResult {
            worker_name: self.name.clone(),
            diff_text: parsed.combined_output.clone(),
            exit_code: if parsed.is_error { 1 } else { 0 },
            duration: Duration::ZERO,
            token_usage: parsed.token_usage,
            raw_output: parsed.combined_output.clone(),
            signals: serialize_signals(&signals),
        })
    }

    pub fn build_review_args(&self, phase: &Phase, diff: &str, label: &str) -> Vec<String> {
        let mut args = self.base_args();
        args.push("-p".to_string());
        args.push(review_prompt(phase, diff, label));
        args
    }

    pub fn parse_review_output(&self, raw: &str) -> Result<ReviewResult> {
        let parsed = self.parse_stream_output(raw)?;
        let payload: ClaudeReviewPayload = serde_json::from_str(parsed.combined_output.trim())
            .context("failed to parse Claude review output as JSON")?;

        let verdict = match payload.verdict.as_str() {
            "approve" => ReviewVerdict::Approve,
            "request_changes" => ReviewVerdict::RequestChanges(
                payload
                    .request_changes_reason
                    .unwrap_or_else(|| "changes requested".to_string()),
            ),
            "abstain" => ReviewVerdict::Abstain,
            other => anyhow::bail!("unsupported Claude review verdict `{other}`"),
        };

        Ok(ReviewResult {
            reviewer_name: self.name.clone(),
            candidate_label: payload.candidate_label,
            verdict,
            scores: payload.scores,
            issues: payload.issues,
            summary: payload.summary,
            duration: Duration::ZERO,
        })
    }

    fn base_args(&self) -> Vec<String> {
        let mut args = self.flags.clone();

        if !args.iter().any(|arg| arg == "--print") {
            args.push("--print".to_string());
        }

        if !args
            .windows(2)
            .any(|pair| pair == ["--output-format", "stream-json"])
        {
            args.push("--output-format".to_string());
            args.push("stream-json".to_string());
        }

        if !args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions")
        {
            args.push("--dangerously-skip-permissions".to_string());
        }

        args
    }

    fn parse_stream_output(&self, raw: &str) -> Result<ParsedClaudeOutput> {
        let mut accumulated_text = String::new();
        let mut final_result: Option<String> = None;
        let mut is_error = false;
        let mut token_usage: Option<TokenUsage> = None;

        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<StreamEvent>(line) {
                Ok(event) => match event {
                    StreamEvent::Assistant { message, .. } => {
                        for content in message.content {
                            if let ContentBlock::Text { text } = content {
                                accumulated_text.push_str(&text);
                                accumulated_text.push('\n');
                            }
                        }
                    }
                    StreamEvent::Result {
                        result,
                        is_error: err,
                        ..
                    } => {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                            token_usage = extract_token_usage(&parsed);
                        }
                        final_result = result;
                        is_error = err;
                    }
                    StreamEvent::User { .. } | StreamEvent::System { .. } => {}
                },
                Err(_) => {
                    accumulated_text.push_str(line);
                    accumulated_text.push('\n');
                }
            }
        }

        let combined_output = final_result
            .unwrap_or(accumulated_text)
            .trim_end_matches('\n')
            .to_string();

        Ok(ParsedClaudeOutput {
            combined_output,
            is_error,
            token_usage,
        })
    }

    async fn run_command(
        &self,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> Result<(String, i32)> {
        let mut cmd = Command::new(&self.command);
        cmd.args(args);
        if let Some(dir) = current_dir {
            cmd.current_dir(dir);
        }

        let output = cmd
            .output()
            .await
            .with_context(|| format!("failed to execute `{}`", self.command))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let raw = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
            (false, true) => stdout,
            (true, false) => stderr,
            (false, false) => format!("{stdout}\n{stderr}"),
            (true, true) => String::new(),
        };
        let exit_code = output
            .status
            .code()
            .unwrap_or_else(|| if output.status.success() { 0 } else { 1 });

        Ok((raw, exit_code))
    }
}

fn serialize_signals(signals: &crate::signals::IterationSignals) -> Vec<String> {
    let mut serialized = Vec::new();

    for progress in &signals.progress {
        serialized.push(format!("<progress>{}</progress>", progress.raw_value));
    }
    for blocker in &signals.blockers {
        serialized.push(format!("<blocker>{}</blocker>", blocker.description));
    }
    for pivot in &signals.pivots {
        serialized.push(format!("<pivot>{}</pivot>", pivot.new_approach));
    }

    serialized
}

fn extract_token_usage(parsed: &serde_json::Value) -> Option<TokenUsage> {
    let usage = parsed.get("usage").or_else(|| {
        parsed
            .get("response")
            .and_then(|response| response.get("usage"))
    })?;
    Some(TokenUsage {
        input_tokens: usage.get("input_tokens")?.as_u64()?.try_into().ok()?,
        output_tokens: usage.get("output_tokens")?.as_u64()?.try_into().ok()?,
    })
}

fn has_any_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter()
        .any(|arg| flags.iter().any(|candidate| arg == candidate))
}

fn extract_codex_text(parsed: &serde_json::Value) -> Option<String> {
    match parsed {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let segments = items
                .iter()
                .filter_map(extract_codex_text)
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>();

            if segments.is_empty() {
                None
            } else {
                Some(segments.join("\n"))
            }
        }
        serde_json::Value::Object(_) => {
            for key in ["output_text", "result", "content", "text", "delta"] {
                if let Some(text) = parsed.get(key).and_then(extract_codex_text) {
                    return Some(text);
                }
            }

            for key in ["message", "response", "item"] {
                if let Some(text) = parsed.get(key).and_then(extract_codex_text) {
                    return Some(text);
                }
            }

            None
        }
        _ => None,
    }
}

fn codex_event_is_final(parsed: &serde_json::Value) -> bool {
    if parsed.get("result").is_some()
        || parsed.get("output_text").is_some()
        || parsed
            .get("response")
            .and_then(|response| response.get("output_text"))
            .is_some()
    {
        return true;
    }

    if let Some(event_type) = parsed.get("type").and_then(serde_json::Value::as_str)
        && (event_type.contains("completed") || event_type == "result")
    {
        return true;
    }

    matches!(
        parsed.get("status").and_then(serde_json::Value::as_str),
        Some("completed" | "success" | "failed" | "error")
    )
}

fn codex_event_is_error(parsed: &serde_json::Value) -> bool {
    if parsed
        .get("is_error")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }

    if parsed.get("error").is_some() {
        return true;
    }

    if let Some(event_type) = parsed.get("type").and_then(serde_json::Value::as_str)
        && (event_type.contains("error") || event_type.contains("failed"))
    {
        return true;
    }

    if matches!(
        parsed.get("status").and_then(serde_json::Value::as_str),
        Some("error" | "failed")
    ) {
        return true;
    }

    matches!(
        parsed
            .get("response")
            .and_then(|response| response.get("status"))
            .and_then(serde_json::Value::as_str),
        Some("error" | "failed")
    )
}

fn parse_review_payload(raw: &str) -> Result<ClaudeReviewPayload> {
    let trimmed = raw.trim();

    if let Ok(payload) = serde_json::from_str(trimmed) {
        return Ok(payload);
    }

    let start = trimmed
        .find('{')
        .context("missing JSON object in review output")?;
    let end = trimmed
        .rfind('}')
        .context("missing JSON object terminator in review output")?;

    serde_json::from_str(&trimmed[start..=end]).context("failed to parse review output as JSON")
}

#[async_trait]
impl Worker for ClaudeWorker {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(
        &self,
        phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult> {
        let start = Instant::now();
        let args = self.build_execute_args(phase, prompt, worktree_path);
        let (raw, exit_code) = self.run_command(&args, Some(worktree_path)).await?;
        let mut result = self.parse_execute_output(&raw)?;
        result.duration = start.elapsed();
        result.exit_code = exit_code.max(result.exit_code);
        Ok(result)
    }

    async fn review(
        &self,
        phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult> {
        let start = Instant::now();
        let args = self.build_review_args(phase, diff, candidate_label);
        let (raw, exit_code) = self.run_command(&args, None).await?;
        let mut result = self.parse_review_output(&raw)?;
        result.duration = start.elapsed();
        if exit_code != 0 {
            anyhow::bail!("Claude review command failed with exit code {exit_code}");
        }
        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct CodexWorker {
    command: String,
    flags: Vec<String>,
    name: String,
    model: Option<String>,
    reasoning_effort: String,
    sandbox: Option<String>,
    approval_policy: Option<String>,
}

#[derive(Debug)]
struct ParsedCodexOutput {
    combined_output: String,
    is_error: bool,
    token_usage: Option<TokenUsage>,
}

impl CodexWorker {
    pub fn new(config: &WorkerConfig) -> Self {
        let name = std::path::Path::new(&config.cmd)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("codex")
            .to_string();

        Self {
            command: config.cmd.clone(),
            flags: config.flags.clone(),
            name,
            model: config.model.clone(),
            reasoning_effort: config
                .reasoning_effort
                .clone()
                .unwrap_or_else(|| "xhigh".to_string()),
            sandbox: config.sandbox.clone(),
            approval_policy: config.approval_policy.clone(),
        }
    }

    pub fn build_execute_args(
        &self,
        _phase: &Phase,
        prompt: &str,
        _worktree_path: &Path,
    ) -> Vec<String> {
        let mut args = self.base_args();
        args.push(prompt.to_string());
        args
    }

    pub fn parse_execute_output(&self, raw: &str) -> Result<WorkerResult> {
        let parsed = self.parse_json_output(raw)?;
        let signals = extract_signals(raw);

        Ok(WorkerResult {
            worker_name: self.name.clone(),
            diff_text: parsed.combined_output.clone(),
            exit_code: if parsed.is_error { 1 } else { 0 },
            duration: Duration::ZERO,
            token_usage: parsed.token_usage,
            raw_output: parsed.combined_output.clone(),
            signals: serialize_signals(&signals),
        })
    }

    pub fn build_review_args(&self, phase: &Phase, diff: &str, label: &str) -> Vec<String> {
        let mut args = self.base_args();
        args.push(review_prompt(phase, diff, label));
        args
    }

    pub fn parse_review_output(&self, raw: &str) -> Result<ReviewResult> {
        let payload = parse_review_payload(raw).or_else(|_| {
            let parsed = self.parse_json_output(raw)?;
            parse_review_payload(&parsed.combined_output)
        })?;

        let verdict = match payload.verdict.as_str() {
            "approve" => ReviewVerdict::Approve,
            "request_changes" => ReviewVerdict::RequestChanges(
                payload
                    .request_changes_reason
                    .unwrap_or_else(|| "changes requested".to_string()),
            ),
            "abstain" => ReviewVerdict::Abstain,
            other => anyhow::bail!("unsupported Codex review verdict `{other}`"),
        };

        Ok(ReviewResult {
            reviewer_name: self.name.clone(),
            candidate_label: payload.candidate_label,
            verdict,
            scores: payload.scores,
            issues: payload.issues,
            summary: payload.summary,
            duration: Duration::ZERO,
        })
    }

    fn base_args(&self) -> Vec<String> {
        let mut args = self.flags.clone();

        if !args.iter().any(|arg| arg == "-q") {
            args.push("-q".to_string());
        }

        if !args.iter().any(|arg| arg == "--json") {
            args.push("--json".to_string());
        }

        if let Some(model) = &self.model
            && !has_any_flag(&args, &["--model", "-m"])
        {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        if !self.reasoning_effort.is_empty() && !has_any_flag(&args, &["--reasoning-effort"]) {
            args.push("--reasoning-effort".to_string());
            args.push(self.reasoning_effort.clone());
        }

        if let Some(sandbox) = &self.sandbox
            && !has_any_flag(&args, &["--sandbox", "-s"])
        {
            args.push("--sandbox".to_string());
            args.push(sandbox.clone());
        }

        if let Some(approval_policy) = &self.approval_policy
            && !has_any_flag(&args, &["--ask-for-approval", "-a"])
        {
            args.push("--ask-for-approval".to_string());
            args.push(approval_policy.clone());
        }

        args
    }

    fn parse_json_output(&self, raw: &str) -> Result<ParsedCodexOutput> {
        let mut accumulated_text = Vec::new();
        let mut final_output: Option<String> = None;
        let mut is_error = false;
        let mut token_usage: Option<TokenUsage> = None;

        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(parsed) => {
                    if let Some(usage) = extract_token_usage(&parsed) {
                        token_usage = Some(usage);
                    }

                    if codex_event_is_error(&parsed) {
                        is_error = true;
                    }

                    if let Some(text) = extract_codex_text(&parsed) {
                        if codex_event_is_final(&parsed) {
                            final_output = Some(text);
                        } else {
                            accumulated_text.push(text);
                        }
                    }
                }
                Err(_) => accumulated_text.push(line.to_string()),
            }
        }

        let combined_output = final_output
            .unwrap_or_else(|| accumulated_text.join("\n"))
            .trim_end_matches('\n')
            .to_string();

        Ok(ParsedCodexOutput {
            combined_output,
            is_error,
            token_usage,
        })
    }

    async fn run_command(
        &self,
        args: &[String],
        current_dir: Option<&Path>,
    ) -> Result<(String, i32)> {
        let mut cmd = Command::new(&self.command);
        cmd.args(args);
        if let Some(dir) = current_dir {
            cmd.current_dir(dir);
        }

        let output = cmd
            .output()
            .await
            .with_context(|| format!("failed to execute `{}`", self.command))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let raw = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
            (false, true) => stdout,
            (true, false) => stderr,
            (false, false) => format!("{stdout}\n{stderr}"),
            (true, true) => String::new(),
        };
        let exit_code = output
            .status
            .code()
            .unwrap_or_else(|| if output.status.success() { 0 } else { 1 });

        Ok((raw, exit_code))
    }
}

#[async_trait]
impl Worker for CodexWorker {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(
        &self,
        phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult> {
        let start = Instant::now();
        let args = self.build_execute_args(phase, prompt, worktree_path);
        let (raw, exit_code) = self.run_command(&args, Some(worktree_path)).await?;
        let mut result = self.parse_execute_output(&raw)?;
        result.duration = start.elapsed();
        result.exit_code = exit_code.max(result.exit_code);
        Ok(result)
    }

    async fn review(
        &self,
        phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult> {
        let start = Instant::now();
        let args = self.build_review_args(phase, diff, candidate_label);
        let (raw, exit_code) = self.run_command(&args, None).await?;
        let mut result = self.parse_review_output(&raw)?;
        result.duration = start.elapsed();
        if exit_code != 0 {
            anyhow::bail!("Codex review command failed with exit code {exit_code}");
        }
        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct MockWorker {
    name: String,
    execute_result: Option<WorkerResult>,
    execute_results: Arc<Mutex<VecDeque<WorkerResult>>>,
    execute_error: Option<String>,
    execute_delay: Duration,
    review_result: Option<ReviewResult>,
    review_error: Option<String>,
    review_delay: Duration,
    execute_count: Arc<AtomicU32>,
    review_count: Arc<AtomicU32>,
    execute_prompts: Arc<Mutex<Vec<String>>>,
    last_execute_worktree_path: Arc<Mutex<Option<String>>>,
    last_review_diff: Arc<Mutex<Option<String>>>,
    last_review_candidate_label: Arc<Mutex<Option<String>>>,
}

impl MockWorker {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            execute_result: None,
            execute_results: Arc::new(Mutex::new(VecDeque::new())),
            execute_error: None,
            execute_delay: Duration::ZERO,
            review_result: None,
            review_error: None,
            review_delay: Duration::ZERO,
            execute_count: Arc::new(AtomicU32::new(0)),
            review_count: Arc::new(AtomicU32::new(0)),
            execute_prompts: Arc::new(Mutex::new(Vec::new())),
            last_execute_worktree_path: Arc::new(Mutex::new(None)),
            last_review_diff: Arc::new(Mutex::new(None)),
            last_review_candidate_label: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_execute_result(mut self, result: WorkerResult) -> Self {
        self.execute_result = Some(result);
        self.execute_results.lock().expect("mutex poisoned").clear();
        self
    }

    pub fn with_execute_error(mut self, error: impl Into<String>) -> Self {
        self.execute_error = Some(error.into());
        self
    }

    pub fn with_execute_delay(mut self, delay: Duration) -> Self {
        self.execute_delay = delay;
        self
    }

    pub fn with_execute_results(mut self, results: Vec<WorkerResult>) -> Self {
        self.execute_result = None;
        *self.execute_results.lock().expect("mutex poisoned") = VecDeque::from(results);
        self
    }

    pub fn with_review_result(mut self, result: ReviewResult) -> Self {
        self.review_result = Some(result);
        self
    }

    pub fn with_review_error(mut self, error: impl Into<String>) -> Self {
        self.review_error = Some(error.into());
        self
    }

    pub fn with_review_delay(mut self, delay: Duration) -> Self {
        self.review_delay = delay;
        self
    }

    pub fn execute_count(&self) -> u32 {
        self.execute_count.load(Ordering::Relaxed)
    }

    pub fn execute_prompts(&self) -> Vec<String> {
        self.execute_prompts.lock().expect("mutex poisoned").clone()
    }

    pub fn last_execute_worktree_path(&self) -> Option<String> {
        self.last_execute_worktree_path
            .lock()
            .expect("mutex poisoned")
            .clone()
    }

    pub fn review_count(&self) -> u32 {
        self.review_count.load(Ordering::Relaxed)
    }

    pub fn last_review_diff(&self) -> Option<String> {
        self.last_review_diff
            .lock()
            .expect("mutex poisoned")
            .clone()
    }

    pub fn last_review_candidate_label(&self) -> Option<String> {
        self.last_review_candidate_label
            .lock()
            .expect("mutex poisoned")
            .clone()
    }

    fn default_execute_result(&self) -> WorkerResult {
        WorkerResult {
            worker_name: self.name.clone(),
            diff_text: String::new(),
            exit_code: 0,
            duration: Duration::ZERO,
            token_usage: None,
            raw_output: String::new(),
            signals: Vec::new(),
        }
    }

    fn default_review_result(&self, candidate_label: &str) -> ReviewResult {
        ReviewResult {
            reviewer_name: self.name.clone(),
            candidate_label: candidate_label.to_string(),
            verdict: ReviewVerdict::Approve,
            scores: ReviewScores {
                correctness: 1.0,
                completeness: 1.0,
                style: 1.0,
                performance: 1.0,
                overall: 1.0,
            },
            issues: Vec::new(),
            summary: String::new(),
            duration: Duration::ZERO,
        }
    }
}

#[async_trait]
impl Worker for MockWorker {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(
        &self,
        _phase: &Phase,
        prompt: &str,
        worktree_path: &Path,
    ) -> Result<WorkerResult> {
        self.execute_count.fetch_add(1, Ordering::Relaxed);
        self.execute_prompts
            .lock()
            .expect("mutex poisoned")
            .push(prompt.to_string());
        *self
            .last_execute_worktree_path
            .lock()
            .expect("mutex poisoned") = Some(worktree_path.display().to_string());

        if !self.execute_delay.is_zero() {
            sleep(self.execute_delay).await;
        }

        if let Some(error) = &self.execute_error {
            anyhow::bail!("{error}");
        }

        if let Some(result) = self
            .execute_results
            .lock()
            .expect("mutex poisoned")
            .pop_front()
        {
            return Ok(result);
        }

        Ok(self
            .execute_result
            .clone()
            .unwrap_or_else(|| self.default_execute_result()))
    }

    async fn review(
        &self,
        _phase: &Phase,
        diff: &str,
        candidate_label: &str,
    ) -> Result<ReviewResult> {
        self.review_count.fetch_add(1, Ordering::Relaxed);

        *self.last_review_diff.lock().expect("mutex poisoned") = Some(diff.to_string());
        *self
            .last_review_candidate_label
            .lock()
            .expect("mutex poisoned") = Some(candidate_label.to_string());

        if !self.review_delay.is_zero() {
            sleep(self.review_delay).await;
        }

        if let Some(error) = &self.review_error {
            anyhow::bail!("{error}");
        }

        Ok(self
            .review_result
            .clone()
            .unwrap_or_else(|| self.default_review_result(candidate_label)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::config::WorkerConfig;
    use crate::council::types::{ReviewResult, ReviewScores, ReviewVerdict, WorkerResult};
    use crate::phase::Phase;
    use std::path::Path;
    use std::time::Duration;

    fn test_phase() -> Phase {
        serde_json::from_str(
            r#"{
                "number": "01",
                "name": "Test Phase",
                "promise": "DONE",
                "budget": 1
            }"#,
        )
        .expect("test phase should deserialize")
    }

    #[tokio::test]
    async fn test_mock_worker_name() {
        let worker = MockWorker::new("test");

        assert_eq!(worker.name(), "test");
    }

    #[tokio::test]
    async fn test_mock_worker_default_execute_returns_ok() {
        let worker = MockWorker::new("test");

        let result = worker
            .execute(
                &test_phase(),
                "implement something",
                Path::new("/tmp/worktree"),
            )
            .await
            .expect("default execute should succeed");

        assert_eq!(result.worker_name, "test");
        assert_eq!(result.diff_text, "");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration, Duration::from_secs(0));
        assert!(result.token_usage.is_none());
        assert_eq!(result.raw_output, "");
        assert!(result.signals.is_empty());
    }

    #[tokio::test]
    async fn test_mock_worker_default_review_returns_approve() {
        let worker = MockWorker::new("test");

        let result = worker
            .review(
                &test_phase(),
                "diff --git a/src/lib.rs b/src/lib.rs",
                "Candidate Alpha",
            )
            .await
            .expect("default review should succeed");

        assert_eq!(result.reviewer_name, "test");
        assert_eq!(result.candidate_label, "Candidate Alpha");
        assert!(matches!(result.verdict, ReviewVerdict::Approve));
        assert!(result.issues.is_empty());
        assert_eq!(result.summary, "");
        assert_eq!(result.duration, Duration::from_secs(0));
    }

    #[tokio::test]
    async fn test_mock_worker_custom_execute_result() {
        let expected = WorkerResult {
            worker_name: "custom-worker".to_string(),
            diff_text: "diff --git a/src/main.rs b/src/main.rs".to_string(),
            exit_code: 7,
            duration: Duration::from_secs(3),
            token_usage: None,
            raw_output: "custom output".to_string(),
            signals: vec!["<progress>done</progress>".to_string()],
        };
        let worker = MockWorker::new("test").with_execute_result(expected.clone());

        let result = worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("custom execute should succeed");

        assert_eq!(result.worker_name, expected.worker_name);
        assert_eq!(result.diff_text, expected.diff_text);
        assert_eq!(result.exit_code, expected.exit_code);
        assert_eq!(result.duration, expected.duration);
        assert_eq!(result.raw_output, expected.raw_output);
        assert_eq!(result.signals, expected.signals);
    }

    #[tokio::test]
    async fn test_mock_worker_custom_review_result() {
        let expected = ReviewResult {
            reviewer_name: "custom-reviewer".to_string(),
            candidate_label: "Candidate Beta".to_string(),
            verdict: ReviewVerdict::RequestChanges("needs tests".to_string()),
            scores: ReviewScores {
                correctness: 0.4,
                completeness: 0.5,
                style: 0.6,
                performance: 0.7,
                overall: 0.55,
            },
            issues: vec!["missing coverage".to_string()],
            summary: "follow up required".to_string(),
            duration: Duration::from_secs(4),
        };
        let worker = MockWorker::new("test").with_review_result(expected.clone());

        let result = worker
            .review(&test_phase(), "diff", "Candidate Alpha")
            .await
            .expect("custom review should succeed");

        assert_eq!(result.reviewer_name, expected.reviewer_name);
        assert_eq!(result.candidate_label, expected.candidate_label);
        match result.verdict {
            ReviewVerdict::RequestChanges(message) => assert_eq!(message, "needs tests"),
            _ => panic!("expected RequestChanges verdict"),
        }
        assert_eq!(result.summary, expected.summary);
        assert_eq!(result.issues, expected.issues);
        assert_eq!(result.duration, expected.duration);
    }

    #[tokio::test]
    async fn test_mock_worker_execute_count_increments() {
        let worker = MockWorker::new("test");

        worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("first execute should succeed");
        worker
            .execute(&test_phase(), "prompt", Path::new("/tmp/worktree"))
            .await
            .expect("second execute should succeed");

        assert_eq!(worker.execute_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_worker_review_count_increments() {
        let worker = MockWorker::new("test");

        worker
            .review(&test_phase(), "diff", "Candidate Alpha")
            .await
            .expect("first review should succeed");
        worker
            .review(&test_phase(), "diff", "Candidate Beta")
            .await
            .expect("second review should succeed");

        assert_eq!(worker.review_count(), 2);
    }

    #[test]
    fn test_mock_worker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MockWorker>();
    }

    #[test]
    fn test_worker_trait_object_safety() {
        fn _accepts_dyn(_w: &dyn Worker) {}

        let worker = MockWorker::new("test");
        _accepts_dyn(&worker);
    }

    mod test_claude_worker {
        use super::*;

        fn worker_config() -> WorkerConfig {
            WorkerConfig {
                cmd: "claude".to_string(),
                role: "worker".to_string(),
                flags: vec![],
                model: None,
                reasoning_effort: None,
                sandbox: None,
                approval_policy: None,
            }
        }

        fn claude_worker() -> ClaudeWorker {
            ClaudeWorker::new(&worker_config())
        }

        #[test]
        fn test_claude_worker_name() {
            let worker = claude_worker();

            assert_eq!(worker.name(), "claude");
        }

        #[test]
        fn test_claude_worker_build_execute_args_includes_cwd() {
            let worker = claude_worker();
            let args = worker.build_execute_args(
                &test_phase(),
                "implement feature",
                Path::new("/tmp/council-worktree"),
            );

            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--cwd", "/tmp/council-worktree"])
            );
        }

        #[test]
        fn test_claude_worker_build_execute_args_includes_output_format() {
            let worker = claude_worker();
            let args =
                worker.build_execute_args(&test_phase(), "implement feature", Path::new("/tmp"));

            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--output-format", "stream-json"])
            );
        }

        #[test]
        fn test_claude_worker_build_execute_args_includes_skip_permissions() {
            let worker = claude_worker();
            let args =
                worker.build_execute_args(&test_phase(), "implement feature", Path::new("/tmp"));

            assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
        }

        #[test]
        fn test_claude_worker_parse_execute_output_valid() {
            let worker = claude_worker();
            let raw = concat!(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"diff --git a/src/lib.rs b/src/lib.rs"}]},"session_id":"session-123"}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"@@ -1 +1 @@\n-old\n+new\n<progress>75</progress>"}]},"session_id":"session-123"}"#,
                "\n",
                r#"{"type":"result","subtype":"success","result":"diff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n<promise>DONE</promise>","is_error":false,"usage":{"input_tokens":1500,"output_tokens":800}}"#
            );

            let result = worker
                .parse_execute_output(raw)
                .expect("valid execute output should parse");

            assert_eq!(result.worker_name, "claude");
            assert_eq!(result.exit_code, 0);
            assert!(
                result
                    .diff_text
                    .contains("diff --git a/src/lib.rs b/src/lib.rs")
            );
            assert!(result.raw_output.contains("<promise>DONE</promise>"));
            assert_eq!(
                result.token_usage.as_ref().map(|u| u.input_tokens),
                Some(1500)
            );
            assert_eq!(
                result.token_usage.as_ref().map(|u| u.output_tokens),
                Some(800)
            );
            assert!(
                result
                    .signals
                    .iter()
                    .any(|signal| signal.contains("<progress>75</progress>"))
            );
        }

        #[test]
        fn test_claude_worker_parse_execute_output_empty() {
            let worker = claude_worker();

            let result = worker
                .parse_execute_output("")
                .expect("empty execute output should parse");

            assert_eq!(result.worker_name, "claude");
            assert_eq!(result.diff_text, "");
            assert_eq!(result.raw_output, "");
            assert_eq!(result.exit_code, 0);
            assert!(result.signals.is_empty());
        }

        #[test]
        fn test_claude_worker_parse_execute_output_error_exit() {
            let worker = claude_worker();
            let raw = concat!(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"permission denied"}]},"session_id":"session-456"}"#,
                "\n",
                r#"{"type":"result","subtype":"error","result":"permission denied","is_error":true}"#
            );

            let result = worker
                .parse_execute_output(raw)
                .expect("error execute output should still parse");

            assert_eq!(result.exit_code, 1);
            assert_eq!(result.diff_text, "permission denied");
            assert_eq!(result.raw_output, "permission denied");
        }

        #[test]
        fn test_claude_worker_build_review_args() {
            let worker = claude_worker();
            let args = worker.build_review_args(
                &test_phase(),
                "diff --git a/src/lib.rs b/src/lib.rs",
                "Candidate Alpha",
            );

            assert!(args.contains(&"--print".to_string()));
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--output-format", "stream-json"])
            );
            let prompt = args.last().expect("review prompt should be last argument");
            assert!(prompt.contains("Candidate Alpha"));
            assert!(prompt.contains("Return JSON only"));
        }

        #[test]
        fn test_claude_worker_parse_review_output_approve() {
            let worker = claude_worker();
            let raw = concat!(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"{\"candidate_label\":\"Candidate Alpha\",\"verdict\":\"approve\",\"request_changes_reason\":null,\"scores\":{\"correctness\":0.95,\"completeness\":0.9,\"style\":0.85,\"performance\":0.8,\"overall\":0.875},\"issues\":[],\"summary\":\"Looks good.\"}"}]},"session_id":"review-session"}"#,
                "\n",
                r#"{"type":"result","subtype":"success","result":"{\"candidate_label\":\"Candidate Alpha\",\"verdict\":\"approve\",\"request_changes_reason\":null,\"scores\":{\"correctness\":0.95,\"completeness\":0.9,\"style\":0.85,\"performance\":0.8,\"overall\":0.875},\"issues\":[],\"summary\":\"Looks good.\"}","is_error":false}"#
            );

            let result = worker
                .parse_review_output(raw)
                .expect("approve review output should parse");

            assert_eq!(result.reviewer_name, "claude");
            assert_eq!(result.candidate_label, "Candidate Alpha");
            assert!(matches!(result.verdict, ReviewVerdict::Approve));
            assert_eq!(result.summary, "Looks good.");
            assert!(result.issues.is_empty());
        }

        #[test]
        fn test_claude_worker_parse_review_output_request_changes() {
            let worker = claude_worker();
            let raw = concat!(
                r#"{"type":"result","subtype":"success","result":"{\"candidate_label\":\"Candidate Beta\",\"verdict\":\"request_changes\",\"request_changes_reason\":\"Missing tests\",\"scores\":{\"correctness\":0.6,\"completeness\":0.5,\"style\":0.75,\"performance\":0.7,\"overall\":0.6375},\"issues\":[\"src/lib.rs:42 add coverage\"],\"summary\":\"Needs tests.\"}","is_error":false}"#
            );

            let result = worker
                .parse_review_output(raw)
                .expect("request changes review output should parse");

            assert_eq!(result.candidate_label, "Candidate Beta");
            match result.verdict {
                ReviewVerdict::RequestChanges(reason) => assert_eq!(reason, "Missing tests"),
                other => panic!("expected request changes verdict, got {other:?}"),
            }
            assert_eq!(
                result.issues,
                vec!["src/lib.rs:42 add coverage".to_string()]
            );
            assert_eq!(result.summary, "Needs tests.");
        }

        #[test]
        fn test_claude_worker_parse_review_output_malformed_json() {
            let worker = claude_worker();
            let raw =
                r#"{"type":"result","subtype":"success","result":"{not json","is_error":false}"#;

            let error = worker
                .parse_review_output(raw)
                .expect_err("malformed review output should error");

            assert!(
                error.to_string().contains("review"),
                "expected review parse context, got: {error:#}"
            );
        }
    }

    mod test_codex_worker {
        use super::*;

        fn worker_config() -> WorkerConfig {
            WorkerConfig {
                cmd: "codex".to_string(),
                role: "worker".to_string(),
                flags: vec![],
                model: Some("gpt-5.4".to_string()),
                reasoning_effort: Some("xhigh".to_string()),
                sandbox: Some("workspace-write".to_string()),
                approval_policy: None,
            }
        }

        fn codex_worker() -> CodexWorker {
            CodexWorker::new(&worker_config())
        }

        #[test]
        fn test_codex_worker_name() {
            let worker = codex_worker();

            assert_eq!(worker.name(), "codex");
        }

        #[test]
        fn test_codex_worker_build_execute_args_includes_model() {
            let worker = codex_worker();
            let args = worker.build_execute_args(
                &test_phase(),
                "implement feature",
                Path::new("/tmp/council-worktree"),
            );

            assert!(args.windows(2).any(|pair| pair == ["--model", "gpt-5.4"]));
        }

        #[test]
        fn test_codex_worker_build_execute_args_includes_reasoning_effort() {
            let worker = codex_worker();
            let args =
                worker.build_execute_args(&test_phase(), "implement feature", Path::new("/tmp"));

            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--reasoning-effort", "xhigh"])
            );
        }

        #[test]
        fn test_codex_worker_build_execute_args_includes_sandbox() {
            let worker = codex_worker();
            let args =
                worker.build_execute_args(&test_phase(), "implement feature", Path::new("/tmp"));

            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--sandbox", "workspace-write"])
            );
        }

        #[test]
        fn test_codex_worker_build_execute_args_defaults_without_model() {
            let mut config = worker_config();
            config.model = None;
            let worker = CodexWorker::new(&config);
            let args =
                worker.build_execute_args(&test_phase(), "implement feature", Path::new("/tmp"));

            assert!(!args.iter().any(|arg| arg == "--model"));
        }

        #[test]
        fn test_codex_worker_parse_execute_output_valid() {
            let worker = codex_worker();
            let raw = concat!(
                r#"{"type":"response.output_text.delta","delta":"diff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n<progress>75</progress>"}"#,
                "\n",
                r#"{"type":"response.completed","response":{"output_text":"diff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n<promise>DONE</promise>","usage":{"input_tokens":321,"output_tokens":123}}}"#
            );

            let result = worker
                .parse_execute_output(raw)
                .expect("valid execute output should parse");

            assert_eq!(result.worker_name, "codex");
            assert_eq!(result.exit_code, 0);
            assert!(
                result
                    .diff_text
                    .contains("diff --git a/src/lib.rs b/src/lib.rs")
            );
            assert!(result.raw_output.contains("<promise>DONE</promise>"));
            assert_eq!(
                result.token_usage.as_ref().map(|usage| usage.input_tokens),
                Some(321)
            );
            assert_eq!(
                result.token_usage.as_ref().map(|usage| usage.output_tokens),
                Some(123)
            );
            assert!(
                result
                    .signals
                    .iter()
                    .any(|signal| signal.contains("<progress>75</progress>"))
            );
        }

        #[test]
        fn test_codex_worker_parse_execute_output_empty() {
            let worker = codex_worker();

            let result = worker
                .parse_execute_output("")
                .expect("empty execute output should parse");

            assert_eq!(result.worker_name, "codex");
            assert_eq!(result.diff_text, "");
            assert_eq!(result.raw_output, "");
            assert_eq!(result.exit_code, 0);
            assert!(result.signals.is_empty());
        }

        #[test]
        fn test_codex_worker_build_review_args() {
            let worker = codex_worker();
            let args = worker.build_review_args(
                &test_phase(),
                "diff --git a/src/lib.rs b/src/lib.rs",
                "Candidate Alpha",
            );

            assert!(args.contains(&"-q".to_string()));
            assert!(args.contains(&"--json".to_string()));
            let prompt = args.last().expect("review prompt should be last argument");
            assert!(prompt.contains("Candidate Alpha"));
            assert!(prompt.contains("Return JSON only"));
        }

        #[test]
        fn test_codex_worker_parse_review_output_approve() {
            let worker = codex_worker();
            let raw = concat!(
                r#"{"type":"response.completed","response":{"output_text":"{\"candidate_label\":\"Candidate Alpha\",\"verdict\":\"approve\",\"request_changes_reason\":null,\"scores\":{\"correctness\":0.95,\"completeness\":0.9,\"style\":0.85,\"performance\":0.8,\"overall\":0.875},\"issues\":[],\"summary\":\"Looks good.\"}"}}"#
            );

            let result = worker
                .parse_review_output(raw)
                .expect("approve review output should parse");

            assert_eq!(result.reviewer_name, "codex");
            assert_eq!(result.candidate_label, "Candidate Alpha");
            assert!(matches!(result.verdict, ReviewVerdict::Approve));
            assert!(result.issues.is_empty());
            assert_eq!(result.summary, "Looks good.");
        }

        #[test]
        fn test_codex_worker_parse_review_output_request_changes() {
            let worker = codex_worker();
            let raw = concat!(
                r#"{"type":"response.completed","response":{"output_text":"{\"candidate_label\":\"Candidate Beta\",\"verdict\":\"request_changes\",\"request_changes_reason\":\"Missing tests\",\"scores\":{\"correctness\":0.6,\"completeness\":0.5,\"style\":0.75,\"performance\":0.7,\"overall\":0.6375},\"issues\":[\"src/lib.rs:42 add coverage\"],\"summary\":\"Needs tests.\"}"}}"#
            );

            let result = worker
                .parse_review_output(raw)
                .expect("request changes review output should parse");

            assert_eq!(result.candidate_label, "Candidate Beta");
            match result.verdict {
                ReviewVerdict::RequestChanges(reason) => assert_eq!(reason, "Missing tests"),
                other => panic!("expected request changes verdict, got {other:?}"),
            }
            assert_eq!(
                result.issues,
                vec!["src/lib.rs:42 add coverage".to_string()]
            );
            assert_eq!(result.summary, "Needs tests.");
        }
    }
}
