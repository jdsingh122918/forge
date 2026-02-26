//! Review specialist types for quality gating.
//!
//! This module defines the review specialists that can examine phase outputs
//! and provide quality gates before progression. Each specialist type has
//! default focus areas appropriate to its domain.
//!
//! ## Specialist Types
//!
//! - [`SpecialistType::SecuritySentinel`]: Security-focused review
//! - [`SpecialistType::PerformanceOracle`]: Performance-focused review
//! - [`SpecialistType::ArchitectureStrategist`]: Architecture-focused review
//! - [`SpecialistType::SimplicityReviewer`]: Simplicity/over-engineering review
//! - [`SpecialistType::Custom`]: User-defined review type
//!
//! ## Example
//!
//! ```
//! use forge::review::{ReviewSpecialist, SpecialistType};
//!
//! // Create a gating security review
//! let security = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
//! assert_eq!(security.display_name(), "Security Sentinel");
//! assert!(security.focus_areas().iter().any(|a| a.contains("injection")));
//!
//! // Create a non-gating custom review
//! let custom = ReviewSpecialist::new(
//!     SpecialistType::Custom("API Compliance".to_string()),
//!     false
//! ).with_focus_areas(vec!["REST conventions".to_string()]);
//! ```

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Type of review specialist.
///
/// Each type represents a domain of expertise with default focus areas.
///
/// ## Deserialization
///
/// This type supports multiple deserialization formats:
/// - Short-form strings: `"security"`, `"performance"`, `"architecture"`, `"simplicity"`
/// - Long-form strings: `"security_sentinel"`, `"performance_oracle"`, etc.
/// - Hyphenated strings: `"security-sentinel"`, `"performance-oracle"`, etc.
/// - Tagged object for custom: `{"custom": "my-review"}`
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistType {
    /// Security-focused review examining vulnerabilities and security best practices.
    #[default]
    SecuritySentinel,
    /// Performance-focused review examining efficiency and resource usage.
    PerformanceOracle,
    /// Architecture-focused review examining design patterns and structure.
    ArchitectureStrategist,
    /// Simplicity-focused review examining over-engineering and complexity.
    SimplicityReviewer,
    /// Custom review type with user-defined name.
    Custom(String),
}

impl SpecialistType {
    /// Get the human-readable display name for this specialist type.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    ///
    /// assert_eq!(SpecialistType::SecuritySentinel.display_name(), "Security Sentinel");
    /// assert_eq!(SpecialistType::Custom("My Review".to_string()).display_name(), "My Review");
    /// ```
    pub fn display_name(&self) -> &str {
        match self {
            Self::SecuritySentinel => "Security Sentinel",
            Self::PerformanceOracle => "Performance Oracle",
            Self::ArchitectureStrategist => "Architecture Strategist",
            Self::SimplicityReviewer => "Simplicity Reviewer",
            Self::Custom(name) => name,
        }
    }

    /// Get the agent name used for spawning review agents.
    ///
    /// This is a lowercase, hyphenated version suitable for agent identifiers.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    ///
    /// assert_eq!(SpecialistType::SecuritySentinel.agent_name(), "security-sentinel");
    /// assert_eq!(SpecialistType::Custom("Code Quality".to_string()).agent_name(), "code-quality");
    /// ```
    pub fn agent_name(&self) -> String {
        match self {
            Self::SecuritySentinel => "security-sentinel".to_string(),
            Self::PerformanceOracle => "performance-oracle".to_string(),
            Self::ArchitectureStrategist => "architecture-strategist".to_string(),
            Self::SimplicityReviewer => "simplicity-reviewer".to_string(),
            Self::Custom(name) => name.to_lowercase().replace(' ', "-"),
        }
    }

