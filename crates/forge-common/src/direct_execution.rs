//! Direct subprocess-backed implementation of the shared execution facade.

use std::collections::HashMap;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, watch};

use crate::TaskOutput;
use crate::facade::{
    ExecutionBackendHealth, ExecutionEvent, ExecutionFacade, ExecutionHandle, ExecutionId,
    ExecutionOutcome, ExecutionOutputMode, ExecutionRequest,
};
use crate::output_parser::{
    ParsedOutputEvent, ParsedOutputMode, ParsedOutputState, parse_output_line,
};

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
    event_channel_closed: bool,
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
        let collected = Arc::new(Mutex::new(CollectedOutput::default()));

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
                let stdin_collected = Arc::clone(&collected);
                tokio::spawn(async move {
                    if let Err(error) = stdin.write_all(&input).await {
                        let mut state = stdin_collected.lock().await;
                        append_internal_error(
                            &mut state,
                            &format!("failed writing child stdin: {error}"),
                        );
                        return;
                    }

                    if let Err(error) = stdin.shutdown().await {
                        let mut state = stdin_collected.lock().await;
                        append_internal_error(
                            &mut state,
                            &format!("failed closing child stdin: {error}"),
                        );
                    }
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
                send_event(
                    &stderr_tx,
                    &stderr_collected,
                    ExecutionEvent::Output(TaskOutput::Stderr(line)),
                    "stderr output",
                )
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
            let (status, kill_reason, mut internal_errors) =
                supervise_child(&mut child, &mut kill_rx).await;

            let stdout_result = stdout_task.await;
            let stderr_result = stderr_task.await;
            let success_exit_code = status
                .as_ref()
                .ok()
                .filter(|status| status.success() && kill_reason.is_none())
                .map(|status| status.code().unwrap_or(0));

            let mut state = collected.lock().await;
            collect_reader_result("stdout", stdout_result, &mut state, &mut internal_errors);
            collect_reader_result("stderr", stderr_result, &mut state, &mut internal_errors);
            drop(state);

            if let Some(exit_code) = success_exit_code {
                send_event(
                    &events_tx,
                    &collected,
                    ExecutionEvent::Exit { code: exit_code },
                    "process exit",
                )
                .await;
            }

            let state = collected.lock().await;

            let outcome = if !internal_errors.is_empty() {
                ExecutionOutcome::Failed {
                    exit_code: status.as_ref().ok().and_then(ExitStatus::code),
                    error: internal_errors.join("; "),
                    stdout: state.stdout.clone(),
                    stderr: state.stderr.clone(),
                    session_id: state.session_id.clone(),
                    final_payload: state.final_payload.clone(),
                }
            } else {
                match status {
                    Ok(_status) if kill_reason.is_some() => ExecutionOutcome::Killed {
                        reason: kill_reason,
                        stdout: state.stdout.clone(),
                        stderr: state.stderr.clone(),
                        session_id: state.session_id.clone(),
                        final_payload: state.final_payload.clone(),
                    },
                    Ok(status) if status.success() => ExecutionOutcome::Completed {
                        exit_code: status.code().unwrap_or(0),
                        stdout: state.stdout.clone(),
                        stderr: state.stderr.clone(),
                        session_id: state.session_id.clone(),
                        final_payload: state.final_payload.clone(),
                    },
                    Ok(status) => ExecutionOutcome::Failed {
                        exit_code: status.code(),
                        error: format!("process exited with status {status}"),
                        stdout: state.stdout.clone(),
                        stderr: state.stderr.clone(),
                        session_id: state.session_id.clone(),
                        final_payload: state.final_payload.clone(),
                    },
                    Err(error) => ExecutionOutcome::Failed {
                        exit_code: None,
                        error: error.to_string(),
                        stdout: state.stdout.clone(),
                        stderr: state.stderr.clone(),
                        session_id: state.session_id.clone(),
                        final_payload: state.final_payload.clone(),
                    },
                }
            };
            drop(state);

            if outcome_tx.send(Some(outcome)).is_err() {
                let mut state = collected.lock().await;
                append_internal_error(
                    &mut state,
                    "execution outcome receiver dropped before the result was published",
                );
            }
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
    let mode = match output_mode {
        ExecutionOutputMode::Text => ParsedOutputMode::Text,
        ExecutionOutputMode::StreamJson => ParsedOutputMode::StreamJson,
    };
    let parsed = {
        let mut state = collected.lock().await;
        let mut parser_state = ParsedOutputState {
            session_id: state.session_id.clone(),
            final_payload: state.final_payload.clone(),
        };
        let parsed = parse_output_line(&mut parser_state, mode, line);
        state.session_id = parser_state.session_id;
        state.final_payload = parser_state.final_payload;
        parsed
    };

    emit_parsed_events(events_tx, collected, parsed).await;

    Ok(())
}

async fn emit_parsed_events(
    events_tx: &mpsc::Sender<ExecutionEvent>,
    collected: &Arc<Mutex<CollectedOutput>>,
    events: Vec<ParsedOutputEvent>,
) {
    for event in events {
        match event {
            ParsedOutputEvent::TaskOutput(output) => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::Output(output),
                    "task output",
                )
                .await;
            }
            ParsedOutputEvent::AssistantText(text) => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::AssistantText(text),
                    "assistant text",
                )
                .await;
            }
            ParsedOutputEvent::Thinking(thinking) => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::Thinking(thinking),
                    "thinking delta",
                )
                .await;
            }
            ParsedOutputEvent::ToolCall { name, input } => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::ToolCall { name, input },
                    "tool call",
                )
                .await;
            }
            ParsedOutputEvent::SessionCaptured(session_id) => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::SessionCaptured(session_id),
                    "session capture",
                )
                .await;
            }
            ParsedOutputEvent::FinalPayload(payload) => {
                send_event(
                    events_tx,
                    collected,
                    ExecutionEvent::FinalPayload(payload),
                    "final payload",
                )
                .await;
            }
        }
    }
}

