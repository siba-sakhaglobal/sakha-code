//! sakha-core: shared IDs, error taxonomy, retry policy, event envelopes,
//! budgets, artifact references, and time helpers used by every other Sakha
//! crate. Per the dependency direction (`ui -> daemon -> agent -> loop ->
//! tools/provider/context/security -> core`), this crate has no internal
//! workspace dependencies.

pub mod artifact;
pub mod budget;
pub mod error;
pub mod event;
pub mod id;
pub mod retry;
pub mod runtime;
pub mod time;

pub use artifact::{ArtifactKind, ArtifactRef};
pub use budget::{Budget, BudgetDimension, BudgetLedger};
pub use error::{ErrorClass, ModuleId, SakhaError, SakhaResult};
pub use event::{EventEnvelope, EventKind};
pub use id::{
    ArtifactId, CheckpointId, GoalId, LoopId, LoopTickId, ModelRequestId, ProviderProfileId,
    SessionId, ToolCallId, TurnId, WorkspaceId,
};
pub use retry::{ExponentialBackoff, RetryAfterOrBackoff, RetryPolicy};
pub use runtime::{CancellationToken, EventBus, EventSink, EventStream, RuntimeError};