    /// Get the default focus areas for this specialist type.
    ///
    /// Returns a list of specific concerns this specialist should examine.
    /// Custom specialists return an empty list by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    ///
    /// let security = SpecialistType::SecuritySentinel;
    /// let areas = security.focus_areas();
    /// assert!(areas.iter().any(|a| a.contains("injection")));
    /// ```
    pub fn focus_areas(&self) -> Vec<&'static str> {
        match self {
            Self::SecuritySentinel => vec![
                "SQL injection vulnerabilities",
                "Cross-site scripting (XSS)",
                "Authentication bypass risks",
                "Secrets exposure in code or logs",
                "Input validation gaps",
                "Command injection vectors",
                "Path traversal vulnerabilities",
                "Insecure deserialization",
            ],
            Self::PerformanceOracle => vec![
                "N+1 query patterns",
                "Missing database indexes",
                "Memory leaks and unbounded growth",
                "Algorithmic complexity issues",
                "Unnecessary allocations",
                "Blocking operations in async code",
                "Cache misuse or missing caching",
                "Inefficient data structures",
            ],
            Self::ArchitectureStrategist => vec![
                "SOLID principle violations",
                "Excessive coupling between modules",
                "Layering violations",
                "Separation of concerns issues",
                "Circular dependencies",
                "Inconsistent abstraction levels",
                "Missing or weak interfaces",
                "God objects or functions",
            ],
            Self::SimplicityReviewer => vec![
                "Over-engineering patterns",
                "Premature abstraction",
                "YAGNI violations",
                "Unnecessary complexity",
                "Dead code or unused features",
                "Overly clever solutions",
                "Excessive indirection",
                "Configuration over convention abuse",
            ],
            Self::Custom(_) => vec![],
        }
    }

    /// Check if this is a built-in specialist type.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    ///
    /// assert!(SpecialistType::SecuritySentinel.is_builtin());
    /// assert!(!SpecialistType::Custom("Test".to_string()).is_builtin());
    /// ```
    pub fn is_builtin(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }

    /// Get all built-in specialist types.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    ///
    /// let builtins = SpecialistType::all_builtins();
    /// assert_eq!(builtins.len(), 4);
    /// ```
    pub fn all_builtins() -> Vec<Self> {
        vec![
            Self::SecuritySentinel,
            Self::PerformanceOracle,
            Self::ArchitectureStrategist,
            Self::SimplicityReviewer,
        ]
    }
}

impl std::fmt::Display for SpecialistType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Custom `Deserialize` for `SpecialistType`.
///
/// Supports three formats:
/// 1. Short-form string alias: `"security"`, `"perf"`, `"architecture"`, `"simplicity"`, etc.
/// 2. Long-form snake_case string: `"security_sentinel"`, `"performance_oracle"`, etc.
/// 3. Tagged object for custom variants: `{"custom": "my-review"}`
///
/// All string forms are routed through `FromStr`, which handles all known aliases and falls
/// back to `Custom(...)` for unrecognized values. The tagged object form is handled by an
/// internal visitor that mirrors the `#[serde(rename_all = "snake_case")]` enum layout.
impl<'de> serde::Deserialize<'de> for SpecialistType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, Visitor};

        struct SpecialistTypeVisitor;

        impl<'de> Visitor<'de> for SpecialistTypeVisitor {
            type Value = SpecialistType;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    r#"a specialist type string (e.g. "security", "security_sentinel") or a tagged object (e.g. {"custom": "my-review"})"#,
                )
            }

            /// Handle plain string values via `FromStr` — supports all aliases.
            fn visit_str<E: de::Error>(self, value: &str) -> Result<SpecialistType, E> {
                SpecialistType::from_str(value).map_err(de::Error::custom)
            }

            /// Handle map values — only `{"custom": "<name>"}` is valid.
            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<SpecialistType, A::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::custom("expected a key in specialist type object"))?;

                if key == "custom" {
                    let value: String = map.next_value()?;
                    // Drain any remaining keys (there should be none).
                    while map.next_key::<serde::de::IgnoredAny>()?.is_some() {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                    Ok(SpecialistType::Custom(value))
                } else {
                    Err(de::Error::unknown_variant(
                        &key,
                        &[
                            "security_sentinel",
                            "performance_oracle",
                            "architecture_strategist",
                            "simplicity_reviewer",
                            "custom",
                        ],
                    ))
                }
            }
        }

        deserializer.deserialize_any(SpecialistTypeVisitor)
    }
}

impl FromStr for SpecialistType {
    type Err = std::convert::Infallible;

