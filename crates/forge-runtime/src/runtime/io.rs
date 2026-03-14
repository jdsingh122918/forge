//! Shared child-process and container-output helpers for runtime backends.

use std::fmt;
use std::process::ExitStatus;
use std::sync::Mutex as StdMutex;

use anyhow::{Context, Result};
use bollard::container::LogOutput;
use bollard::errors::Error as BollardError;
use chrono::{DateTime, Utc};
use forge_common::events::TaskOutput;
use forge_common::ids::{AgentId, RunId, TaskNodeId};
use forge_common::output_parser::{
    ParsedOutputEvent, ParsedOutputMode, ParsedOutputState, parse_output_line,
};
use forge_common::runtime::{AgentOutputMode, AgentStatus};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::StreamExt;

#[derive(Debug, Clone)]
pub struct RuntimeOutputEnvelope {
    pub run_id: RunId,
    pub task_id: TaskNodeId,
    pub agent_id: AgentId,
    pub output: TaskOutput,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeOutputSink {
    tx: Option<mpsc::UnboundedSender<RuntimeOutputEnvelope>>,
}

impl RuntimeOutputSink {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn channel() -> (Self, mpsc::UnboundedReceiver<RuntimeOutputEnvelope>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx: Some(tx) }, rx)
    }

    fn emit(&self, envelope: RuntimeOutputEnvelope) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(envelope);
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeOutputEmitter {
    run_id: RunId,
    task_id: TaskNodeId,
    agent_id: AgentId,
    output_mode: ParsedOutputMode,
    sink: RuntimeOutputSink,
}

impl RuntimeOutputEmitter {
    pub fn new(
        run_id: RunId,
        task_id: TaskNodeId,
        agent_id: AgentId,
        output_mode: AgentOutputMode,
        sink: RuntimeOutputSink,
    ) -> Self {
        Self {
            run_id,
            task_id,
            agent_id,
            output_mode: match output_mode {
                AgentOutputMode::PlainText => ParsedOutputMode::Text,
                AgentOutputMode::StreamJson => ParsedOutputMode::StreamJson,
            },
            sink,
        }
    }

    pub fn emit_stdout_line(&self, state: &mut ParsedOutputState, line: String) {
        for event in parse_output_line(state, self.output_mode, line) {
            if let ParsedOutputEvent::TaskOutput(output) = event {
                self.emit(output);
            }
        }
    }

    pub fn emit_stderr_line(&self, line: String) {
        self.emit(TaskOutput::Stderr(line));
    }

    fn emit(&self, output: TaskOutput) {
        self.sink.emit(RuntimeOutputEnvelope {
            run_id: self.run_id.clone(),
            task_id: self.task_id.clone(),
            agent_id: self.agent_id.clone(),
            output,
            timestamp: Utc::now(),
        });
    }
}

pub struct TrackedChild {
    child: Mutex<Child>,
    kill_reason: StdMutex<Option<String>>,
}

impl TrackedChild {
    pub async fn new(
        mut child: Child,
        stdin_payload: Vec<u8>,
        emitter: RuntimeOutputEmitter,
    ) -> Result<Self> {
        if let Some(stdout) = child.stdout.take() {
            spawn_stdout_task(stdout, emitter.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_stderr_task(stderr, emitter.clone());
        }

        if let Some(mut stdin) = child.stdin.take() {
            if !stdin_payload.is_empty() {
                stdin
                    .write_all(&stdin_payload)
                    .await
                    .context("failed to write prepared stdin payload")?;
            }
            stdin
                .shutdown()
                .await
                .context("failed to close prepared stdin payload")?;
        }

        Ok(Self {
            child: Mutex::new(child),
            kill_reason: StdMutex::new(None),
        })
    }

    pub async fn try_wait(&self) -> std::io::Result<Option<ExitStatus>> {
        let mut child = self.child.lock().await;
        child.try_wait()
    }

    pub async fn wait(&self) -> std::io::Result<ExitStatus> {
        let mut child = self.child.lock().await;
        child.wait().await
    }

    pub async fn force_kill(&self) -> std::io::Result<()> {
        let mut child = self.child.lock().await;
        child.start_kill()
    }

    pub fn remember_kill_reason(&self, reason: impl Into<String>) {
        let mut kill_reason = self.kill_reason.lock().expect("kill reason mutex poisoned");
        *kill_reason = Some(reason.into());
    }

    pub fn kill_reason(&self) -> Option<String> {
        self.kill_reason
            .lock()
            .expect("kill reason mutex poisoned")
            .clone()
    }
}

impl fmt::Debug for TrackedChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedChild").finish_non_exhaustive()
    }
}

