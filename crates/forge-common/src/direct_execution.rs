//! Direct subprocess-backed implementation of the shared execution facade.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, Mutex};

use crate::facade::{
    ExecutionBackendHealth, ExecutionEvent, ExecutionFacade, ExecutionHandle, ExecutionId,
    ExecutionOutcome, ExecutionOutputMode, ExecutionRequest,
};
use crate::TaskOutput;

#[derive(Clone, Default)]
pub struct DirectExecutionFacade {
    executions: Arc<Mutex<HashMap<ExecutionId, ActiveExecution>>>,
}

#[derive(Clone)]
struct ActiveExecution {
    kill_tx: mpsc::UnboundedSender<Option<String>>,
    outcome_rx: watch::Receiver<Option<ExecutionOutcome>>,
}

#[derive(Default)]
struct CollectedOutput {
    stdout: String,
    stderr: String,
    session_id: Option<String>,
    final_payload: Option<Value>,
}

impl DirectExecutionFacade {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExecutionFacade for DirectExecutionFacade {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionHandle> {
        let execution_id = ExecutionId::generate();
        let (events_tx, events_rx) = mpsc::channel(128);
        let (kill_tx, mut kill_rx) = mpsc::unbounded_channel();
        let (outcome_tx, outcome_rx) = watch::channel(None);

        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if !request.stdin.is_empty() {
            command.stdin(Stdio::piped());
        }

