//! Shared execution facade for direct subprocess execution today and daemon
//! delegation later.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::TaskOutput;
use crate::ids::{RunId, TaskNodeId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionId(String);

impl ExecutionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionBackendKind {
    Claude,
    Codex,
    ForgeSubcommand,
    Custom(String),
}

impl ExecutionBackendKind {
    pub fn label(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::ForgeSubcommand => "forge-subcommand",
            Self::Custom(label) => label.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionOutputMode {
    Text,
    StreamJson,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionResumeMode {
    Fresh,
    ResumeSession(String),
    ContinueSession(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskNodeId>,
    pub backend: ExecutionBackendKind,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_dir: PathBuf,
    pub prompt: Option<String>,
    pub stdin: Vec<u8>,
    pub output_mode: ExecutionOutputMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub resume_mode: ExecutionResumeMode,
}

#[derive(Debug)]
pub struct ExecutionHandle {
    pub id: ExecutionId,
    pub events: mpsc::Receiver<ExecutionEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionEvent {
    Output(TaskOutput),
    AssistantText(String),
    Thinking(String),
    ToolCall { name: String, input: Value },
    SessionCaptured(String),
    FinalPayload(Value),
    Exit { code: i32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExecutionOutcome {
    Completed {
        exit_code: i32,
        stdout: String,
        stderr: String,
        session_id: Option<String>,
        final_payload: Option<Value>,
    },
    Failed {
        exit_code: Option<i32>,
        error: String,
        stdout: String,
        stderr: String,
        session_id: Option<String>,
        final_payload: Option<Value>,
    },
    Killed {
        reason: Option<String>,
        stdout: String,
        stderr: String,
        session_id: Option<String>,
        final_payload: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBackendHealth {
    pub backend: String,
    pub available: bool,
    pub version: Option<String>,
    pub capabilities: Vec<String>,
    pub details: Option<String>,
}

#[async_trait]
pub trait ExecutionFacade: Send + Sync {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionHandle>;
    async fn wait(&self, execution_id: &ExecutionId) -> Result<ExecutionOutcome>;
    async fn kill(&self, execution_id: &ExecutionId, reason: Option<&str>) -> Result<()>;
    async fn health_check(&self) -> Result<ExecutionBackendHealth>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct MockFacade {
        outcome: ExecutionOutcome,
        health: ExecutionBackendHealth,
    }

    #[async_trait]
    impl ExecutionFacade for MockFacade {
        async fn execute(&self, _request: ExecutionRequest) -> Result<ExecutionHandle> {
            let (tx, rx) = mpsc::channel(8);
            tokio::spawn(async move {
                let _ = tx
                    .send(ExecutionEvent::SessionCaptured("session-123".to_string()))
                    .await;
                let _ = tx
                    .send(ExecutionEvent::AssistantText("working".to_string()))
                    .await;
                let _ = tx
                    .send(ExecutionEvent::ToolCall {
                        name: "Read".to_string(),
                        input: serde_json::json!({ "file_path": "src/main.rs" }),
                    })
                    .await;
            });

            Ok(ExecutionHandle {
                id: ExecutionId::generate(),
                events: rx,
            })
        }

        async fn wait(&self, _execution_id: &ExecutionId) -> Result<ExecutionOutcome> {
            Ok(self.outcome.clone())
        }

        async fn kill(&self, _execution_id: &ExecutionId, _reason: Option<&str>) -> Result<()> {
            Err(anyhow!("mock kill not implemented"))
        }

        async fn health_check(&self) -> Result<ExecutionBackendHealth> {
            Ok(self.health.clone())
        }
    }

    #[tokio::test]
    async fn mock_facade_streams_rich_events() {
        let facade: Arc<dyn ExecutionFacade> = Arc::new(MockFacade {
            outcome: ExecutionOutcome::Completed {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                session_id: Some("session-123".to_string()),
                final_payload: None,
            },
            health: ExecutionBackendHealth {
                backend: "mock".to_string(),
                available: true,
                version: Some("1".to_string()),
                capabilities: vec!["events".to_string()],
                details: None,
            },
        });

        let request = ExecutionRequest {
            run_id: None,
            task_id: None,
            backend: ExecutionBackendKind::Codex,
            program: "codex".to_string(),
            args: vec!["--version".to_string()],
            env: BTreeMap::new(),
            working_dir: PathBuf::from("."),
            prompt: None,
            stdin: Vec::new(),
            output_mode: ExecutionOutputMode::Text,
            allowed_tools: vec![],
            disallowed_tools: vec![],
            resume_mode: ExecutionResumeMode::Fresh,
        };

        let mut handle = facade.execute(request).await.unwrap();
        let mut seen = Vec::new();
        while let Some(event) = handle.events.recv().await {
            seen.push(event);
            if seen.len() == 3 {
                break;
            }
        }

        assert!(matches!(seen[0], ExecutionEvent::SessionCaptured(_)));
        assert!(matches!(seen[1], ExecutionEvent::AssistantText(_)));
        assert!(matches!(seen[2], ExecutionEvent::ToolCall { .. }));
    }

    #[tokio::test]
    async fn failed_outcomes_are_representable() {
        let facade = MockFacade {
            outcome: ExecutionOutcome::Failed {
                exit_code: Some(1),
                error: "boom".to_string(),
                stdout: String::new(),
                stderr: "boom".to_string(),
                session_id: None,
                final_payload: Some(serde_json::json!({ "status": "error" })),
            },
            health: ExecutionBackendHealth {
                backend: "mock".to_string(),
                available: true,
                version: None,
                capabilities: vec![],
                details: None,
            },
        };

        let outcome = facade.wait(&ExecutionId::generate()).await.unwrap();
        assert!(matches!(
            outcome,
            ExecutionOutcome::Failed {
                exit_code: Some(1),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn structured_health_is_returned() {
        let facade = MockFacade {
            outcome: ExecutionOutcome::Completed {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                session_id: None,
                final_payload: None,
            },
            health: ExecutionBackendHealth {
                backend: "direct".to_string(),
                available: true,
                version: Some("0.1.0".to_string()),
                capabilities: vec!["text".to_string(), "stream-json".to_string()],
                details: Some("subprocess-backed".to_string()),
            },
        };

        let health = facade.health_check().await.unwrap();
        assert_eq!(health.backend, "direct");
        assert!(health.available);
        assert_eq!(health.capabilities.len(), 2);
    }

    #[tokio::test]
    async fn execution_id_display_is_stable() {
        let id = ExecutionId::new("exec-1");
        let display = Arc::new(Mutex::new(String::new()));
        *display.lock().await = id.to_string();
        assert_eq!(display.lock().await.as_str(), "exec-1");
    }
}
