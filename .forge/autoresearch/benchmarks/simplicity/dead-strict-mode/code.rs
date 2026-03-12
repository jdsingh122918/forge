use serde::{Deserialize, Serialize};
use anyhow;

/// Permission mode controls what tools are available and how iteration approval works.
///
/// | Mode       | When to use              | Gate behavior                           |
/// |------------|--------------------------|-----------------------------------------|
/// | `Readonly` | Auditing / inspection    | Restricts toolset to read-only tools    |
/// | `Standard` | Normal development       | Threshold-based auto-approve (<=N files)|
/// | `Autonomous`| Well-tested, CI         | Auto-approves all iterations            |
/// | `Strict`   | Sensitive / high-risk    | Requires manual approval every iteration|
///
/// PROBLEM: Strict is documented as requiring manual approval, but the implementation
/// does not differentiate it from Standard. No code path checks for Strict specifically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Require approval for every iteration (sensitive phases).
    /// DEAD: No gate logic differentiates this from Standard.
    Strict,
    /// Approve phase start, auto-continue iterations (default).
    #[default]
    Standard,
    /// Auto-approve all iterations.
    Autonomous,
    /// Read-only: restrict to read-only tools.
    Readonly,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionMode::Strict => write!(f, "strict"),
            PermissionMode::Standard => write!(f, "standard"),
            PermissionMode::Autonomous => write!(f, "autonomous"),
            PermissionMode::Readonly => write!(f, "readonly"),
        }
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(PermissionMode::Strict),
            "standard" => Ok(PermissionMode::Standard),
            "autonomous" => Ok(PermissionMode::Autonomous),
            "readonly" => Ok(PermissionMode::Readonly),
            _ => anyhow::bail!(
                "Invalid permission mode '{}'. Valid values: strict, standard, autonomous, readonly",
                s
            ),
        }
    }
}

/// Returns restricted tool set for a permission mode, or None for unrestricted modes.
///
/// NOTE: Strict returns None (unrestricted) — identical to Standard.
/// If Strict truly required manual approval, this function would need to return
/// a different tool set or the gate logic would need a separate check.
pub fn tools_for_permission_mode(mode: PermissionMode) -> Option<Vec<String>> {
    match mode {
        PermissionMode::Readonly => Some(vec![
            "Read".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "Bash".to_string(), // read-only commands only
        ]),
        PermissionMode::Strict => None,     // SAME AS STANDARD — dead variant
        PermissionMode::Standard => None,    // Unrestricted
        PermissionMode::Autonomous => None,  // Unrestricted
    }
}

/// Gate check: should auto-approve this iteration?
///
/// NOTE: Strict is not checked here at all — it falls through to the
/// Standard behavior. There is no "require manual approval" code path.
pub fn should_auto_approve(mode: PermissionMode, files_changed: usize, threshold: usize) -> bool {
    match mode {
        PermissionMode::Autonomous => true,
        PermissionMode::Readonly => false,
        // Strict falls through to Standard — no special handling
        PermissionMode::Strict | PermissionMode::Standard => {
            files_changed <= threshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_mode_display() {
        assert_eq!(PermissionMode::Strict.to_string(), "strict");
        assert_eq!(PermissionMode::Standard.to_string(), "standard");
    }

    #[test]
    fn test_tools_for_strict_mode() {
        // Strict returns None — same as Standard
        assert!(tools_for_permission_mode(PermissionMode::Strict).is_none());
    }

    #[test]
    fn test_strict_auto_approve_same_as_standard() {
        // Strict behaves identically to Standard
        assert_eq!(
            should_auto_approve(PermissionMode::Strict, 3, 5),
            should_auto_approve(PermissionMode::Standard, 3, 5),
        );
    }
}