pub fn spawn_docker_attach_task<O, I>(
    mut output: O,
    mut input: Option<I>,
    stdin_payload: Vec<u8>,
    emitter: RuntimeOutputEmitter,
) where
    O: tokio_stream::Stream<Item = std::result::Result<LogOutput, BollardError>>
        + Unpin
        + Send
        + 'static,
    I: AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if let Some(mut input) = input.take() {
            if !stdin_payload.is_empty()
                && let Err(error) = input.write_all(&stdin_payload).await
            {
                emitter.emit_stderr_line(format!(
                    "[forge-runtime::docker] failed to write prepared stdin payload: {error}"
                ));
                return;
            }
            if let Err(error) = input.shutdown().await {
                emitter.emit_stderr_line(format!(
                    "[forge-runtime::docker] failed to close prepared stdin payload: {error}"
                ));
            }
        }

        let mut stdout_state = ParsedOutputState::default();
        let mut stdout_buffer = String::new();
        let mut stderr_buffer = String::new();
        while let Some(item) = output.next().await {
            match item {
                Ok(LogOutput::StdOut { message }) | Ok(LogOutput::Console { message }) => {
                    emit_chunk_lines(&mut stdout_buffer, &message, |line| {
                        emitter.emit_stdout_line(&mut stdout_state, line)
                    });
                }
                Ok(LogOutput::StdErr { message }) => {
                    emit_chunk_lines(&mut stderr_buffer, &message, |line| {
                        emitter.emit_stderr_line(line)
                    });
                }
                Ok(LogOutput::StdIn { .. }) => {}
                Err(error) => {
                    emitter.emit_stderr_line(format!(
                        "[forge-runtime::docker] attach stream ended with error: {error}"
                    ));
                    break;
                }
            }
        }

        flush_partial_line(&mut stdout_buffer, |line| {
            emitter.emit_stdout_line(&mut stdout_state, line)
        });
        flush_partial_line(&mut stderr_buffer, |line| emitter.emit_stderr_line(line));
    });
}

pub fn status_from_exit_status(
    exit_status: ExitStatus,
    kill_reason: Option<String>,
) -> AgentStatus {
    if exit_status.success() {
        return AgentStatus::Exited {
            exit_code: exit_status.code().unwrap_or_default(),
        };
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(reason) = kill_reason {
            return AgentStatus::Killed { reason };
        }

        if let Some(signal) = exit_status.signal() {
            return AgentStatus::Crashed {
                exit_code: exit_status.code(),
                error: format!("terminated by signal {signal}"),
            };
        }
    }

    AgentStatus::Crashed {
        exit_code: exit_status.code(),
        error: "process exited unsuccessfully".to_string(),
    }
}

fn spawn_stdout_task<R>(reader: R, emitter: RuntimeOutputEmitter)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        let mut state = ParsedOutputState::default();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => emitter.emit_stdout_line(&mut state, line),
                Ok(None) => break,
                Err(error) => {
                    emitter.emit_stderr_line(format!(
                        "[forge-runtime::io] failed reading stdout: {error}"
                    ));
                    break;
                }
            }
        }
    });
}

fn spawn_stderr_task<R>(reader: R, emitter: RuntimeOutputEmitter)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => emitter.emit_stderr_line(line),
                Ok(None) => break,
                Err(error) => {
                    emitter.emit_stderr_line(format!(
                        "[forge-runtime::io] failed reading stderr: {error}"
                    ));
                    break;
                }
            }
        }
    });
}

fn emit_chunk_lines(buffer: &mut String, chunk: &[u8], mut emit: impl FnMut(String)) {
    buffer.push_str(&String::from_utf8_lossy(chunk));

    while let Some(newline) = buffer.find('\n') {
        let mut line = buffer.drain(..=newline).collect::<String>();
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        emit(line);
    }
}

fn flush_partial_line(buffer: &mut String, mut emit: impl FnMut(String)) {
    if !buffer.is_empty() {
        emit(std::mem::take(buffer));
    }
}