async fn send_event(
    events_tx: &mpsc::Sender<ExecutionEvent>,
    collected: &Arc<Mutex<CollectedOutput>>,
    event: ExecutionEvent,
    context: &str,
) {
    if events_tx.send(event).await.is_err() {
        let mut state = collected.lock().await;
        if !state.event_channel_closed {
            state.event_channel_closed = true;
            append_internal_error(
                &mut state,
                &format!(
                    "execution event receiver dropped while sending {context}; continuing without live events"
                ),
            );
        }
    }
}

async fn supervise_child(
    child: &mut tokio::process::Child,
    kill_rx: &mut mpsc::UnboundedReceiver<Option<String>>,
) -> (std::io::Result<ExitStatus>, Option<String>, Vec<String>) {
    let mut kill_reason = None;
    let mut internal_errors = Vec::new();

    let status = loop {
        tokio::select! {
            received = kill_rx.recv() => {
                match received {
                    Some(reason) => {
                        kill_reason = reason;
                        if let Err(error) = child.start_kill() {
                            internal_errors.push(format!("failed to kill child process: {error}"));
                            break child.wait().await;
                        }
                    }
                    None => break child.wait().await,
                }
            }
            status = child.wait() => break status,
        }
    };

    (status, kill_reason, internal_errors)
}

fn collect_reader_result(
    name: &str,
    result: std::result::Result<Result<()>, tokio::task::JoinError>,
    collected: &mut CollectedOutput,
    internal_errors: &mut Vec<String>,
) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let message = format!("{name} capture failed: {error:#}");
            append_internal_error(collected, &message);
            internal_errors.push(message);
        }
        Err(error) => {
            let message = format!("{name} capture task failed to join: {error}");
            append_internal_error(collected, &message);
            internal_errors.push(message);
        }
    }
}

fn append_internal_error(collected: &mut CollectedOutput, message: &str) {
    collected
        .stderr
        .push_str(&format!("[forge-common::direct_execution] {message}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tokio::time::{Duration, sleep};

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
                "printf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello <progress>50%</progress>\"}]},\"session_id\":\"session-1\"}' '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"done <promise>DONE</promise>\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}'",
            ])
        };

        let mut handle = facade.execute(request).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = handle.events.recv().await {
            events.push(event);
            if events
                .iter()
                .any(|event| matches!(event, ExecutionEvent::FinalPayload(_)))
            {
                break;
            }
        }

        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::SessionCaptured(session) if session == "session-1")));
        assert!(events.iter().any(
            |event| matches!(event, ExecutionEvent::AssistantText(text) if text.contains("hello"))
        ));
        assert!(events.iter().any(|event| matches!(event, ExecutionEvent::Output(TaskOutput::Signal { kind, content }) if kind == "progress" && content == "50%")));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ExecutionEvent::Output(TaskOutput::PromiseDone)))
        );
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

    #[tokio::test]
    async fn dropped_event_receiver_is_recorded_but_does_not_fail_execution() {
        let facade = DirectExecutionFacade::new();
        let request = text_request(vec!["-c", "printf 'hello\\n'"]);
        let handle = facade.execute(request).await.unwrap();
        drop(handle.events);

        let outcome = facade.wait(&handle.id).await.unwrap();

        match outcome {
            ExecutionOutcome::Completed { stdout, stderr, .. } => {
                assert!(stdout.contains("hello"));
                assert!(stderr.contains("execution event receiver dropped"));
            }
            other => panic!("expected completed outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn capture_errors_fail_the_execution() {
        let facade = DirectExecutionFacade::new();
        let request = text_request(vec!["-c", "printf '\\377'"]);
        let handle = facade.execute(request).await.unwrap();
        let outcome = facade.wait(&handle.id).await.unwrap();

        match outcome {
            ExecutionOutcome::Failed { error, stderr, .. } => {
                assert!(error.contains("stdout capture failed"));
                assert!(stderr.contains("stdout capture failed"));
            }
            other => panic!("expected failed outcome, got {other:?}"),
        }
    }
}