        if !request.env.is_empty() {
            command.envs(&request.env);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to spawn {}", request.program))?;

        if !request.stdin.is_empty() {
            if let Some(mut stdin) = child.stdin.take() {
                let input = request.stdin.clone();
                tokio::spawn(async move {
                    let _ = stdin.write_all(&input).await;
                });
            }
        }

        let stdout = child
            .stdout
            .take()
            .context("child stdout was not captured")?;
        let stderr = child
            .stderr
            .take()
            .context("child stderr was not captured")?;

        let collected = Arc::new(Mutex::new(CollectedOutput::default()));
        let stdout_collected = Arc::clone(&collected);
        let stderr_collected = Arc::clone(&collected);
        let stdout_tx = events_tx.clone();
        let stderr_tx = events_tx.clone();
        let output_mode = request.output_mode;

        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await.context("failed reading stdout")? {
                {
                    let mut collected = stdout_collected.lock().await;
                    collected.stdout.push_str(&line);
                    collected.stdout.push('\n');
                }
                handle_stdout_line(&stdout_tx, &stdout_collected, output_mode, line).await?;
            }
            Ok::<(), anyhow::Error>(())
        });

        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Some(line) = lines.next_line().await.context("failed reading stderr")? {
                {
                    let mut collected = stderr_collected.lock().await;
                    collected.stderr.push_str(&line);
                    collected.stderr.push('\n');
                }
                let _ = stderr_tx
                    .send(ExecutionEvent::Output(TaskOutput::Stderr(line)))
                    .await;
            }
            Ok::<(), anyhow::Error>(())
        });

        self.executions.lock().await.insert(
            execution_id.clone(),
            ActiveExecution {
                kill_tx,
                outcome_rx: outcome_rx.clone(),
            },
        );

        tokio::spawn(async move {
            let mut kill_reason: Option<String> = None;
            let status = loop {
                tokio::select! {
                    received = kill_rx.recv() => {
                        match received {
                            Some(reason) => {
                                kill_reason = reason;
                                if let Err(error) = child.start_kill() {
                                    let collected = collected.lock().await;
                                    let _ = outcome_tx.send(Some(ExecutionOutcome::Failed {
                                        exit_code: None,
                                        error: error.to_string(),
                                        stdout: collected.stdout.clone(),
                                        stderr: collected.stderr.clone(),
                                        session_id: collected.session_id.clone(),
                                        final_payload: collected.final_payload.clone(),
                                    }));
                                    return;
                                }
                            }
                            None => break child.wait().await,
                        }
                    }
                    status = child.wait() => break status,
                }
            };

            let stdout_result = stdout_task.await;
            let stderr_result = stderr_task.await;

            let mut collected = collected.lock().await;
            if let Ok(Err(error)) = stdout_result {
                collected.stderr.push_str(&format!("{error:#}\n"));
            }
            if let Ok(Err(error)) = stderr_result {
                collected.stderr.push_str(&format!("{error:#}\n"));
            }

            let outcome = match status {
                Ok(_status) if kill_reason.is_some() => ExecutionOutcome::Killed {
                    reason: kill_reason,
                    stdout: collected.stdout.clone(),
                    stderr: collected.stderr.clone(),
                    session_id: collected.session_id.clone(),
                    final_payload: collected.final_payload.clone(),
                },
                Ok(status) if status.success() => {
                    let exit_code = status.code().unwrap_or(0);
                    let _ = events_tx.send(ExecutionEvent::Exit { code: exit_code }).await;
                    ExecutionOutcome::Completed {
                        exit_code,
                        stdout: collected.stdout.clone(),
                        stderr: collected.stderr.clone(),
                        session_id: collected.session_id.clone(),
                        final_payload: collected.final_payload.clone(),
                    }
                }
                Ok(status) => ExecutionOutcome::Failed {
                    exit_code: status.code(),
                    error: format!("process exited with status {status}"),
                    stdout: collected.stdout.clone(),
                    stderr: collected.stderr.clone(),
                    session_id: collected.session_id.clone(),
                    final_payload: collected.final_payload.clone(),
                },
                Err(error) => ExecutionOutcome::Failed {
                    exit_code: None,
                    error: error.to_string(),
                    stdout: collected.stdout.clone(),
                    stderr: collected.stderr.clone(),
                    session_id: collected.session_id.clone(),
                    final_payload: collected.final_payload.clone(),
                },
            };

            let _ = outcome_tx.send(Some(outcome));
        });

        Ok(ExecutionHandle {
            id: execution_id,
            events: events_rx,
        })
    }

    async fn wait(&self, execution_id: &ExecutionId) -> Result<ExecutionOutcome> {
        let mut receiver = self
            .executions
            .lock()
            .await
            .get(execution_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown execution id: {execution_id}"))?
            .outcome_rx;

        loop {
            if let Some(outcome) = receiver.borrow().clone() {
                return Ok(outcome);
            }
            receiver
                .changed()
                .await
                .map_err(|_| anyhow!("execution channel closed unexpectedly"))?;
        }
    }

    async fn kill(&self, execution_id: &ExecutionId, reason: Option<&str>) -> Result<()> {
        let sender = self
            .executions
            .lock()
            .await
            .get(execution_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown execution id: {execution_id}"))?
            .kill_tx;

        sender
            .send(reason.map(ToOwned::to_owned))
            .map_err(|_| anyhow!("execution already finished"))?;
        Ok(())
    }

    async fn health_check(&self) -> Result<ExecutionBackendHealth> {
        Ok(ExecutionBackendHealth {
            backend: "direct-subprocess".to_string(),
            available: true,
            version: None,
            capabilities: vec![
                "text".to_string(),
                "stream-json".to_string(),
                "kill".to_string(),
            ],
            details: Some("tokio::process-backed execution".to_string()),
        })
    }
}

async fn handle_stdout_line(
    events_tx: &mpsc::Sender<ExecutionEvent>,
    collected: &Arc<Mutex<CollectedOutput>>,
    output_mode: ExecutionOutputMode,
    line: String,
) -> Result<()> {
    match output_mode {
        ExecutionOutputMode::Text => {
            let _ = events_tx
                .send(ExecutionEvent::Output(TaskOutput::Stdout(line)))
                .await;
        }
        ExecutionOutputMode::StreamJson => {
            if let Ok(json) = serde_json::from_str::<Value>(&line) {
                handle_stream_json_event(events_tx, collected, &json).await?;
            } else {
                let _ = events_tx
                    .send(ExecutionEvent::Output(TaskOutput::Stdout(line)))
                    .await;
            }
        }
    }

    Ok(())
}

async fn handle_stream_json_event(
    events_tx: &mpsc::Sender<ExecutionEvent>,
    collected: &Arc<Mutex<CollectedOutput>>,
    json: &Value,
) -> Result<()> {
    if let Some(session_id) = json.get("session_id").and_then(Value::as_str) {
        let mut state = collected.lock().await;
        let should_emit = state.session_id.as_deref() != Some(session_id);
        if should_emit {
            state.session_id = Some(session_id.to_string());
            drop(state);
            let _ = events_tx
                .send(ExecutionEvent::SessionCaptured(session_id.to_string()))
                .await;
        }
    }

    if let Some(usage) = json.get("usage") {
        if let Some(cumulative) = extract_total_tokens(usage) {
            let _ = events_tx
                .send(ExecutionEvent::Output(TaskOutput::TokenUsage {
                    tokens: cumulative,
                    cumulative,
                }))
                .await;
        }
    }

    if let Some(event_type) = json.get("type").and_then(Value::as_str) {
        match event_type {
            "assistant" => emit_assistant_events(events_tx, json).await,
            "content_block_delta" => emit_delta_events(events_tx, json).await,
            "result" | "response.completed" => {
                if let Some(result_text) = json.get("result").and_then(Value::as_str) {
                    emit_text_signals(events_tx, result_text).await;
                }
                if let Some(output_text) = json
                    .get("response")
                    .and_then(|response| response.get("output_text"))
                    .and_then(Value::as_str)
                {
                    emit_text_signals(events_tx, output_text).await;
                }
                {
                    let mut state = collected.lock().await;
                    state.final_payload = Some(json.clone());
                }
                let _ = events_tx
                    .send(ExecutionEvent::FinalPayload(json.clone()))
                    .await;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn emit_assistant_events(events_tx: &mpsc::Sender<ExecutionEvent>, json: &Value) {
    let Some(content) = json
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };

    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let _ = events_tx
                        .send(ExecutionEvent::AssistantText(text.to_string()))
                        .await;
                    emit_text_signals(events_tx, text).await;
                }
            }
            Some("tool_use") => {
                let _ = events_tx
                    .send(ExecutionEvent::ToolCall {
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string(),
                        input: block.get("input").cloned().unwrap_or(Value::Null),
                    })
                    .await;
            }
            _ => {}
        }
    }
}

