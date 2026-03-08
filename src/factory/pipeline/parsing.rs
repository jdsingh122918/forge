use serde::Serialize;
use tokio::sync::broadcast;

use crate::factory::db::DbHandle;
use crate::factory::models::*;
use crate::factory::ws::{WsMessage, broadcast_message};

/// Tracks progress parsed from subprocess stdout lines.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct ProgressInfo {
    #[serde(default)]
    pub phase: Option<i32>,
    #[serde(default)]
    pub phase_count: Option<i32>,
    #[serde(default)]
    pub iteration: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "content_type")]
#[allow(dead_code)]
pub enum StreamJsonEvent {
    Text {
        text: String,
    },
    ToolStart {
        tool_name: String,
        tool_id: String,
        input_summary: String,
    },
    ToolEnd {
        tool_id: String,
        output_preview: String,
    },
    Thinking {
        text: String,
    },
    Skip,
}

pub fn parse_stream_json_line(line: &str) -> StreamJsonEvent {
    // Try to parse as JSON
    let json: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            // Not JSON — treat as plain text
            return StreamJsonEvent::Text {
                text: line.to_string(),
            };
        }
    };

    // Skip metadata-only messages
    let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match msg_type {
        "message_start" | "message_stop" | "message_delta" | "content_block_stop" | "result" => {
            return StreamJsonEvent::Skip;
        }
        _ => {}
    }

    // content_block_delta with text_delta → Text
    if msg_type == "content_block_delta"
        && let Some(delta) = json.get("delta")
    {
        if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta")
            && let Some(text) = delta.get("text").and_then(|t| t.as_str())
        {
            return StreamJsonEvent::Text {
                text: text.to_string(),
            };
        }
        if delta.get("type").and_then(|t| t.as_str()) == Some("thinking_delta")
            && let Some(text) = delta.get("thinking").and_then(|t| t.as_str())
        {
            return StreamJsonEvent::Thinking {
                text: text.to_string(),
            };
        }
    }

    // content_block_start with tool_use → ToolStart
    if msg_type == "content_block_start"
        && let Some(cb) = json.get("content_block")
        && cb.get("type").and_then(|t| t.as_str()) == Some("tool_use")
    {
        let tool_name = cb
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();
        let tool_id = cb
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        let input_summary = extract_tool_input_summary(&tool_name, cb.get("input"));
        return StreamJsonEvent::ToolStart {
            tool_name,
            tool_id,
            input_summary,
        };
    }

    // Legacy format: subtype == "tool_use"
    if json.get("subtype").and_then(|s| s.as_str()) == Some("tool_use") {
        let tool_name = json
            .get("tool_name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();
        let tool_id = json
            .get("tool_use_id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        let input_summary = extract_tool_input_summary(&tool_name, json.get("input"));
        return StreamJsonEvent::ToolStart {
            tool_name,
            tool_id,
            input_summary,
        };
    }

    // Full assistant message with content array containing tool_use
    if json.get("role").and_then(|r| r.as_str()) == Some("assistant")
        && let Some(content) = json.get("content").and_then(|c| c.as_array())
    {
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                let tool_name = block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let tool_id = block
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_summary = extract_tool_input_summary(&tool_name, block.get("input"));
                return StreamJsonEvent::ToolStart {
                    tool_name,
                    tool_id,
                    input_summary,
                };
            }
        }
    }

    StreamJsonEvent::Skip
}

pub fn extract_tool_input_summary(tool_name: &str, input: Option<&serde_json::Value>) -> String {
    let input = match input {
        Some(v) => v,
        None => return String::new(),
    };

    match tool_name {
        "Read" | "read" | "file_read" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
        "Write" | "write" | "file_write" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
        "Edit" | "edit" | "file_edit" => input
            .get("file_path")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string(),
        "Bash" | "bash" | "execute_command" => {
            let cmd = input.get("command").and_then(|c| c.as_str()).unwrap_or("");
            if cmd.len() > 80 {
                format!("{}...", &cmd[..80])
            } else {
                cmd.to_string()
            }
        }
        _ => String::new(),
    }
}

