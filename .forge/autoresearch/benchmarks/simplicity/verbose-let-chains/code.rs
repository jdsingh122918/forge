use serde_json::Value;

#[derive(Debug, Clone)]
pub enum StreamJsonEvent {
    Text { text: String },
    Thinking { text: String },
    ToolStart { tool_name: String, tool_id: String, input_summary: String },
    ToolResult { tool_id: String, output_summary: String },
    Error { message: String },
    Done,
}

/// Parse a stream JSON line into a structured event.
///
/// VERBOSE: Uses deeply nested if/if-let chains where let-chains would be clearer.
pub fn parse_stream_json_line(line: &str) -> StreamJsonEvent {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamJsonEvent::Done;
    }

    let json: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return StreamJsonEvent::Text { text: trimmed.to_string() },
    };

    let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

    // VERBOSE: Nested if/if-let — 3 levels of indentation for a simple condition.
    // Could be: if msg_type == "content_block_delta" && let Some(delta) = json.get("delta") {
    if msg_type == "content_block_delta" {
        if let Some(delta) = json.get("delta") {
            if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                    return StreamJsonEvent::Text {
                        text: text.to_string(),
                    };
                }
            }
            if delta.get("type").and_then(|t| t.as_str()) == Some("thinking_delta") {
                if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                    return StreamJsonEvent::Thinking {
                        text: text.to_string(),
                    };
                }
            }
        }
    }

    // VERBOSE: Three levels of nesting for tool_use detection.
    // Could be one flat let-chain condition.
    if msg_type == "content_block_start" {
        if let Some(cb) = json.get("content_block") {
            if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
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
                return StreamJsonEvent::ToolStart {
                    tool_name,
                    tool_id,
                    input_summary: String::new(),
                };
            }
        }
    }

    // VERBOSE: Another nested pattern for assistant message parsing.
    if json.get("role").and_then(|r| r.as_str()) == Some("assistant") {
        if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
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
                    return StreamJsonEvent::ToolStart {
                        tool_name,
                        tool_id,
                        input_summary: String::new(),
                    };
                }
            }
        }
    }

    StreamJsonEvent::Text { text: trimmed.to_string() }
}

/// Extract file change from a tool event.
///
/// VERBOSE: Same deeply nested pattern.
pub fn extract_file_change(event: &StreamJsonEvent) -> Option<String> {
    if let StreamJsonEvent::ToolStart { tool_name, input_summary, .. } = event {
        if tool_name == "Write" || tool_name == "Edit" {
            if !input_summary.is_empty() {
                return Some(input_summary.clone());
            }
        }
    }
    None
}
