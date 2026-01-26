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
//!
//! ## Planned Components (future phases)
//!
//! - `arbiter`: LLM-based resolution for failed reviews (Phase 16)
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

pub mod findings;
pub mod specialists;

// Re-export main types
pub use findings::{FindingSeverity, ReviewAggregation, ReviewFinding, ReviewReport, ReviewVerdict};
pub use specialists::{ReviewSpecialist, SpecialistType};