pub fn extract_file_change(
    tool_name: &str,
    input: Option<&serde_json::Value>,
) -> Option<(String, FileAction)> {
    let input = input?;

    match tool_name {
        "Write" | "write" | "file_write" => {
            let path = input.get("file_path").and_then(|p| p.as_str())?;
            Some((path.to_string(), FileAction::Created))
        }
        "Edit" | "edit" | "file_edit" => {
            let path = input.get("file_path").and_then(|p| p.as_str())?;
            Some((path.to_string(), FileAction::Modified))
        }
        _ => None,
    }
}

/// Attempt to parse a line as progress JSON.
/// Accepts lines like: `{"phase": 2, "phase_count": 5, "iteration": 3}`
/// Also handles lines that contain JSON embedded after a prefix (e.g., `[progress] {...}`).
pub(crate) fn try_parse_progress(line: &str) -> Option<ProgressInfo> {
    let trimmed = line.trim();

    // Try direct parse first
    if let Ok(info) = serde_json::from_str::<ProgressInfo>(trimmed)
        && (info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some())
    {
        return Some(info);
    }

    // Try to find JSON object embedded in the line
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let json_str = &trimmed[start..=end];
        if let Ok(info) = serde_json::from_str::<ProgressInfo>(json_str)
            && (info.phase.is_some() || info.phase_count.is_some() || info.iteration.is_some())
        {
            return Some(info);
        }
    }

    None
}

/// Compute a rough progress percentage from the progress info.
pub(crate) fn compute_percent(progress: &ProgressInfo) -> Option<u8> {
    match (progress.phase, progress.phase_count) {
        (Some(phase), Some(total)) if total > 0 => {
            let pct = ((phase as f64 / total as f64) * 100.0).min(100.0) as u8;
            Some(pct)
        }
        _ => None,
    }
}

/// A PhaseEvent from the DAG executor's JSON output.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum PhaseEventJson {
    Started {
        phase: String,
        wave: usize,
    },
    Progress {
        phase: String,
        iteration: u32,
        budget: u32,
        percent: Option<u32>,
    },
    Completed {
        phase: String,
        result: serde_json::Value,
    },
    ReviewStarted {
        phase: String,
    },
    ReviewCompleted {
        phase: String,
        passed: bool,
        findings_count: usize,
    },
    WaveStarted {
        wave: usize,
        phases: Vec<String>,
    },
    WaveCompleted {
        wave: usize,
        success_count: usize,
        failed_count: usize,
    },
    DagCompleted {
        success: bool,
    },
    #[serde(other)]
    Unknown,
}

/// Try to parse a line as a DAG executor PhaseEvent.
/// Returns the event if successfully parsed and actionable.
pub(crate) fn try_parse_phase_event(line: &str) -> Option<PhaseEventJson> {
    let trimmed = line.trim();
    if let Ok(event) = serde_json::from_str::<PhaseEventJson>(trimmed) {
        return match &event {
            PhaseEventJson::Unknown => None,
            _ => Some(event),
        };
    }
    // Try embedded JSON
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let json_str = &trimmed[start..=end];
        if let Ok(event) = serde_json::from_str::<PhaseEventJson>(json_str) {
            return match &event {
                PhaseEventJson::Unknown => None,
                _ => Some(event),
            };
        }
    }
    None
}

