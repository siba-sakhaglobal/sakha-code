//! Policy sources and merge order for `PermissionPolicy`. See spec
//! `modules/14-security-sandbox-permissions.md` "Policy Sources".
//!
//! Priority order (highest first): CLI flags, workspace local policy, project
//! policy, user policy, organization policy, built-in defaults. Rules from
//! higher-priority sources are evaluated first, so an explicit allow/deny at
//! a higher-priority layer always wins over a lower-priority layer's rule for
//! the same request.

use serde::{Deserialize, Serialize};

use crate::glob::glob_match;
use crate::permissions::{PermissionDecision, PermissionRequest};
use crate::risk::{RiskClassifier, RiskLevel};
use crate::sandbox::SandboxProfile;
use sakha_core::SakhaError;

/// Where a policy layer originated from. Priority order (highest first):
/// CLI flags, workspace local policy, project policy, user policy,
/// organization policy, built-in defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    BuiltinDefault,
    Organization,
    User,
    Project,
    WorkspaceLocal,
    CliFlag,
}

impl PolicySource {
    /// Priority rank: higher value wins. `CliFlag` is highest.
    pub fn priority(&self) -> u8 {
        match self {
            PolicySource::BuiltinDefault => 0,
            PolicySource::Organization => 1,
            PolicySource::User => 2,
            PolicySource::Project => 3,
            PolicySource::WorkspaceLocal => 4,
            PolicySource::CliFlag => 5,
        }
    }
}

/// A single glob rule: matches against `PermissionRequest::rule_key()`
/// (`"<namespace>:<subject>"`, e.g. `"shell.run_arbitrary:rm -rf *"`) or bare
/// namespace globs (e.g. `"file.*"`).
pub type Rule = String;

/// A single layer of policy configuration, mergeable with others by priority.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyLayer {
    pub source: Option<PolicySource>,
    pub allow_rules: Vec<Rule>,
    pub deny_rules: Vec<Rule>,
    /// Rules that require explicit human approval even though not denied.
    #[serde(default)]
    pub ask_rules: Vec<Rule>,
    pub default_sandbox: Option<SandboxProfile>,
}

impl PolicyLayer {
    pub fn priority(&self) -> u8 {
        self.source.unwrap_or(PolicySource::BuiltinDefault).priority()
    }
}

/// The merged permission policy used to evaluate tool/action requests.
#[derive(Debug, Clone, Default)]
pub struct PermissionPolicy {
    pub layers: Vec<PolicyLayer>,
    /// Below this risk level, an action with no matching rule at any layer is
    /// auto-allowed. At or above it, an unmatched action requires approval.
    pub auto_allow_below: RiskLevel,
}

impl PermissionPolicy {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            auto_allow_below: RiskLevel::Medium,
        }
    }

    /// Adds a policy layer; layers are merged in `PolicySource` priority order
    /// at evaluation time, not insertion order.
    pub fn with_layer(mut self, layer: PolicyLayer) -> Self {
        self.layers.push(layer);
        self
    }

    /// Sets the risk threshold below which unmatched requests auto-allow.
    pub fn with_auto_allow_below(mut self, level: RiskLevel) -> Self {
        self.auto_allow_below = level;
        self
    }

    /// Merges all layers into an effective, ordered rule set (highest
    /// priority source wins on conflicting allow/deny for the same rule).
    /// Kept for inspection/debugging and backward compatibility; simply
    /// concatenates every layer's rules in priority order without collapsing
    /// duplicates.
    pub fn merged(&self) -> PolicyLayer {
        let mut layers = self.layers.clone();
        layers.sort_by(|a, b| a.priority().cmp(&b.priority()));
        let mut merged = PolicyLayer::default();
        for layer in layers {
            merged.allow_rules.extend(layer.allow_rules);
            merged.deny_rules.extend(layer.deny_rules);
            merged.ask_rules.extend(layer.ask_rules);
            if layer.default_sandbox.is_some() {
                merged.default_sandbox = layer.default_sandbox;
            }
        }
        merged
    }

    /// Resolves the effective default sandbox profile: highest-priority layer
    /// that declares one wins.
    pub fn effective_sandbox(&self) -> Option<SandboxProfile> {
        let mut layers: Vec<&PolicyLayer> = self.layers.iter().collect();
        layers.sort_by(|a, b| b.priority().cmp(&a.priority()));
        layers.iter().find_map(|l| l.default_sandbox)
    }

    /// Evaluates a permission request against the merged policy.
    ///
    /// Resolution order:
    /// 1. Walk merged rules highest-priority-first; the first rule (deny, ask,
    ///    or allow) whose glob matches the request's rule key or bare
    ///    namespace wins outright, regardless of disposition ordering within
    ///    that same layer — deny is checked before ask before allow *within*
    ///    a layer so a single layer can't accidentally allow something it
    ///    also denies.
    /// 2. If no rule matches anywhere, fall back to the risk classifier: risk
    ///    below `auto_allow_below` auto-allows; otherwise the request needs
    ///    human approval.
    pub fn check(&self, request: &PermissionRequest) -> Result<PermissionDecision, SakhaError> {
        let rule_key = request.rule_key();
        let namespace = request.kind.rule_namespace();

        let mut layers: Vec<&PolicyLayer> = self.layers.iter().collect();
        layers.sort_by(|a, b| b.priority().cmp(&a.priority()));

        for layer in layers {
            if let Some(m) = Self::first_match(&layer.deny_rules, &rule_key, namespace) {
                return Ok(PermissionDecision::denied(format!(
                    "denied by {:?} policy rule '{}'",
                    layer.source.unwrap_or(PolicySource::BuiltinDefault),
                    m
                )));
            }
            if let Some(m) = Self::first_match(&layer.ask_rules, &rule_key, namespace) {
                return Ok(PermissionDecision::needs_approval(format!(
                    "requires approval per {:?} policy rule '{}'",
                    layer.source.unwrap_or(PolicySource::BuiltinDefault),
                    m
                )));
            }
            if let Some(m) = Self::first_match(&layer.allow_rules, &rule_key, namespace) {
                return Ok(PermissionDecision::allowed_with_note(format!(
                    "allowed by {:?} policy rule '{}'",
                    layer.source.unwrap_or(PolicySource::BuiltinDefault),
                    m
                )));
            }
        }

        // No layer matched: fall back to risk-based default.
        let risk = RiskClassifier::new().classify(request);
        if risk < self.auto_allow_below {
            Ok(PermissionDecision::Allowed)
        } else {
            Ok(PermissionDecision::needs_approval(format!(
                "no policy rule matched; risk level {:?} requires approval",
                risk
            )))
        }
    }

    fn first_match<'a>(rules: &'a [Rule], rule_key: &str, namespace: &str) -> Option<&'a str> {
        rules
            .iter()
            .find(|r| glob_match(r, rule_key) || glob_match(r, namespace))
            .map(|s| s.as_str())
    }
}

