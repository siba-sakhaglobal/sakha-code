//! Trust model for MCP servers/plugins. See spec "Trust Model".

use serde::{Deserialize, Serialize};

/// Trust tiers assigned to MCP servers/plugins. See spec "Trust Model".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Blocked,
    MarketplaceUnverified,
    Marketplace,
    OrganizationApproved,
    Workspace,
    LocalDev,
}

/// A policy deciding whether a given trust level may proceed for a given
/// action (tool call, resource read).
#[derive(Debug, Clone, Default)]
pub struct TrustPolicy {
    pub minimum_trust_level: Option<TrustLevel>,
}

impl TrustPolicy {
    pub fn new(minimum_trust_level: TrustLevel) -> Self {
        Self { minimum_trust_level: Some(minimum_trust_level) }
    }

    /// Returns true if `level` satisfies the configured minimum.
    pub fn allows(&self, level: TrustLevel) -> bool {
        match self.minimum_trust_level {
            Some(minimum) => level >= minimum,
            None => true,
        }
    }
}
