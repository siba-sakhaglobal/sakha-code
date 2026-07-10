//! `ToolExecutor`: runs the tool execution pipeline described in
//! `modules/07-tool-system.md` "Tool Execution Pipeline":
//! parse -> validate -> risk -> permission -> idempotency -> execute ->
//! capture -> audit -> normalized result.
//!
//! Compression (step 8, "compress result") is intentionally out of scope for
//! this crate — `sakha-compression` owns that, and a higher layer (the
//! agent's tool-call interpreter) is expected to route `ToolResult` through
//! it. This executor still writes the audit record and stores idempotency
//! results so a duplicate call can be detected here.

use std::collections::HashMap;
use std::sync::Mutex;

use sakha_core::{ArtifactRef, SakhaError, SakhaResult, ToolCallId};
use sakha_security::{PermissionPolicy, RiskClassifier, RiskLevel};

use crate::registry::ToolRegistry;
use crate::tool::{IdempotencyPolicy, ToolContext, ToolName, ToolResult};

/// A single tool call as parsed from a model response, before validation.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub tool_name: ToolName,
    pub input_json: serde_json::Value,
}

/// A completed audit record for one tool call, per spec "every tool call
/// writes an audit event before execution".
#[derive(Debug, Clone)]
pub struct ToolAuditRecord {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub started: bool,
    pub completed: bool,
    pub risk_level: RiskLevel,
    pub was_deduped: bool,
    pub raw_output_ref: Option<ArtifactRef>,
}

/// Computes a stable idempotency key from a tool name + validated input JSON,
/// used to detect duplicate calls with identical intent. Uses
/// `sakha_core::artifact::content_hash_hex` over the canonicalized
/// (serde_json's `to_string` on a `Value` is already key-order-stable for a
/// given parse) JSON bytes.
fn compute_idempotency_key(tool_name: &ToolName, input_json: &serde_json::Value) -> String {
    let mut bytes = tool_name.0.clone().into_bytes();
    bytes.push(0);
    bytes.extend_from_slice(input_json.to_string().as_bytes());
    sakha_core::artifact::content_hash_hex(&bytes)
}

/// Upper bound on the number of entries kept in the idempotency cache. Once
/// reached, the oldest entries (by insertion order) are evicted to make room
/// for new ones, so a long-running session can never grow this cache without
/// limit (spec: tool execution must stay within a bounded footprint).
const MAX_IDEMPOTENCY_CACHE_ENTRIES: usize = 1000;

/// Recovers a `Mutex` guard even if the mutex was poisoned by a prior
/// panicking holder, so a single panicked call site can never take down every
/// subsequent call through this executor. Poisoning here just means "the data
/// might be inconsistent from a half-finished mutation"; for an audit log
/// (append-only) and an idempotency cache (best-effort dedup) that's an
/// acceptable degradation, whereas propagating the panic (via `.unwrap()`)
/// would violate the "executor returns Err, never panics" contract.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Locks the idempotency cache, recovering from poisoning, and wraps the
/// (infallible in practice) result in `SakhaResult` so callers can use `?`
/// uniformly with other fallible steps in `execute()`.
fn lock_idempotency_cache(
    mutex: &Mutex<HashMap<String, ToolResult>>,
) -> SakhaResult<std::sync::MutexGuard<'_, HashMap<String, ToolResult>>> {
    Ok(lock_or_recover(mutex))
}

