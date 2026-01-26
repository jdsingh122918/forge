//! Review system for quality gating in Forge.
//!
//! This module provides the review infrastructure that enables quality gates
//! before phase progression. Review specialists examine phase outputs and
//! provide verdicts that can optionally block further execution.
//!
//! ## Components
//!
//! - [`specialists`]: Review specialist types and configuration
//! - [`findings`]: Review output types (findings, reports, aggregations)
//! - [`arbiter`]: LLM-based resolution for failed reviews
//!
//! ## Example
//!
//! ```
//! use forge::review::{ReviewSpecialist, SpecialistType};
//! use forge::review::findings::{ReviewFinding, ReviewReport, ReviewVerdict, FindingSeverity};
//!
//! // Create a gating security review
//! let security = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
//!
//! // Check focus areas
//! let areas = security.focus_areas();
//! assert!(areas.iter().any(|a| a.contains("injection")));
//!
//! // Create a review report with findings
//! let finding = ReviewFinding::new(
//!     FindingSeverity::Warning,
//!     "src/auth.rs",
//!     "Potential SQL injection"
//! ).with_line(42);
//!
//! let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Warn)
//!     .with_summary("One finding related to SQL queries")
//!     .add_finding(finding);
//!
//! // Create multiple specialists for comprehensive review
//! let specialists = vec![
//!     ReviewSpecialist::gating(SpecialistType::SecuritySentinel),
//!     ReviewSpecialist::advisory(SpecialistType::PerformanceOracle),
//!     ReviewSpecialist::advisory(SpecialistType::SimplicityReviewer),
//! ];
//! ```
//!
//! ## Arbiter Usage
//!
//! The arbiter handles failed reviews by deciding whether to proceed, fix, or escalate:
//!
//! ```
//! use forge::review::{
//!     ArbiterConfig, ArbiterInput, ArbiterVerdict,
//!     ReviewAggregation, ReviewReport, ReviewVerdict, ReviewFinding, FindingSeverity,
//! };
//! use forge::review::arbiter::apply_rule_based_decision;
//!
//! // Create a failed review aggregation
//! let finding = ReviewFinding::new(
//!     FindingSeverity::Warning,
//!     "src/auth.rs",
//!     "Token stored in localStorage"
//! );
//! let report = ReviewReport::new("05", "security-sentinel", ReviewVerdict::Fail)
//!     .add_finding(finding);
//! let aggregation = ReviewAggregation::new("05").add_report(report);
//!
//! // Create arbiter input from the aggregation
//! let input = ArbiterInput::from_aggregation(&aggregation, 20, 5)
//!     .with_phase_name("OAuth Integration");
//!
//! // Configure arbiter (using rule-based for sync example)
//! let config = ArbiterConfig::arbiter_mode()
//!     .with_confidence_threshold(0.7);
//!
//! // Get a rule-based decision
//! let decision = apply_rule_based_decision(&input, &config);
//!
//! match decision.decision {
//!     ArbiterVerdict::Proceed => println!("Proceeding despite findings"),
//!     ArbiterVerdict::Fix => println!("Attempting to fix: {:?}", decision.fix_instructions),
//!     ArbiterVerdict::Escalate => println!("Escalating: {:?}", decision.escalation_summary),
//! }
//! ```

pub mod arbiter;
pub mod findings;
pub mod specialists;

// Re-export main types
pub use arbiter::{
    ArbiterConfig, ArbiterDecision, ArbiterExecutor, ArbiterInput, ArbiterResult, ArbiterVerdict,
    DecisionSource, ResolutionMode,
};
pub use findings::{FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport, ReviewVerdict};
pub use specialists::{ReviewSpecialist, SpecialistType};
