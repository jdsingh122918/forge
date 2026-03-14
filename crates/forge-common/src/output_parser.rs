//! Shared parser for line-oriented agent output.

use serde_json::Value;

use crate::events::TaskOutput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedOutputMode {
    Text,
    StreamJson,
}

#[derive(Debug, Clone)]
pub enum ParsedOutputEvent {
    TaskOutput(TaskOutput),
    AssistantText(String),
    Thinking(String),
    ToolCall { name: String, input: Value },
    SessionCaptured(String),
    FinalPayload(Value),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedOutputState {
    pub session_id: Option<String>,
    pub final_payload: Option<Value>,
}

pub fn parse_output_line(
    state: &mut ParsedOutputState,
    mode: ParsedOutputMode,
    line: String,
) -> Vec<ParsedOutputEvent> {
    match mode {
        ParsedOutputMode::Text => vec![ParsedOutputEvent::TaskOutput(TaskOutput::Stdout(line))],
        ParsedOutputMode::StreamJson => serde_json::from_str::<Value>(&line)
            .map(|json| parse_stream_json_event(state, &json))
            .unwrap_or_else(|_| vec![ParsedOutputEvent::TaskOutput(TaskOutput::Stdout(line))]),
    }
}

fn parse_stream_json_event(state: &mut ParsedOutputState, json: &Value) -> Vec<ParsedOutputEvent> {
    let mut events = Vec::new();

    if let Some(session_id) = json.get("session_id").and_then(Value::as_str)
        && state.session_id.as_deref() != Some(session_id)
    {
        state.session_id = Some(session_id.to_string());
        events.push(ParsedOutputEvent::SessionCaptured(session_id.to_string()));
    }

    if let Some(usage) = json.get("usage")
        && let Some(cumulative) = extract_total_tokens(usage)
    {
        events.push(ParsedOutputEvent::TaskOutput(TaskOutput::TokenUsage {
            tokens: cumulative,
            cumulative,
        }));
    }

    if let Some(event_type) = json.get("type").and_then(Value::as_str) {
        match event_type {
            "assistant" => emit_assistant_events(&mut events, json),
            "content_block_delta" => emit_delta_events(&mut events, json),
            "result" | "response.completed" => {
                if let Some(result_text) = json.get("result").and_then(Value::as_str) {
                    emit_text_signals(&mut events, result_text);
                }
                if let Some(output_text) = json
                    .get("response")
                    .and_then(|response| response.get("output_text"))
                    .and_then(Value::as_str)
                {
                    emit_text_signals(&mut events, output_text);
                }
                state.final_payload = Some(json.clone());
                events.push(ParsedOutputEvent::FinalPayload(json.clone()));
            }
            _ => {}
        }
    }

    events
}

fn emit_assistant_events(events: &mut Vec<ParsedOutputEvent>, json: &Value) {
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
                    events.push(ParsedOutputEvent::AssistantText(text.to_string()));
                    emit_text_signals(events, text);
                }
            }
            Some("tool_use") => {
                events.push(ParsedOutputEvent::ToolCall {
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string(),
                    input: block.get("input").cloned().unwrap_or(Value::Null),
                });
            }
            _ => {}
        }
    }
}

fn emit_delta_events(events: &mut Vec<ParsedOutputEvent>, json: &Value) {
    if let Some(delta) = json.get("delta")
        && let Some(thinking) = delta
            .get("thinking")
            .or_else(|| delta.get("thinking_delta"))
            .and_then(Value::as_str)
    {
        events.push(ParsedOutputEvent::Thinking(thinking.to_string()));
    }
}

fn emit_text_signals(events: &mut Vec<ParsedOutputEvent>, text: &str) {
    if text.contains("<promise>DONE</promise>") {
        events.push(ParsedOutputEvent::TaskOutput(TaskOutput::PromiseDone));
    }

    for signal_name in ["progress", "blocker", "pivot"] {
        let open = format!("<{signal_name}>");
        let close = format!("</{signal_name}>");
        if let Some(start) = text.find(&open) {
            let content_start = start + open.len();
            if let Some(relative_end) = text[content_start..].find(&close) {
                let content_end = content_start + relative_end;
                events.push(ParsedOutputEvent::TaskOutput(TaskOutput::Signal {
                    kind: signal_name.to_string(),
                    content: text[content_start..content_end].to_string(),
                }));
            }
        }
    }
}

fn extract_total_tokens(usage: &Value) -> Option<u64> {
    usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let input = usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
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

    #[test]
    fn parses_stream_json_into_shared_output_events() {
        let mut state = ParsedOutputState::default();
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello <progress>50%</progress>"}]},"session_id":"session-1"}"#.to_string();

        let events = parse_output_line(&mut state, ParsedOutputMode::StreamJson, line);

        assert!(events.iter().any(|event| matches!(
            event,
            ParsedOutputEvent::SessionCaptured(session_id) if session_id == "session-1"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ParsedOutputEvent::AssistantText(text) if text.contains("hello")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ParsedOutputEvent::TaskOutput(TaskOutput::Signal { kind, content })
                if kind == "progress" && content == "50%"
        )));
    }
}
