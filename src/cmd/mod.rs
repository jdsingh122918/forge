//! CLI command implementations.
//!
//! Each submodule owns one or more related `Commands` variants:
//!
//! | Module          | Commands handled                                   |
//! |-----------------|-----------------------------------------------------|
//! | `run`           | `Run`, `Phase`                                     |
//! | `phase`         | `List`, `Status`, `Reset`, `Audit`                 |
//! | `project`       | `Init`, `Interview`, `Generate`, `Implement`       |
//! | `patterns`      | `Learn`, `Patterns`                                |
//! | `config`        | `Config`                                           |
//! | `skills`        | `Skills`                                           |
//! | `compact`       | `Compact`                                          |
//! | `swarm`         | `Swarm`                                            |
//! | `factory`       | `Factory`                                          |

pub mod compact;
pub mod config;
pub mod factory;
pub mod patterns;
pub mod phase;
pub mod project;
pub mod run;
pub mod skills;
pub mod swarm;

pub use compact::cmd_compact;
pub use config::cmd_config;
pub use factory::cmd_factory;
pub use patterns::{cmd_learn, cmd_patterns};
pub use phase::{cmd_audit, cmd_list, cmd_reset, cmd_status};
pub use project::{cmd_generate, cmd_implement, cmd_init, cmd_interview};
pub use run::{run_orchestrator, run_single_phase};
pub use skills::cmd_skills;
pub use swarm::{cmd_swarm, cmd_swarm_abort, cmd_swarm_status};
