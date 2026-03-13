//! Protocol and capability constants exposed by the daemon.

pub const PROTOCOL_VERSION: u32 = 1;
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "submit_run_v1",
    "attach_run_v1",
    "child_task_v1",
    "streaming_v1",
];