    /// Parse a specialist type from a string identifier.
    ///
    /// Recognizes common aliases and falls back to Custom for unknown values.
    /// This implementation never fails - unknown strings become Custom variants.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::SpecialistType;
    /// use std::str::FromStr;
    ///
    /// assert_eq!(SpecialistType::from_str("security").unwrap(), SpecialistType::SecuritySentinel);
    /// assert_eq!(SpecialistType::from_str("perf").unwrap(), SpecialistType::PerformanceOracle);
    /// assert_eq!(SpecialistType::from_str("unknown").unwrap(), SpecialistType::Custom("unknown".to_string()));
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "security" | "security-sentinel" | "security_sentinel" => Self::SecuritySentinel,
            "performance" | "perf" | "performance-oracle" | "performance_oracle" => {
                Self::PerformanceOracle
            }
            "architecture" | "arch" | "architecture-strategist" | "architecture_strategist" => {
                Self::ArchitectureStrategist
            }
            "simplicity" | "simple" | "simplicity-reviewer" | "simplicity_reviewer" => {
                Self::SimplicityReviewer
            }
            _ => Self::Custom(s.to_string()),
        })
    }
}

/// Configuration for a review specialist.
///
/// Combines a specialist type with gating behavior and optional custom focus areas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewSpecialist {
    /// Type of specialist.
    pub specialist_type: SpecialistType,
    /// Whether this review gates phase completion.
    /// If true, failures must be resolved before proceeding.
    #[serde(default)]
    pub gate: bool,
    /// Custom focus areas (overrides defaults if non-empty).
    #[serde(default)]
    pub custom_focus_areas: Vec<String>,
}

impl ReviewSpecialist {
    /// Create a new review specialist configuration.
    ///
    /// # Arguments
    ///
    /// * `specialist_type` - The type of specialist
    /// * `gate` - Whether failures should block phase completion
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::{ReviewSpecialist, SpecialistType};
    ///
    /// let gating = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
    /// assert!(gating.gate);
    ///
    /// let advisory = ReviewSpecialist::new(SpecialistType::PerformanceOracle, false);
    /// assert!(!advisory.gate);
    /// ```
    pub fn new(specialist_type: SpecialistType, gate: bool) -> Self {
        Self {
            specialist_type,
            gate,
            custom_focus_areas: Vec::new(),
        }
    }

    /// Create a gating review specialist.
    ///
    /// Convenience method for creating specialists that block on failures.
    pub fn gating(specialist_type: SpecialistType) -> Self {
        Self::new(specialist_type, true)
    }

    /// Create a non-gating (advisory) review specialist.
    ///
    /// Convenience method for creating specialists that only warn.
    pub fn advisory(specialist_type: SpecialistType) -> Self {
        Self::new(specialist_type, false)
    }

    /// Add custom focus areas to this specialist.
    ///
    /// Custom focus areas replace the default ones when non-empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::{ReviewSpecialist, SpecialistType};
    ///
    /// let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true)
    ///     .with_focus_areas(vec!["API key exposure".to_string()]);
    ///
    /// let areas = specialist.focus_areas();
    /// assert_eq!(areas.len(), 1);
    /// assert_eq!(areas[0], "API key exposure");
    /// ```
    pub fn with_focus_areas(mut self, areas: Vec<String>) -> Self {
        self.custom_focus_areas = areas;
        self
    }

    /// Get the display name for this specialist.
    pub fn display_name(&self) -> &str {
        self.specialist_type.display_name()
    }

    /// Get the agent name for this specialist.
    pub fn agent_name(&self) -> String {
        self.specialist_type.agent_name()
    }

    /// Get the focus areas for this specialist.
    ///
    /// Returns custom focus areas if set, otherwise returns default focus areas
    /// for the specialist type.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge::review::{ReviewSpecialist, SpecialistType};
    ///
    /// // Default focus areas
    /// let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
    /// let areas = specialist.focus_areas();
    /// assert!(areas.iter().any(|a| a.contains("injection")));
    ///
    /// // Custom focus areas override defaults
    /// let custom = specialist.with_focus_areas(vec!["Custom area".to_string()]);
    /// assert_eq!(custom.focus_areas(), vec!["Custom area"]);
    /// ```
    pub fn focus_areas(&self) -> Vec<&str> {
        if self.custom_focus_areas.is_empty() {
            self.specialist_type.focus_areas()
        } else {
            self.custom_focus_areas.iter().map(|s| s.as_str()).collect()
        }
    }

    /// Check if this specialist gates phase completion.
    pub fn is_gating(&self) -> bool {
        self.gate
    }

    /// Check if this is a built-in specialist type.
    pub fn is_builtin(&self) -> bool {
        self.specialist_type.is_builtin()
    }

