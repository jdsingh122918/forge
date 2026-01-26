//! Review system for quality gating in Forge.
//!
//! This module provides the review infrastructure that enables quality gates
//! before phase progression. Review specialists examine phase outputs and
//! provide verdicts that can optionally block further execution.
//!
//! ## Components
//!
//! - [`specialists`]: Review specialist types and configuration
//!
//! ## Planned Components (future phases)
//!
//! - `findings`: Review output types (Phase 15)
//! - `arbiter`: LLM-based resolution for failed reviews (Phase 16)
//!
//! ## Example
//!
//! ```
//! use forge::review::{ReviewSpecialist, SpecialistType};
//!
//! // Create a gating security review
//! let security = ReviewSpecialist::gating(SpecialistType::SecuritySentinel);
//!
//! // Check focus areas
//! let areas = security.focus_areas();
//! assert!(areas.iter().any(|a| a.contains("injection")));
//!
//! // Create multiple specialists for comprehensive review
//! let specialists = vec![
//!     ReviewSpecialist::gating(SpecialistType::SecuritySentinel),
//!     ReviewSpecialist::advisory(SpecialistType::PerformanceOracle),
//!     ReviewSpecialist::advisory(SpecialistType::SimplicityReviewer),
//! ];
//! ```

pub mod specialists;

// Re-export main types
pub use specialists::{ReviewSpecialist, SpecialistType};
