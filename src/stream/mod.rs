use serde::Deserialize;
use serde_json::Value;

/// Events from Claude CLI's stream-json output format
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "assistant")]
    Assistant {
        message: AssistantMessage,
        #[serde(default)]
        session_id: String,
    },

    #[serde(rename = "user")]
    User {
        #[serde(default)]
        tool_use_result: Option<ToolUseResult>,
    },

    #[serde(rename = "result")]
    Result {
        subtype: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        is_error: bool,
    },

    #[serde(rename = "system")]
    System { subtype: String },
}

#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "tool_use")]
    ToolUse {
        name: String,
        input: Value,
        #[serde(default)]
        id: String,
    },

    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Deserialize)]
pub struct ToolUseResult {
    #[serde(default)]
    pub file: Option<FileInfo>,
}

#[derive(Debug, Deserialize)]
pub struct FileInfo {
    #[serde(rename = "filePath")]
    pub file_path: String,
}

/// Extract a human-readable description from a tool use event
pub fn describe_tool_use(name: &str, input: &Value) -> String {
    match name {
        "Read" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(shorten_path)
                .unwrap_or_else(|| "file".to_string());
            format!("Reading: {}", path)
        }
        "Write" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(shorten_path)
                .unwrap_or_else(|| "file".to_string());
            format!("Creating: {}", path)
        }
        "Edit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(shorten_path)
                .unwrap_or_else(|| "file".to_string());
            format!("Editing: {}", path)
        }
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| truncate_str(s, 40))
                .unwrap_or_else(|| "command".to_string());
            format!("Running: {}", cmd)
        }
        "Glob" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
            format!("Searching: {}", pattern)
        }
        "Grep" => {
            let pattern = input
                .get("pattern")
                .and_then(|v| v.as_str())
                .map(|s| truncate_str(s, 30))
                .unwrap_or_else(|| "pattern".to_string());
            format!("Grep: {}", pattern)
        }
        "Task" => {
            let desc = input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("subagent");
            format!("Agent: {}", desc)
        }
        _ => name.to_string(),
    }
}

/// Get an emoji for a tool
pub fn tool_emoji(name: &str) -> &'static str {
    match name {
        "Read" => "\u{1F4D6}",        // 📖
        "Write" => "\u{1F4DD}",       // 📝
        "Edit" => "\u{270F}\u{FE0F}", // ✏️
        "Bash" => "\u{2699}\u{FE0F}", // ⚙️
        "Glob" => "\u{1F50D}",        // 🔍
        "Grep" => "\u{1F50E}",        // 🔎
        "Task" => "\u{1F916}",        // 🤖
        _ => "\u{1F527}",             // 🔧
    }
}

/// Shorten a file path to just the last 2 components
fn shorten_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 2 {
        path.to_string()
    } else {
        parts[parts.len() - 2..].join("/")
    }
}

/// Truncate a string with ellipsis
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Truncate thinking text to a reasonable snippet
pub fn truncate_thinking(text: &str, max_len: usize) -> String {
    // Take first line or first max_len chars
    let first_line = text.lines().next().unwrap_or(text);
    truncate_str(first_line.trim(), max_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assistant_tool_use() {
        let json = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/foo/bar.rs"},"id":"123"}]},"session_id":"abc"}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();

        if let StreamEvent::Assistant { message, .. } = event {
            assert_eq!(message.content.len(), 1);
            if let ContentBlock::ToolUse { name, input, .. } = &message.content[0] {
                assert_eq!(name, "Read");
                assert_eq!(
                    input.get("file_path").unwrap().as_str().unwrap(),
                    "/foo/bar.rs"
                );
            } else {
                panic!("Expected ToolUse");
            }
        } else {
            panic!("Expected Assistant event");
        }
    }

    #[test]
    fn test_parse_assistant_text() {
        let json = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello world"}]},"session_id":"abc"}"#;
        let event: StreamEvent = serde_json::from_str(json).unwrap();

        if let StreamEvent::Assistant { message, .. } = event {
            if let ContentBlock::Text { text } = &message.content[0] {
                assert_eq!(text, "Hello world");
            } else {
                panic!("Expected Text");
            }
        } else {
            panic!("Expected Assistant event");
        }
    }

    #[test]
    fn test_describe_tool_use() {
        let input = serde_json::json!({"file_path": "/Users/foo/project/src/main.rs"});
        assert_eq!(describe_tool_use("Read", &input), "Reading: src/main.rs");

        let input = serde_json::json!({"command": "cargo test --release"});
        assert_eq!(
            describe_tool_use("Bash", &input),
            "Running: cargo test --release"
        );
    }
}
