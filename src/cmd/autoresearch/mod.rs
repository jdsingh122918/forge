//! Autoresearch command — automated specialist benchmark evaluation.

pub mod judge;

// Re-export benchmark types from the library crate for use by future command handlers.
#[allow(unused_imports)]
pub use forge::autoresearch::benchmarks;
