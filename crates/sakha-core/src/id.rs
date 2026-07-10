//! Typed newtype identifiers used across the Sakha workspace.
//!
//! Every domain identifier wraps a `uuid::Uuid` so that IDs from different
//! domains (a `SessionId` vs a `TurnId`) cannot be accidentally interchanged
//! at compile time, while still supporting stable string (de)serialization.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generates a typed newtype ID wrapping a `Uuid`.
macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Creates a new random ID (UUID v4).
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an ID from an existing `Uuid`.
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Returns the inner `Uuid`.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// A nil (all-zero) ID, useful as a sentinel/default in tests.
            pub fn nil() -> Self {
                Self(Uuid::nil())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

typed_id!(
    /// Identifies a workspace (a project/repo root Sakha is operating on).
    WorkspaceId
);
typed_id!(
    /// Identifies a running or resumable agent session.
    SessionId
);
typed_id!(
    /// Identifies a single turn (one user input -> one assistant output) within a session.
    TurnId
);
typed_id!(
    /// Identifies a single tool invocation.
    ToolCallId
);
typed_id!(
    /// Identifies a goal tracked by the planning/loop layer.
    GoalId
);
typed_id!(
    /// Identifies a long-running loop (scheduled, event-driven, verification, etc).
    LoopId
);
typed_id!(
    /// Identifies a stored artifact (raw or compressed content blob).
    ArtifactId
);
typed_id!(
    /// Identifies a model request sent to a provider.
    ModelRequestId
);
typed_id!(
    /// Identifies a provider profile (credentials + model + limits).
    ProviderProfileId
);
typed_id!(
    /// Identifies a checkpoint written by the runtime.
    CheckpointId
);
typed_id!(
    /// Identifies a single loop tick execution.
    LoopTickId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrips_through_json() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        let back: SessionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, back);
    }

    #[test]
    fn id_roundtrips_through_display_and_from_str() {
        let id = TurnId::new();
        let s = id.to_string();
        let back: TurnId = s.parse().expect("parse");
        assert_eq!(id, back);
    }

    #[test]
    fn distinct_ids_are_not_interchangeable_at_type_level() {
        // This test mostly documents intent: SessionId and TurnId are distinct
        // types, so the following would fail to compile if uncommented:
        // let s: SessionId = TurnId::new(); // compile error
        let session = SessionId::new();
        let turn = TurnId::new();
        assert_ne!(session.as_uuid(), Uuid::nil());
        assert_ne!(turn.as_uuid(), Uuid::nil());
    }

    #[test]
    fn nil_id_is_stable() {
        assert_eq!(GoalId::nil().as_uuid(), Uuid::nil());
    }

    #[test]
    fn new_ids_are_unique() {
        let a = LoopId::new();
        let b = LoopId::new();
        assert_ne!(a, b);
    }
}