/// Process a PhaseEvent and emit corresponding WsMessages + DB updates.
pub(crate) async fn process_phase_event(
    event: &PhaseEventJson,
    run_id: RunId,
    db: &DbHandle,
    tx: &broadcast::Sender<String>,
) {
    match event {
        PhaseEventJson::Started { phase, wave } => {
            if let Err(e) = db
                .upsert_pipeline_phase(
                    run_id,
                    phase,
                    phase,
                    &PhaseStatus::Running,
                    None,
                    None,
                )
                .await
            {
                broadcast_message(
                    tx,
                    &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to upsert pipeline phase (started): {:#}", e),
                    },
                );
            }
            broadcast_message(
                tx,
                &WsMessage::PipelinePhaseStarted {
                    run_id,
                    phase_number: phase.clone(),
                    phase_name: phase.clone(),
                    wave: *wave,
                },
            );
        }
        PhaseEventJson::Progress {
            phase,
            iteration,
            budget,
            percent,
        } => {
            if let Err(e) = db
                .upsert_pipeline_phase(
                    run_id,
                    phase,
                    phase,
                    &PhaseStatus::Running,
                    Some(*iteration as i32),
                    Some(*budget as i32),
                )
                .await
            {
                broadcast_message(
                    tx,
                    &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to upsert pipeline phase (progress): {:#}", e),
                    },
                );
            }
            broadcast_message(
                tx,
                &WsMessage::PipelineProgress {
                    run_id,
                    phase: phase.parse::<i32>().unwrap_or(0),
                    iteration: *iteration as i32,
                    percent: percent.map(|p| p.min(100) as u8),
                },
            );
        }
        PhaseEventJson::Completed { phase, result } => {
            let success = result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let status = if success { PhaseStatus::Completed } else { PhaseStatus::Failed };
            if let Err(e) = db
                .upsert_pipeline_phase(
                    run_id,
                    phase,
                    phase,
                    &status,
                    None,
                    None,
                )
                .await
            {
                broadcast_message(
                    tx,
                    &WsMessage::PipelineError {
                        run_id,
                        message: format!("Failed to upsert pipeline phase (completed): {:#}", e),
                    },
                );
            }
            broadcast_message(
                tx,
                &WsMessage::PipelinePhaseCompleted {
                    run_id,
                    phase_number: phase.clone(),
                    success,
                },
            );
        }
        PhaseEventJson::ReviewStarted { phase } => {
            broadcast_message(
                tx,
                &WsMessage::PipelineReviewStarted {
                    run_id,
                    phase_number: phase.clone(),
                },
            );
        }
        PhaseEventJson::ReviewCompleted {
            phase,
            passed,
            findings_count,
        } => {
            broadcast_message(
                tx,
                &WsMessage::PipelineReviewCompleted {
                    run_id,
                    phase_number: phase.clone(),
                    passed: *passed,
                    findings_count: *findings_count,
                },
            );
        }
        PhaseEventJson::DagCompleted { success: _ } => {
            // Handled by the outer pipeline completion logic
        }
        PhaseEventJson::WaveStarted { .. } | PhaseEventJson::WaveCompleted { .. } => {
            // Wave events are informational, no DB/WS action needed
        }
        PhaseEventJson::Unknown => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_parse_progress_valid_json() {
        let line = r#"{"phase": 2, "phase_count": 5, "iteration": 3}"#;
        let progress = try_parse_progress(line).expect("should parse");
        assert_eq!(progress.phase, Some(2));
        assert_eq!(progress.phase_count, Some(5));
        assert_eq!(progress.iteration, Some(3));
    }

    #[test]
    fn test_try_parse_progress_embedded_json() {
        let line = r#"[progress] {"phase": 1, "phase_count": 3}"#;
        let progress = try_parse_progress(line).expect("should parse embedded");
        assert_eq!(progress.phase, Some(1));
        assert_eq!(progress.phase_count, Some(3));
        assert_eq!(progress.iteration, None);
    }

    #[test]
    fn test_try_parse_progress_no_progress_fields() {
        let line = r#"{"message": "hello"}"#;
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_plain_text() {
        let line = "Just some regular output text";
        assert!(try_parse_progress(line).is_none());
    }

    #[test]
    fn test_try_parse_progress_partial_fields() {
        let line = r#"{"iteration": 7}"#;
        let progress = try_parse_progress(line).expect("should parse partial");
        assert_eq!(progress.phase, None);
        assert_eq!(progress.phase_count, None);
        assert_eq!(progress.iteration, Some(7));
    }

    #[test]
    fn test_compute_percent() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: Some(4),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(50));

        let p = ProgressInfo {
            phase: Some(5),
            phase_count: Some(5),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(100));

        let p = ProgressInfo {
            phase: Some(1),
            phase_count: Some(10),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), Some(10));
    }

    #[test]
    fn test_compute_percent_no_total() {
        let p = ProgressInfo {
            phase: Some(2),
            phase_count: None,
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
    }

    #[test]
    fn test_compute_percent_zero_total() {
        let p = ProgressInfo {
            phase: Some(0),
            phase_count: Some(0),
            iteration: None,
        };
        assert_eq!(compute_percent(&p), None);
    }

    #[test]
    fn test_try_parse_phase_event_started() {
        let line = r#"{"type": "started", "phase": "1", "wave": 0}"#;
        let event = try_parse_phase_event(line).expect("should parse Started event");
        match event {
            PhaseEventJson::Started { phase, wave } => {
                assert_eq!(phase, "1");
                assert_eq!(wave, 0);
            }
            other => panic!("Expected Started, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_completed() {
        let line = r#"{"type": "completed", "phase": "2", "result": {"success": true}}"#;
        let event = try_parse_phase_event(line).expect("should parse Completed event");
        match event {
            PhaseEventJson::Completed { phase, result } => {
                assert_eq!(phase, "2");
                assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(true));
            }
            other => panic!("Expected Completed, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_progress() {
        let line =
            r#"{"type": "progress", "phase": "3", "iteration": 5, "budget": 10, "percent": 50}"#;
        let event = try_parse_phase_event(line).expect("should parse Progress event");
        match event {
            PhaseEventJson::Progress {
                phase,
                iteration,
                budget,
                percent,
            } => {
                assert_eq!(phase, "3");
                assert_eq!(iteration, 5);
                assert_eq!(budget, 10);
                assert_eq!(percent, Some(50));
            }
            other => panic!("Expected Progress, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_embedded_json() {
        let line = r#"[2024-01-15T10:30:00Z] {"type": "progress", "phase": "1", "iteration": 2, "budget": 5}"#;
        let event = try_parse_phase_event(line).expect("should parse embedded JSON");
        match event {
            PhaseEventJson::Progress {
                phase,
                iteration,
                budget,
                ..
            } => {
                assert_eq!(phase, "1");
                assert_eq!(iteration, 2);
                assert_eq!(budget, 5);
            }
            other => panic!("Expected Progress, got {:?}", other),
        }
    }

    #[test]
    fn test_try_parse_phase_event_unknown_returns_none() {
        let line = r#"{"type": "some_unknown_event", "data": 42}"#;
        assert!(try_parse_phase_event(line).is_none());
    }

    #[test]
    fn test_try_parse_phase_event_plain_text_returns_none() {
        let line = "Just some regular log output";
        assert!(try_parse_phase_event(line).is_none());
    }

    #[test]
    fn test_try_parse_phase_event_dag_completed() {
        let line = r#"{"type": "dag_completed", "success": true}"#;
        let event = try_parse_phase_event(line).expect("should parse DagCompleted event");
        match event {
            PhaseEventJson::DagCompleted { success } => {
                assert!(success);
            }
            other => panic!("Expected DagCompleted, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_delta() {
        let line = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello world"}}"#;
        let result = parse_stream_json_line(line);
        assert_eq!(
            result,
            StreamJsonEvent::Text {
                text: "Hello world".to_string()
            }
        );
    }

    #[test]
    fn test_parse_message_start_skipped() {
        let line = r#"{"type":"message_start","message":{"id":"msg_123"}}"#;
        assert_eq!(parse_stream_json_line(line), StreamJsonEvent::Skip);
    }

    #[test]
    fn test_parse_message_stop_skipped() {
        let line = r#"{"type":"message_stop"}"#;
        assert_eq!(parse_stream_json_line(line), StreamJsonEvent::Skip);
    }

    #[test]
    fn test_parse_message_delta_usage_skipped() {
        let line = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":50}}"#;
        assert_eq!(parse_stream_json_line(line), StreamJsonEvent::Skip);
    }

    #[test]
    fn test_parse_content_block_stop_skipped() {
        let line = r#"{"type":"content_block_stop","index":0}"#;
        assert_eq!(parse_stream_json_line(line), StreamJsonEvent::Skip);
    }

    #[test]
    fn test_parse_result_skipped() {
        let line = r#"{"type":"result","subtype":"success"}"#;
        assert_eq!(parse_stream_json_line(line), StreamJsonEvent::Skip);
    }

    #[test]
    fn test_parse_tool_use_content_block_start() {
        let line = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_123","name":"Edit","input":{"file_path":"src/main.rs"}}}"#;
        match parse_stream_json_line(line) {
            StreamJsonEvent::ToolStart {
                tool_name,
                tool_id,
                input_summary,
            } => {
                assert_eq!(tool_name, "Edit");
                assert_eq!(tool_id, "toolu_123");
                assert_eq!(input_summary, "src/main.rs");
            }
            other => panic!("Expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_thinking_delta() {
        let line = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me analyze..."}}"#;
        assert_eq!(
            parse_stream_json_line(line),
            StreamJsonEvent::Thinking {
                text: "Let me analyze...".to_string()
            }
        );
    }

    #[test]
    fn test_parse_assistant_envelope_with_tool_use() {
        let line = r#"{"role":"assistant","content":[{"type":"tool_use","id":"toolu_456","name":"Write","input":{"file_path":"src/new.rs","content":"fn main() {}"}}]}"#;
        match parse_stream_json_line(line) {
            StreamJsonEvent::ToolStart {
                tool_name,
                tool_id,
                input_summary,
            } => {
                assert_eq!(tool_name, "Write");
                assert_eq!(tool_id, "toolu_456");
                assert_eq!(input_summary, "src/new.rs");
            }
            other => panic!("Expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_plain_text_fallback() {
        let line = "This is just plain text output";
        assert_eq!(
            parse_stream_json_line(line),
            StreamJsonEvent::Text {
                text: "This is just plain text output".to_string()
            }
        );
    }

    #[test]
    fn test_parse_subtype_tool_use() {
        let line = r#"{"subtype":"tool_use","tool_name":"Bash","tool_use_id":"tu_789","input":{"command":"cargo test"}}"#;
        match parse_stream_json_line(line) {
            StreamJsonEvent::ToolStart {
                tool_name,
                tool_id,
                input_summary,
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(tool_id, "tu_789");
                assert_eq!(input_summary, "cargo test");
            }
            other => panic!("Expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_tool_input_summary_read() {
        let input = serde_json::json!({"file_path": "src/lib.rs"});
        assert_eq!(
            extract_tool_input_summary("Read", Some(&input)),
            "src/lib.rs"
        );
    }

    #[test]
    fn test_extract_tool_input_summary_bash() {
        let long_cmd = "a".repeat(100);
        let input = serde_json::json!({"command": long_cmd});
        let result = extract_tool_input_summary("Bash", Some(&input));
        assert_eq!(result.len(), 83); // 80 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_extract_tool_input_summary_unknown() {
        let input = serde_json::json!({"something": "else"});
        assert_eq!(extract_tool_input_summary("UnknownTool", Some(&input)), "");
    }

    #[test]
    fn test_extract_file_change_write() {
        let input = serde_json::json!({"file_path": "src/new.rs", "content": "fn main() {}"});
        assert_eq!(
            extract_file_change("Write", Some(&input)),
            Some(("src/new.rs".to_string(), FileAction::Created))
        );
    }

    #[test]
    fn test_extract_file_change_edit() {
        let input = serde_json::json!({"file_path": "src/lib.rs", "old_string": "foo", "new_string": "bar"});
        assert_eq!(
            extract_file_change("Edit", Some(&input)),
            Some(("src/lib.rs".to_string(), FileAction::Modified))
        );
    }

    #[test]
    fn test_extract_file_change_read_returns_none() {
        let input = serde_json::json!({"file_path": "src/lib.rs"});
        assert_eq!(extract_file_change("Read", Some(&input)), None);
    }

    #[test]
    fn test_extract_file_change_none_input() {
        assert_eq!(extract_file_change("Write", None), None);
    }
}
