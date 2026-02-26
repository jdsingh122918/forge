pub mod audit;
pub mod compaction;
pub mod config;
pub mod dag;
pub mod decomposition;
pub mod errors;
pub mod factory;
pub mod forge_config;
pub mod gates;
pub mod generate;
pub mod hooks;
pub mod implement;
pub mod init;
pub mod interview;
pub mod orchestrator;
pub mod patterns;
pub mod phase;
pub mod review;
pub mod signals;
pub mod skills;
pub mod stream;
pub mod subphase;
pub mod swarm;
pub mod tracker;
pub mod ui;
pub mod util;

// Re-export from phase for backward compatibility
pub mod phases {
    //! Backward compatibility re-exports from phase module.
    //! Use `crate::phase` directly in new code.
    pub use crate::phase::{Phase, get_all_phases, get_phase, get_phases_from};
}
