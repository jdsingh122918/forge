//! Shared child-process helpers for runtime backends.

use std::fmt;
use std::process::ExitStatus;
use std::sync::Mutex as StdMutex;

use anyhow::{Context, Result};
use forge_common::runtime::AgentStatus;
use tokio::io::{AsyncRead, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::Mutex;

pub struct TrackedChild {
    child: Mutex<Child>,
    kill_reason: StdMutex<Option<String>>,
}

impl TrackedChild {
    pub async fn new(mut child: Child, stdin_payload: Vec<u8>) -> Result<Self> {
        if let Some(stdout) = child.stdout.take() {
            spawn_drain_task(stdout, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_drain_task(stderr, "stderr");
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

fn spawn_drain_task<R>(mut reader: R, stream_name: &'static str)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let _ = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await;
        tracing::debug!(stream = stream_name, "runtime child stream drained");
    });
}
