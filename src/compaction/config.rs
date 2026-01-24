//! Context limit configuration parsing.

use anyhow::{Context, Result};

/// Represents a context limit configuration.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextLimit {
    /// Percentage of model window (e.g., 80%)
    Percentage(f32),
    /// Absolute character count
    Absolute(usize),
}

impl ContextLimit {
    /// Calculate the effective character limit based on the model window size.
    pub fn effective_limit(&self, model_window_chars: usize) -> usize {
        match self {
            ContextLimit::Percentage(pct) => {
                ((model_window_chars as f32) * (*pct / 100.0)) as usize
            }
            ContextLimit::Absolute(chars) => *chars,
        }
    }

    /// Check if this limit is a percentage.
    pub fn is_percentage(&self) -> bool {
        matches!(self, ContextLimit::Percentage(_))
    }
}

impl Default for ContextLimit {
    fn default() -> Self {
        ContextLimit::Percentage(80.0)
    }
}

impl std::fmt::Display for ContextLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextLimit::Percentage(pct) => write!(f, "{}%", pct),
            ContextLimit::Absolute(chars) => write!(f, "{}", chars),
        }
    }
}

/// Parse a context limit string into a ContextLimit.
///
/// Accepts:
/// - Percentage format: "80%", "60%", etc.
/// - Absolute format: "50000", "100000", etc.
///
/// # Examples
///
/// ```ignore
/// use forge::compaction::parse_context_limit;
///
/// assert_eq!(parse_context_limit("80%")?, ContextLimit::Percentage(80.0));
/// assert_eq!(parse_context_limit("50000")?, ContextLimit::Absolute(50000));
/// ```
pub fn parse_context_limit(s: &str) -> Result<ContextLimit> {
    let s = s.trim();

    if s.is_empty() {
        anyhow::bail!("Context limit cannot be empty");
    }

    if let Some(num_str) = s.strip_suffix('%') {
        let pct: f32 = num_str
            .parse()
            .with_context(|| format!("Invalid percentage in context limit: {}", s))?;

        if pct <= 0.0 || pct > 100.0 {
            anyhow::bail!(
                "Context limit percentage must be between 0 and 100, got {}",
                pct
            );
        }

        Ok(ContextLimit::Percentage(pct))
    } else {
        let chars: usize = s
            .parse()
            .with_context(|| format!("Invalid absolute context limit: {}", s))?;

        if chars == 0 {
            anyhow::bail!("Context limit cannot be zero");
        }

        Ok(ContextLimit::Absolute(chars))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_percentage() {
        assert_eq!(
            parse_context_limit("80%").unwrap(),
            ContextLimit::Percentage(80.0)
        );
        assert_eq!(
            parse_context_limit("50%").unwrap(),
            ContextLimit::Percentage(50.0)
        );
        assert_eq!(
            parse_context_limit("100%").unwrap(),
            ContextLimit::Percentage(100.0)
        );
        assert_eq!(
            parse_context_limit("33.5%").unwrap(),
            ContextLimit::Percentage(33.5)
        );
    }

    #[test]
    fn test_parse_absolute() {
        assert_eq!(
            parse_context_limit("50000").unwrap(),
            ContextLimit::Absolute(50000)
        );
        assert_eq!(
            parse_context_limit("100000").unwrap(),
            ContextLimit::Absolute(100000)
        );
    }

    #[test]
    fn test_parse_with_whitespace() {
        assert_eq!(
            parse_context_limit("  80%  ").unwrap(),
            ContextLimit::Percentage(80.0)
        );
        assert_eq!(
            parse_context_limit(" 50000 ").unwrap(),
            ContextLimit::Absolute(50000)
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_context_limit("").is_err());
        assert!(parse_context_limit("invalid").is_err());
        assert!(parse_context_limit("0%").is_err());
        assert!(parse_context_limit("150%").is_err());
        assert!(parse_context_limit("0").is_err());
        assert!(parse_context_limit("-50%").is_err());
    }

    #[test]
    fn test_effective_limit() {
        let model_window = 800_000; // 800k chars

        let pct_limit = ContextLimit::Percentage(80.0);
        assert_eq!(pct_limit.effective_limit(model_window), 640_000);

        let abs_limit = ContextLimit::Absolute(500_000);
        assert_eq!(abs_limit.effective_limit(model_window), 500_000);
    }

    #[test]
    fn test_default() {
        assert_eq!(ContextLimit::default(), ContextLimit::Percentage(80.0));
    }

    #[test]
    fn test_display() {
        assert_eq!(ContextLimit::Percentage(80.0).to_string(), "80%");
        assert_eq!(ContextLimit::Absolute(50000).to_string(), "50000");
    }
}
