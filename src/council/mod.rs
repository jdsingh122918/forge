pub mod chairman;
pub mod config;
pub mod engine;
pub mod merge;
pub mod prompts;
pub mod reviewer;
pub mod types;
pub mod worker;

pub use chairman::{Chairman, ChairmanDecision, SynthesisResult};
pub use config::{CouncilConfig, WorkerConfig};
pub use engine::CouncilEngine;
pub use merge::{PatchSet, WorktreeManager, apply_patch, detect_conflicts};
pub use reviewer::{PeerReviewEngine, ReviewRound};
pub use types::*;
pub use worker::{ClaudeWorker, MockWorker, Worker};