async fn emit_delta_events(events_tx: &mpsc::Sender<ExecutionEvent>, json: &Value) {
    if let Some(delta) = json.get("delta") {
        if let Some(thinking) = delta
            .get("thinking")
            .or_else(|| delta.get("thinking_delta"))
            .and_then(Value::as_str)
        {
            let _ = events_tx
                .send(ExecutionEvent::Thinking(thinking.to_string()))
                .await;
        }
    }
}

async fn emit_text_signals(events_tx: &mpsc::Sender<ExecutionEvent>, text: &str) {
    if text.contains("<promise>DONE</promise>") {
        let _ = events_tx
            .send(ExecutionEvent::Output(TaskOutput::PromiseDone))
            .await;
    }

    for signal_name in ["progress", "blocker", "pivot"] {
        let open = format!("<{signal_name}>");
        let close = format!("</{signal_name}>");
        if let Some(start) = text.find(&open) {
            let content_start = start + open.len();
            if let Some(relative_end) = text[content_start..].find(&close) {
                let content_end = content_start + relative_end;
                let _ = events_tx
                    .send(ExecutionEvent::Output(TaskOutput::Signal {
                        kind: signal_name.to_string(),
                        content: text[content_start..content_end].to_string(),
                    }))
                    .await;
            }
        }
    }
}

fn extract_total_tokens(usage: &Value) -> Option<u64> {
    usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let input = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            let output = usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cache_read = usage
                .get("cache_read_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let cache_write = usage
                .get("cache_write_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let total = input + output + cache_read + cache_write;
            (total > 0).then_some(total)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tokio::time::{sleep, Duration};

    fn text_request(args: Vec<&str>) -> ExecutionRequest {
        ExecutionRequest {
            run_id: None,
            task_id: None,
            backend: crate::ExecutionBackendKind::Custom("test".to_string()),
            program: "sh".to_string(),
            args: args.into_iter().map(str::to_string).collect(),
            env: BTreeMap::new(),
            working_dir: PathBuf::from("."),
            prompt: None,
            stdin: Vec::new(),
            output_mode: ExecutionOutputMode::Text,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            resume_mode: crate::ExecutionResumeMode::Fresh,
        }
    }

    #[tokio::test]
    async fn executes_text_mode_commands() {
        let facade = DirectExecutionFacade::new();
        let request = text_request(vec!["-c", "printf 'hello\\n'"]);
        let handle = facade.execute(request).await.unwrap();
        let outcome = facade.wait(&handle.id).await.unwrap();

        match outcome {
            ExecutionOutcome::Completed { stdout, .. } => {
                assert!(stdout.contains("hello"));
            }
            other => panic!("expected completed outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parses_stream_json_output() {
        let facade = DirectExecutionFacade::new();
        let request = ExecutionRequest {
            output_mode: ExecutionOutputMode::StreamJson,
            ..text_request(vec![
                "-c",
                "printf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello <progress>50%</progress>\"}]},\"session_id\":\"session-1\"}' '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"done <promise>DONE</promise>\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}'"
            ])
        };

        let mut handle = facade.execute(request).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = handle.events.recv().await {
            events.push(event);
            if events.iter().any(|event| matches!(event, ExecutionEvent::FinalPayload(_))) {
                break;
            }
        }

        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::SessionCaptured(session) if session == "session-1")));
        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::AssistantText(text) if text.contains("hello"))));
        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::Output(TaskOutput::Signal { kind, content }) if kind == "progress" && content == "50%")));
        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::Output(TaskOutput::PromiseDone))));
    }

    #[tokio::test]
    async fn kill_returns_killed_outcome() {
        let facade = DirectExecutionFacade::new();
        let request = text_request(vec!["-c", "sleep 30"]);
        let handle = facade.execute(request).await.unwrap();

        sleep(Duration::from_millis(50)).await;
        facade.kill(&handle.id, Some("test kill")).await.unwrap();
        let outcome = facade.wait(&handle.id).await.unwrap();

        assert!(matches!(
            outcome,
            ExecutionOutcome::Killed {
                reason: Some(reason),
                ..
            } if reason == "test kill"
        ));
    }
}