    /// Create all built-in specialists configured as gating.
    ///
    /// Used by sensitive phase detection to enable full review coverage.
    pub fn all_builtin_as_gating() -> Vec<Self> {
        SpecialistType::all_builtins()
            .into_iter()
            .map(Self::gating)
            .collect()
    }
}

impl Default for ReviewSpecialist {
    fn default() -> Self {
        Self::gating(SpecialistType::SecuritySentinel)
    }
}

impl std::fmt::Display for ReviewSpecialist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.gate {
            write!(f, "{} (gating)", self.specialist_type)
        } else {
            write!(f, "{} (advisory)", self.specialist_type)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================
    // SpecialistType tests
    // =========================================

    #[test]
    fn test_specialist_type_display_name() {
        assert_eq!(
            SpecialistType::SecuritySentinel.display_name(),
            "Security Sentinel"
        );
        assert_eq!(
            SpecialistType::PerformanceOracle.display_name(),
            "Performance Oracle"
        );
        assert_eq!(
            SpecialistType::ArchitectureStrategist.display_name(),
            "Architecture Strategist"
        );
        assert_eq!(
            SpecialistType::SimplicityReviewer.display_name(),
            "Simplicity Reviewer"
        );
        assert_eq!(
            SpecialistType::Custom("My Review".to_string()).display_name(),
            "My Review"
        );
    }

    #[test]
    fn test_specialist_type_agent_name() {
        assert_eq!(
            SpecialistType::SecuritySentinel.agent_name(),
            "security-sentinel"
        );
        assert_eq!(
            SpecialistType::PerformanceOracle.agent_name(),
            "performance-oracle"
        );
        assert_eq!(
            SpecialistType::ArchitectureStrategist.agent_name(),
            "architecture-strategist"
        );
        assert_eq!(
            SpecialistType::SimplicityReviewer.agent_name(),
            "simplicity-reviewer"
        );
        assert_eq!(
            SpecialistType::Custom("Code Quality".to_string()).agent_name(),
            "code-quality"
        );
    }

    #[test]
    fn test_specialist_type_focus_areas_security() {
        let areas = SpecialistType::SecuritySentinel.focus_areas();
        assert!(!areas.is_empty());
        assert!(areas.iter().any(|a| a.contains("injection")));
        assert!(areas.iter().any(|a| a.contains("XSS")));
    }

    #[test]
    fn test_specialist_type_focus_areas_performance() {
        let areas = SpecialistType::PerformanceOracle.focus_areas();
        assert!(!areas.is_empty());
        assert!(areas.iter().any(|a| a.contains("N+1")));
        assert!(areas.iter().any(|a| a.contains("index")));
    }

    #[test]
    fn test_specialist_type_focus_areas_architecture() {
        let areas = SpecialistType::ArchitectureStrategist.focus_areas();
        assert!(!areas.is_empty());
        assert!(areas.iter().any(|a| a.contains("SOLID")));
        assert!(areas.iter().any(|a| a.contains("coupling")));
    }

    #[test]
    fn test_specialist_type_focus_areas_simplicity() {
        let areas = SpecialistType::SimplicityReviewer.focus_areas();
        assert!(!areas.is_empty());
        assert!(
            areas
                .iter()
                .any(|a| a.to_lowercase().contains("over-engineering"))
        );
        assert!(areas.iter().any(|a| a.contains("YAGNI")));
    }

    #[test]
    fn test_specialist_type_focus_areas_custom_empty() {
        let areas = SpecialistType::Custom("Test".to_string()).focus_areas();
        assert!(areas.is_empty());
    }

    #[test]
    fn test_specialist_type_from_str() {
        assert_eq!(
            SpecialistType::from_str("security").unwrap(),
            SpecialistType::SecuritySentinel
        );
        assert_eq!(
            SpecialistType::from_str("SECURITY").unwrap(),
            SpecialistType::SecuritySentinel
        );
        assert_eq!(
            SpecialistType::from_str("security-sentinel").unwrap(),
            SpecialistType::SecuritySentinel
        );
        assert_eq!(
            SpecialistType::from_str("performance").unwrap(),
            SpecialistType::PerformanceOracle
        );
        assert_eq!(
            SpecialistType::from_str("perf").unwrap(),
            SpecialistType::PerformanceOracle
        );
        assert_eq!(
            SpecialistType::from_str("architecture").unwrap(),
            SpecialistType::ArchitectureStrategist
        );
        assert_eq!(
            SpecialistType::from_str("arch").unwrap(),
            SpecialistType::ArchitectureStrategist
        );
        assert_eq!(
            SpecialistType::from_str("simplicity").unwrap(),
            SpecialistType::SimplicityReviewer
        );
        assert_eq!(
            SpecialistType::from_str("simple").unwrap(),
            SpecialistType::SimplicityReviewer
        );
        assert_eq!(
            SpecialistType::from_str("unknown").unwrap(),
            SpecialistType::Custom("unknown".to_string())
        );
    }

    #[test]
    fn test_specialist_type_is_builtin() {
        assert!(SpecialistType::SecuritySentinel.is_builtin());
        assert!(SpecialistType::PerformanceOracle.is_builtin());
        assert!(SpecialistType::ArchitectureStrategist.is_builtin());
        assert!(SpecialistType::SimplicityReviewer.is_builtin());
        assert!(!SpecialistType::Custom("Test".to_string()).is_builtin());
    }

    #[test]
    fn test_specialist_type_all_builtins() {
        let builtins = SpecialistType::all_builtins();
        assert_eq!(builtins.len(), 4);
        assert!(builtins.contains(&SpecialistType::SecuritySentinel));
        assert!(builtins.contains(&SpecialistType::PerformanceOracle));
        assert!(builtins.contains(&SpecialistType::ArchitectureStrategist));
        assert!(builtins.contains(&SpecialistType::SimplicityReviewer));
    }

    #[test]
    fn test_specialist_type_serialization() {
        let json = serde_json::to_string(&SpecialistType::SecuritySentinel).unwrap();
        assert_eq!(json, "\"security_sentinel\"");

        let custom = serde_json::to_string(&SpecialistType::Custom("Test".to_string())).unwrap();
        assert_eq!(custom, "{\"custom\":\"Test\"}");
    }

    #[test]
    fn test_specialist_type_deserialization() {
        // Long-form snake_case strings (original format)
        let security: SpecialistType = serde_json::from_str("\"security_sentinel\"").unwrap();
        assert_eq!(security, SpecialistType::SecuritySentinel);

        let custom: SpecialistType = serde_json::from_str("{\"custom\":\"Test\"}").unwrap();
        assert_eq!(custom, SpecialistType::Custom("Test".to_string()));

        // Short-form alias strings (used in phases.json configs)
        let security_short: SpecialistType = serde_json::from_str("\"security\"").unwrap();
        assert_eq!(security_short, SpecialistType::SecuritySentinel);

        let perf_short: SpecialistType = serde_json::from_str("\"performance\"").unwrap();
        assert_eq!(perf_short, SpecialistType::PerformanceOracle);

        let arch_short: SpecialistType = serde_json::from_str("\"architecture\"").unwrap();
        assert_eq!(arch_short, SpecialistType::ArchitectureStrategist);

        let simplicity_short: SpecialistType = serde_json::from_str("\"simplicity\"").unwrap();
        assert_eq!(simplicity_short, SpecialistType::SimplicityReviewer);

        // Hyphenated strings
        let security_hyphen: SpecialistType =
            serde_json::from_str("\"security-sentinel\"").unwrap();
        assert_eq!(security_hyphen, SpecialistType::SecuritySentinel);

        // Unknown strings fall back to Custom
        let unknown: SpecialistType = serde_json::from_str("\"my-custom-review\"").unwrap();
        assert_eq!(
            unknown,
            SpecialistType::Custom("my-custom-review".to_string())
        );
    }

    #[test]
    fn test_specialist_type_deserialization_all_formats() {
        // Verify the three formats described in the requirements:
        // 1. Short-form string: "security" -> SecuritySentinel
        let s1: SpecialistType = serde_json::from_str("\"security\"").unwrap();
        assert_eq!(s1, SpecialistType::SecuritySentinel);

        // 2. Long-form string: "security_sentinel" -> SecuritySentinel
        let s2: SpecialistType = serde_json::from_str("\"security_sentinel\"").unwrap();
        assert_eq!(s2, SpecialistType::SecuritySentinel);

        // 3. Tagged object: {"custom": "my-review"} -> Custom("my-review")
        let s3: SpecialistType = serde_json::from_str("{\"custom\":\"my-review\"}").unwrap();
        assert_eq!(s3, SpecialistType::Custom("my-review".to_string()));
    }

    #[test]
    fn test_specialist_type_display() {
        assert_eq!(
            format!("{}", SpecialistType::SecuritySentinel),
            "Security Sentinel"
        );
        assert_eq!(
            format!("{}", SpecialistType::Custom("My Type".to_string())),
            "My Type"
        );
    }

    #[test]
    fn test_specialist_type_default() {
        let default = SpecialistType::default();
        assert_eq!(default, SpecialistType::SecuritySentinel);
    }

    // =========================================
    // ReviewSpecialist tests
    // =========================================

    #[test]
    fn test_review_specialist_new() {
        let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
        assert_eq!(specialist.specialist_type, SpecialistType::SecuritySentinel);
        assert!(specialist.gate);
        assert!(specialist.custom_focus_areas.is_empty());
    }

    #[test]
    fn test_review_specialist_gating() {
        let specialist = ReviewSpecialist::gating(SpecialistType::PerformanceOracle);
        assert!(specialist.gate);
        assert!(specialist.is_gating());
    }

    #[test]
    fn test_review_specialist_advisory() {
        let specialist = ReviewSpecialist::advisory(SpecialistType::ArchitectureStrategist);
        assert!(!specialist.gate);
        assert!(!specialist.is_gating());
    }

    #[test]
    fn test_review_specialist_with_focus_areas() {
        let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true)
            .with_focus_areas(vec![
                "Custom area 1".to_string(),
                "Custom area 2".to_string(),
            ]);

        let areas = specialist.focus_areas();
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0], "Custom area 1");
        assert_eq!(areas[1], "Custom area 2");
    }

    #[test]
    fn test_review_specialist_default_focus_areas() {
        let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
        let areas = specialist.focus_areas();
        assert!(!areas.is_empty());
        assert!(areas.iter().any(|a| a.contains("injection")));
    }

    #[test]
    fn test_review_specialist_display_name() {
        let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
        assert_eq!(specialist.display_name(), "Security Sentinel");
    }

    #[test]
    fn test_review_specialist_agent_name() {
        let specialist = ReviewSpecialist::new(SpecialistType::PerformanceOracle, false);
        assert_eq!(specialist.agent_name(), "performance-oracle");
    }

    #[test]
    fn test_review_specialist_is_builtin() {
        let builtin = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true);
        assert!(builtin.is_builtin());

        let custom = ReviewSpecialist::new(SpecialistType::Custom("Test".to_string()), false);
        assert!(!custom.is_builtin());
    }

    #[test]
    fn test_review_specialist_display() {
        let gating = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
        assert_eq!(format!("{}", gating), "Security Sentinel (gating)");

        let advisory = ReviewSpecialist::advisory(SpecialistType::PerformanceOracle);
        assert_eq!(format!("{}", advisory), "Performance Oracle (advisory)");
    }

    #[test]
    fn test_review_specialist_default() {
        let default = ReviewSpecialist::default();
        assert_eq!(default.specialist_type, SpecialistType::SecuritySentinel);
        assert!(default.gate);
    }

    #[test]
    fn test_review_specialist_serialization() {
        let specialist = ReviewSpecialist::new(SpecialistType::SecuritySentinel, true)
            .with_focus_areas(vec!["Test area".to_string()]);

        let json = serde_json::to_string(&specialist).unwrap();
        let parsed: ReviewSpecialist = serde_json::from_str(&json).unwrap();

        assert_eq!(specialist.specialist_type, parsed.specialist_type);
        assert_eq!(specialist.gate, parsed.gate);
        assert_eq!(specialist.custom_focus_areas, parsed.custom_focus_areas);
    }

    #[test]
    fn test_review_specialist_deserialization_minimal() {
        let json = r#"{"specialist_type":"performance_oracle"}"#;
        let specialist: ReviewSpecialist = serde_json::from_str(json).unwrap();

        assert_eq!(
            specialist.specialist_type,
            SpecialistType::PerformanceOracle
        );
        assert!(!specialist.gate); // defaults to false
        assert!(specialist.custom_focus_areas.is_empty());
    }

    #[test]
    fn test_all_builtin_as_gating() {
        let specialists = ReviewSpecialist::all_builtin_as_gating();
        assert_eq!(specialists.len(), 4);
        for specialist in &specialists {
            assert!(specialist.is_gating());
            assert!(specialist.is_builtin());
        }
    }
}
