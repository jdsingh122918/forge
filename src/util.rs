//! Shared utility functions for the Forge crate.

/// Extract a JSON object from text that may contain other content.
/// Uses brace-counting to find the outermost JSON object.
pub fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0;
    let mut end = start;

    for (i, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth == 0 && end > start {
        Some(text[start..end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_object_simple() {
        let text = r#"{"key": "value"}"#;
        assert_eq!(extract_json_object(text), Some(r#"{"key": "value"}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_with_prefix() {
        let text = r#"Here is the JSON: {"key": "value"}"#;
        assert_eq!(extract_json_object(text), Some(r#"{"key": "value"}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_with_suffix() {
        let text = r#"{"key": "value"} and some more text"#;
        assert_eq!(extract_json_object(text), Some(r#"{"key": "value"}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_nested() {
        let text = r#"{"outer": {"inner": "value"}}"#;
        assert_eq!(extract_json_object(text), Some(r#"{"outer": {"inner": "value"}}"#.to_string()));
    }

    #[test]
    fn test_extract_json_object_no_json() {
        let text = "No JSON here";
        assert_eq!(extract_json_object(text), None);
    }

    #[test]
    fn test_extract_json_object_unclosed() {
        let text = r#"{"key": "value""#;
        assert_eq!(extract_json_object(text), None);
    }
}
