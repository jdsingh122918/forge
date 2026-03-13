//! Strongly-typed identifier newtypes for the Forge runtime domain.
//!
//! Each ID wraps a `String` and provides `Display`, `From<String>`, `AsRef<str>`,
//! and serde support. Using newtype wrappers prevents accidental misuse of one
//! identifier kind where another is expected (e.g., passing a `RunId` where an
//! `AgentId` is required).

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! define_id {
    ($(#[doc = $doc:expr])* $name:ident) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            /// Create a new identifier from any string-like value.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Generate a new random identifier using a UUID v4.
            pub fn generate() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }

            /// Return the inner string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the wrapper and return the inner `String`.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id! {
    /// Unique identifier for an entire run (a submitted plan execution).
    RunId
}

define_id! {
    /// Unique identifier for a task node within a run graph.
    ///
    /// Task nodes are the durable unit of work: they track dependencies, budget,
    /// expected output, approvals, memory scope, and audit trail.
    TaskNodeId
}

define_id! {
    /// Unique identifier for an agent instance (the concrete worker process
    /// executing a task node).
    AgentId
}

define_id! {
    /// Unique identifier for a milestone (operator-facing phase marker).
    ///
    /// Top-level phases remain user-visible milestones; runtime decomposition
    /// produces child `TaskNode`s rather than sub-phases.
    MilestoneId
}

define_id! {
    /// Unique identifier for a pending approval request.
    ///
    /// Approval attaches to task-node creation — once approved, retries or
    /// backend restarts do not re-prompt unless requested capabilities change.
    ApprovalId
}

define_id! {
    /// Unique identifier for a pending child-task spawn request that is
    /// awaiting approval.
    SpawnId
}

define_id! {
    /// Unique identifier for a message bus channel (used for request/reply
    /// communication between task nodes).
    ChannelId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_types() {
        let run = RunId::new("r1");
        let task = TaskNodeId::new("r1");
        // Same inner value but different types — this is the point of newtypes.
        assert_eq!(run.as_str(), task.as_str());
        // Compile-time: you cannot pass a RunId where a TaskNodeId is expected.
    }

    #[test]
    fn generate_produces_unique_ids() {
        let a = AgentId::generate();
        let b = AgentId::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn roundtrip_serde() {
        let id = RunId::new("test-run-123");
        let json = serde_json::to_string(&id).unwrap();
        let back: RunId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