/// Drives a `ToolCall` through validate -> risk -> permission -> idempotency
/// -> execute, against a `ToolRegistry` and `PermissionPolicy`, and keeps an
/// in-memory audit trail plus an idempotency result cache.
pub struct ToolExecutor {
    pub registry: ToolRegistry,
    pub policy: PermissionPolicy,
    risk_classifier: RiskClassifier,
    audit_log: Mutex<Vec<ToolAuditRecord>>,
    idempotency_cache: Mutex<HashMap<String, ToolResult>>,
    /// Insertion order of `idempotency_cache` keys, used to evict the oldest
    /// entry once `MAX_IDEMPOTENCY_CACHE_ENTRIES` is reached.
    idempotency_order: Mutex<std::collections::VecDeque<String>>,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry, policy: PermissionPolicy) -> Self {
        Self {
            registry,
            policy,
            risk_classifier: RiskClassifier::new(),
            audit_log: Mutex::new(Vec::new()),
            idempotency_cache: Mutex::new(HashMap::new()),
            idempotency_order: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// Executes one tool call end-to-end. Returns `Err` (never panics) when
    /// the tool is unknown, validation fails, or permission is denied.
    pub async fn execute(&self, call: ToolCall, context: &ToolContext) -> SakhaResult<ToolResult> {
        let tool = self.registry.get_or_err(&call.tool_name)?;
        let spec = tool.spec();

        // Validate first: a call with malformed input should fail before we
        // ever ask about permission or risk.
        let validated = tool.validate(call.input_json)?;

        // Risk classification: the highest risk level across all permission
        // kinds this tool declares, used for audit + as a signal higher
        // layers (e.g. the agent) can use to require explicit approval.
        let risk_level = spec
            .permission_spec
            .required
            .iter()
            .map(|kind| {
                let request = sakha_security::PermissionRequest::new(*kind, call.tool_name.0.as_str(), "risk check");
                self.risk_classifier.classify(&request)
            })
            .max()
            .unwrap_or(RiskLevel::Low);

        // Idempotency: for idempotent tools, a duplicate call (same tool +
        // same validated input) short-circuits to the cached result instead
        // of re-executing.
        let idempotency_key = compute_idempotency_key(&call.tool_name, &validated.0);
        if matches!(
            spec.idempotency_policy,
            IdempotencyPolicy::Idempotent | IdempotencyPolicy::IdempotentWithKey
        ) {
            let cached = lock_idempotency_cache(&self.idempotency_cache)?
                .get(&idempotency_key)
                .cloned();
            if let Some(cached) = cached {
                self.record_audit(ToolAuditRecord {
                    call_id: call.id,
                    tool_name: call.tool_name.clone(),
                    started: true,
                    completed: true,
                    risk_level,
                    was_deduped: true,
                    raw_output_ref: cached.raw_output_ref.clone(),
                });
                return Ok(cached);
            }
        }

        // Permission bridge: every required permission kind must resolve to
        // `Allowed`, whether via the context's injected checker callback or
        // the static policy fallback.
        let bridge = crate::permissions::PermissionBridge::new(&self.policy);
        let decisions = bridge.check_all(&call.tool_name.0, &spec.permission_spec, context)?;
        if let Some(denied) = decisions.iter().find(|d| !d.is_allowed()) {
            self.record_audit(ToolAuditRecord {
                call_id: call.id,
                tool_name: call.tool_name.clone(),
                started: true,
                completed: false,
                risk_level,
                was_deduped: false,
                raw_output_ref: None,
            });
            return Err(SakhaError::permission(
                "sakha-tools",
                format!("tool {} denied: {:?}", call.tool_name, denied),
            ));
        }

        let _plan = tool.plan(&validated, context).await?;

        self.record_audit(ToolAuditRecord {
            call_id: call.id,
            tool_name: call.tool_name.clone(),
            started: true,
            completed: false,
            risk_level,
            was_deduped: false,
            raw_output_ref: None,
        });

        let result = tool.execute(&validated, context).await?;

        self.record_audit(ToolAuditRecord {
            call_id: call.id,
            tool_name: call.tool_name.clone(),
            started: true,
            completed: true,
            risk_level,
            was_deduped: false,
            raw_output_ref: result.raw_output_ref.clone(),
        });

        if matches!(
            spec.idempotency_policy,
            IdempotencyPolicy::Idempotent | IdempotencyPolicy::IdempotentWithKey
        ) {
            self.cache_idempotent_result(idempotency_key, result.clone());
        }

        Ok(result)
    }

    /// Inserts `result` into the idempotency cache under `key`, evicting the
    /// oldest entry first if the cache is already at
    /// `MAX_IDEMPOTENCY_CACHE_ENTRIES`, so the cache never grows without
    /// bound across a long-running session.
    fn cache_idempotent_result(&self, key: String, result: ToolResult) {
        let mut cache = lock_or_recover(&self.idempotency_cache);
        let mut order = lock_or_recover(&self.idempotency_order);

        if !cache.contains_key(&key) {
            if cache.len() >= MAX_IDEMPOTENCY_CACHE_ENTRIES {
                if let Some(oldest) = order.pop_front() {
                    cache.remove(&oldest);
                }
            }
            order.push_back(key.clone());
        }
        cache.insert(key, result);
    }

    fn record_audit(&self, record: ToolAuditRecord) {
        lock_or_recover(&self.audit_log).push(record);
    }

    /// Returns a snapshot of all audit records written so far, in order.
    pub fn audit_records(&self) -> Vec<ToolAuditRecord> {
        lock_or_recover(&self.audit_log).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins;
    use std::sync::Arc;

    fn registry_with_file_tools() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(builtins::file::FileReadTool));
        registry.register(Arc::new(builtins::file::FileWriteTool));
        registry
    }

    #[tokio::test]
    async fn unknown_tool_is_rejected() {
        let registry = ToolRegistry::new();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);
        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("does.not.exist"),
            input_json: serde_json::json!({}),
        };
        let context = ToolContext::new(".");
        let result = executor.execute(call, &context).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn permission_denied_prevents_execution() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::denied("no")));

        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.write"),
            input_json: serde_json::json!({"path": "a.txt", "content": "hello"}),
        };

        let result = executor.execute(call, &context).await;
        assert!(result.is_err());
        assert!(!dir.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn permission_allowed_permits_execution() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::allowed()));

        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.write"),
            input_json: serde_json::json!({"path": "a.txt", "content": "hello"}),
        };

        let result = executor.execute(call, &context).await.unwrap();
        assert!(matches!(result.status, crate::tool::ToolCallStatus::Succeeded));
        assert!(dir.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_before_permission_check() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        // No checker at all attached; if validation ran first and failed, we
        // never even consult the (missing) checker/policy for a decision.
        let context = ToolContext::new(dir.path());

        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.write"),
            input_json: serde_json::json!({"path": "a.txt"}), // missing "content"
        };

        let result = executor.execute(call, &context).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn duplicate_idempotent_call_is_deduped() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "hello").await.unwrap();

        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::allowed()));

        let call1 = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.read"),
            input_json: serde_json::json!({"path": "a.txt"}),
        };
        let call2 = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.read"),
            input_json: serde_json::json!({"path": "a.txt"}),
        };

        executor.execute(call1, &context).await.unwrap();
        executor.execute(call2, &context).await.unwrap();

        let records = executor.audit_records();
        // First call writes a "started" record then a "completed" record;
        // the second (deduped) call short-circuits to a single record.
        assert_eq!(records.len(), 3);
        assert!(!records[0].was_deduped);
        assert!(!records[1].was_deduped);
        assert!(records[2].was_deduped);
    }

    #[tokio::test]
    async fn audit_record_written_for_denied_call() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::denied("no")));

        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.write"),
            input_json: serde_json::json!({"path": "a.txt", "content": "hello"}),
        };

        let _ = executor.execute(call, &context).await;
        let records = executor.audit_records();
        assert_eq!(records.len(), 1);
        assert!(records[0].started);
        assert!(!records[0].completed);
    }

    #[tokio::test]
    async fn idempotency_cache_is_bounded_and_evicts_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);
        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::allowed()));

        // Insert more entries than the cache can hold; this must not grow
        // the cache without bound, and must not panic.
        for i in 0..(super::MAX_IDEMPOTENCY_CACHE_ENTRIES + 10) {
            let filename = format!("f{i}.txt");
            tokio::fs::write(dir.path().join(&filename), "x").await.unwrap();
            let call = ToolCall {
                id: ToolCallId::new(),
                tool_name: ToolName::new("file.read"),
                input_json: serde_json::json!({"path": filename}),
            };
            executor.execute(call, &context).await.unwrap();
        }

        let cache_len = lock_or_recover(&executor.idempotency_cache).len();
        assert!(cache_len <= super::MAX_IDEMPOTENCY_CACHE_ENTRIES);
    }

    #[tokio::test]
    async fn poisoned_audit_log_mutex_does_not_panic_subsequent_calls() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_with_file_tools();
        let policy = PermissionPolicy::new();
        let executor = ToolExecutor::new(registry, policy);

        // Poison the audit_log mutex by panicking while holding the lock.
        let poison_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = executor.audit_log.lock().unwrap();
            panic!("simulated panic while holding the audit log lock");
        }));
        assert!(poison_result.is_err());
        assert!(executor.audit_log.is_poisoned());

        // A subsequent call must still return normally (Err or Ok), never panic.
        let context = ToolContext::new(dir.path())
            .with_permission_checker(Arc::new(|_req| sakha_security::PermissionDecision::allowed()));
        let call = ToolCall {
            id: ToolCallId::new(),
            tool_name: ToolName::new("file.write"),
            input_json: serde_json::json!({"path": "a.txt", "content": "hello"}),
        };
        let result = executor.execute(call, &context).await;
        assert!(result.is_ok());
    }
}