impl PermissionDecision {
    /// Allowed, but keep the reason around for audit trails (not part of the
    /// public equality contract — `PermissionDecision::Allowed` still
    /// compares equal to this since we intentionally don't attach a reason
    /// field to the `Allowed` variant to keep `is_allowed()` simple).
    fn allowed_with_note(_reason: impl Into<String>) -> Self {
        PermissionDecision::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::PermissionKind;

    #[test]
    fn higher_priority_policy_overrides_lower() {
        let policy = PermissionPolicy::new()
            .with_layer(PolicyLayer {
                source: Some(PolicySource::BuiltinDefault),
                allow_rules: vec!["file.read".into()],
                ..Default::default()
            })
            .with_layer(PolicyLayer {
                source: Some(PolicySource::CliFlag),
                deny_rules: vec!["file.read".into()],
                ..Default::default()
            });
        let merged = policy.merged();
        assert!(merged.allow_rules.contains(&"file.read".to_string()));
        assert!(merged.deny_rules.contains(&"file.read".to_string()));

        // And evaluation actually resolves to the higher-priority (CliFlag) deny.
        let req = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        let decision = policy.check(&req).unwrap();
        assert!(decision.is_denied());
    }

    #[test]
    fn workspace_overrides_user() {
        let policy = PermissionPolicy::new()
            .with_layer(PolicyLayer {
                source: Some(PolicySource::User),
                deny_rules: vec!["shell.*".into()],
                ..Default::default()
            })
            .with_layer(PolicyLayer {
                source: Some(PolicySource::WorkspaceLocal),
                allow_rules: vec!["shell.run_safe".into()],
                ..Default::default()
            });
        let req = PermissionRequest::new(PermissionKind::ShellRunSafe, "echo hi", "run");
        let decision = policy.check(&req).unwrap();
        assert!(decision.is_allowed());
    }

    #[test]
    fn unmatched_low_risk_auto_allows() {
        let policy = PermissionPolicy::new();
        let req = PermissionRequest::new(PermissionKind::FileRead, "a.txt", "read");
        let decision = policy.check(&req).unwrap();
        assert!(decision.is_allowed());
    }

    #[test]
    fn unmatched_high_risk_needs_approval() {
        let policy = PermissionPolicy::new();
        let req = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /", "run");
        let decision = policy.check(&req).unwrap();
        assert!(decision.needs_human_approval());
    }

    #[test]
    fn glob_rule_matches_command_subject() {
        let policy = PermissionPolicy::new().with_layer(PolicyLayer {
            source: Some(PolicySource::WorkspaceLocal),
            deny_rules: vec!["shell.run_arbitrary:rm -rf *".into()],
            ..Default::default()
        });
        let req = PermissionRequest::new(PermissionKind::ShellRunArbitrary, "rm -rf /home", "run");
        let decision = policy.check(&req).unwrap();
        assert!(decision.is_denied());
    }
}
