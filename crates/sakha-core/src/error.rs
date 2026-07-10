//! Cross-cutting error taxonomy shared by every layer (provider, tools, loop, daemon).
//!
//! See spec `modules/22-error-taxonomy-retry.md`. One error model so retry/abort
//! decisions are policy-driven rather than scattered `match` arms across crates.

use std::fmt;

use serde::{Deserialize, Serialize};

/// High-level classification of a failure. Drives default retry/abort handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// Network reset, 429, 5xx, provider overload. Default: retry with backoff.
    Transient,
    /// Token/cost/time budget exhausted. Default: stop loop, write handoff.
    Budget,
    /// Denied tool call, sandbox block. Default: surface to user, never retry silently.
    Permission,
    /// Bad tool args, schema mismatch. Default: return to model once, then fail turn.
    InvalidInput,
    /// Checkpoint corrupt, migration failure, patch conflict. Default: halt, need repair.
    Integrity,
    /// Config invalid, auth invalid. Default: abort with actionable message.
    Fatal,
}

impl ErrorClass {
    /// Whether this class is retryable by default (before considering idempotency
    /// or budget exhaustion, which can still veto a retry).
    pub fn default_retryable(self) -> bool {
        matches!(self, ErrorClass::Transient)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Transient => "transient",
            ErrorClass::Budget => "budget",
            ErrorClass::Permission => "permission",
            ErrorClass::InvalidInput => "invalid_input",
            ErrorClass::Integrity => "integrity",
            ErrorClass::Fatal => "fatal",
        }
    }
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Identifies the module/subsystem that raised an error, for audit and routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleId(pub String);

impl ModuleId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ModuleId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ModuleId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// The canonical Sakha error type. Carries enough structure for policy-driven
/// retry/abort decisions and for audit logging, while remaining ergonomic to
/// construct and to chain via `source`.
#[derive(Debug, Serialize, Deserialize)]
pub struct SakhaError {
    pub class: ErrorClass,
    pub retryable: bool,
    pub source_module: ModuleId,
    pub message: String,
    /// Optional provider or tool identifier associated with this error.
    pub subject_id: Option<String>,
    /// Optional lower-level cause, rendered as a string (kept serializable).
    pub cause: Option<String>,
}

impl SakhaError {
    pub fn new(class: ErrorClass, source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        let retryable = class.default_retryable();
        Self {
            class,
            retryable,
            source_module: source_module.into(),
            message: message.into(),
            subject_id: None,
            cause: None,
        }
    }

    pub fn transient(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Transient, source_module, message)
    }

    pub fn budget(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Budget, source_module, message)
    }

    pub fn permission(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Permission, source_module, message)
    }

    pub fn invalid_input(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::InvalidInput, source_module, message)
    }

    pub fn integrity(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Integrity, source_module, message)
    }

    pub fn fatal(source_module: impl Into<ModuleId>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Fatal, source_module, message)
    }

    /// Overrides the retryable flag (e.g. a Transient error that has exhausted
    /// its idempotency guarantee, or a caller-forced non-retry).
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_subject(mut self, subject_id: impl Into<String>) -> Self {
        self.subject_id = Some(subject_id.into());
        self
    }

    pub fn with_cause(mut self, cause: impl fmt::Display) -> Self {
        self.cause = Some(cause.to_string());
        self
    }
}

impl fmt::Display for SakhaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}: {}",
            self.class, self.source_module, self.message
        )?;
        if let Some(subject) = &self.subject_id {
            write!(f, " (subject={subject})")?;
        }
        if let Some(cause) = &self.cause {
            write!(f, " caused by: {cause}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SakhaError {}

impl From<anyhow::Error> for SakhaError {
    fn from(err: anyhow::Error) -> Self {
        SakhaError::fatal("unknown", err.to_string())
    }
}

pub type SakhaResult<T> = Result<T, SakhaError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_class_module_and_message() {
        let err = SakhaError::transient("sakha-provider", "connection reset");
        let s = err.to_string();
        assert!(s.contains("transient"));
        assert!(s.contains("sakha-provider"));
        assert!(s.contains("connection reset"));
    }

    #[test]
    fn transient_is_retryable_by_default() {
        let err = SakhaError::transient("sakha-provider", "429");
        assert!(err.retryable);
    }

    #[test]
    fn permission_is_not_retryable_by_default() {
        let err = SakhaError::permission("sakha-security", "denied");
        assert!(!err.retryable);
    }

    #[test]
    fn with_retryable_overrides_default() {
        let err = SakhaError::transient("sakha-provider", "network reset").with_retryable(false);
        assert!(!err.retryable);
    }

    #[test]
    fn with_subject_and_cause_are_included_in_display() {
        let err = SakhaError::integrity("sakha-memory", "checkpoint corrupt")
            .with_subject("checkpoint-42")
            .with_cause("crc mismatch");
        let s = err.to_string();
        assert!(s.contains("checkpoint-42"));
        assert!(s.contains("crc mismatch"));
    }

    #[test]
    fn error_class_serializes_snake_case() {
        let json = serde_json::to_string(&ErrorClass::InvalidInput).unwrap();
        assert_eq!(json, "\"invalid_input\"");
    }
}
