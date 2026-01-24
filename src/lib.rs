pub mod audit;
pub mod config;
pub mod forge_config;
pub mod gates;
pub mod generate;
pub mod init;
pub mod interview;
pub mod orchestrator;
pub mod patterns;
pub mod phase;
pub mod stream;
pub mod tracker;
pub mod ui;

// Re-export from phase for backward compatibility
pub mod phases {
    //! Backward compatibility re-exports from phase module.
    //! Use `crate::phase` directly in new code.
    pub use crate::phase::{Phase, get_all_phases, get_phase, get_phases_from};
}
