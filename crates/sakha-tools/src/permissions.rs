//! Bridges tool permission specs to a permission decision, via either an
//! injected `ToolContext::permission_checker` callback (preferred — lets the
//! host implement interactive approval, allow/deny lists, or test fixtures)
//! or a static `sakha_security::PermissionPolicy` fallback.

use sakha_core::SakhaResult;
use sakha_security::{PermissionDecision, PermissionPolicy, PermissionRequest};

use crate::tool::{PermissionCheckRequest, ToolContext, ToolPermissionSpec};

/// Evaluates a tool's declared `ToolPermissionSpec` against either the
/// `ToolContext`'s injected checker callback or a `PermissionPolicy`,
/// producing one decision per required permission kind. The executor
/// requires `PermissionDecision::Allowed` for every entry before execution.
pub struct PermissionBridge<'a> {
    pub policy: &'a PermissionPolicy,
}

impl<'a> PermissionBridge<'a> {
    pub fn new(policy: &'a PermissionPolicy) -> Self {
        Self { policy }
    }

    pub fn check_all(
        &self,
        tool_name: &str,
        spec: &ToolPermissionSpec,
        context: &ToolContext,
    ) -> SakhaResult<Vec<PermissionDecision>> {
        spec.required
            .iter()
            .map(|kind| {
                if let Some(checker) = &context.permission_checker {
                    let request = PermissionCheckRequest {
                        kind: *kind,
                        tool_name: tool_name.to_string(),
                        subject: tool_name.to_string(),
                        reason: "tool execution".to_string(),
                    };
                    Ok(checker(&request))
                } else {
                    let mut request = PermissionRequest::new(*kind, tool_name, "tool execution");
                    request.tool_name = Some(tool_name.to_string());
                    self.policy.check(&request)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sakha_security::PermissionKind;
    use std::sync::Arc;

    #[test]
    fn checker_callback_takes_precedence_over_policy() {
        let policy = PermissionPolicy::new();
        let bridge = PermissionBridge::new(&policy);

        let spec = ToolPermissionSpec {
            required: vec![PermissionKind::FileWrite],
        };

        let context = ToolContext::new(".").with_permission_checker(Arc::new(|_req| {
            PermissionDecision::denied("test: always deny")
        }));

        let decisions = bridge.check_all("file.write", &spec, &context).unwrap();
        assert_eq!(decisions.len(), 1);
        assert!(!decisions[0].is_allowed());
    }

    #[test]
    fn checker_callback_can_allow() {
        let policy = PermissionPolicy::new();
        let bridge = PermissionBridge::new(&policy);

        let spec = ToolPermissionSpec {
            required: vec![PermissionKind::FileRead],
        };

        let context =
            ToolContext::new(".").with_permission_checker(Arc::new(|_req| PermissionDecision::allowed()));

        let decisions = bridge.check_all("file.read", &spec, &context).unwrap();
        assert!(decisions[0].is_allowed());
    }

    #[test]
    fn falls_back_to_policy_when_no_checker_present() {
        let policy = PermissionPolicy::new();
        let bridge = PermissionBridge::new(&policy);
        let spec = ToolPermissionSpec {
            required: vec![PermissionKind::FileRead],
        };
        let context = ToolContext::new(".");
        // With no checker attached, the bridge falls back to
        // `PermissionPolicy::check`. An empty policy auto-allows low-risk
        // requests (like FileRead) via its risk-classifier fallback.
        let decisions = bridge.check_all("file.read", &spec, &context).unwrap();
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].is_allowed());
    }

    #[test]
    fn falls_back_to_policy_and_denies_via_explicit_rule() {
        let policy = PermissionPolicy::new().with_layer(sakha_security::PolicyLayer {
            source: Some(sakha_security::PolicySource::WorkspaceLocal),
            deny_rules: vec!["file.write".into()],
            ..Default::default()
        });
        let bridge = PermissionBridge::new(&policy);
        let spec = ToolPermissionSpec {
            required: vec![PermissionKind::FileWrite],
        };
        let context = ToolContext::new(".");
        let decisions = bridge.check_all("file.write", &spec, &context).unwrap();
        assert!(!decisions[0].is_allowed());
    }
}
