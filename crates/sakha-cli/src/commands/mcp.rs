//! `sakha mcp add`: registers an MCP server into a local registry and prints
//! its resolved config. Persistence across invocations follows once MCP
//! server configs are folded into `SakhaConfig`/`~/.sakha/config.toml`;
//! today this validates and echoes the registration against the real
//! `sakha-mcp::McpServerRegistry` type.

use clap::Args;
use serde::Serialize;

use sakha_mcp::{McpServerConfig, McpServerRegistry, TrustLevel};

use crate::output::{print_error, print_output, OutputFormat};

#[derive(Debug, Args)]
pub struct McpAddArgs {
    /// Unique name for this MCP server.
    pub name: String,

    /// Command to launch a stdio MCP server (mutually exclusive with `--url`).
    #[arg(long)]
    pub command: Option<String>,

    /// Args passed to `--command`.
    #[arg(long)]
    pub arg: Vec<String>,

    /// URL for a remote MCP server (mutually exclusive with `--command`).
    #[arg(long)]
    pub url: Option<String>,

    #[arg(long, value_enum, default_value = "workspace")]
    pub trust: TrustArg,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TrustArg {
    Blocked,
    MarketplaceUnverified,
    Marketplace,
    OrganizationApproved,
    Workspace,
    LocalDev,
}

impl From<TrustArg> for TrustLevel {
    fn from(value: TrustArg) -> Self {
        match value {
            TrustArg::Blocked => TrustLevel::Blocked,
            TrustArg::MarketplaceUnverified => TrustLevel::MarketplaceUnverified,
            TrustArg::Marketplace => TrustLevel::Marketplace,
            TrustArg::OrganizationApproved => TrustLevel::OrganizationApproved,
            TrustArg::Workspace => TrustLevel::Workspace,
            TrustArg::LocalDev => TrustLevel::LocalDev,
        }
    }
}

#[derive(Debug, Serialize)]
struct McpServerView {
    name: String,
    command: Option<String>,
    args: Vec<String>,
    url: Option<String>,
    trust_level: String,
}

pub fn execute(args: McpAddArgs) -> i32 {
    if args.command.is_none() && args.url.is_none() {
        print_error("either --command or --url must be provided", args.output);
        return 2;
    }

    let config = McpServerConfig {
        name: args.name.clone(),
        command: args.command.clone(),
        args: args.arg.clone(),
        url: args.url.clone(),
        trust_level: args.trust.into(),
    };

    let mut registry = McpServerRegistry::new();
    registry.register(config);

    match registry.get(&args.name) {
        Some(cfg) => {
            print_output(
                &McpServerView {
                    name: cfg.name.clone(),
                    command: cfg.command.clone(),
                    args: cfg.args.clone(),
                    url: cfg.url.clone(),
                    trust_level: format!("{:?}", cfg.trust_level),
                },
                args.output,
            );
            println!("note: MCP registry is per-invocation until persisted config is wired (see sakha-cli src/commands/mcp.rs)");
            0
        }
        None => {
            print_error("failed to register MCP server", args.output);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_add_with_command_succeeds() {
        let code = execute(McpAddArgs {
            name: "local-fs".into(),
            command: Some("mcp-server-fs".into()),
            arg: vec!["--root".into(), ".".into()],
            url: None,
            trust: TrustArg::LocalDev,
            output: OutputFormat::Json,
        });
        assert_eq!(code, 0);
    }

    #[test]
    fn mcp_add_without_command_or_url_fails() {
        let code = execute(McpAddArgs {
            name: "broken".into(),
            command: None,
            arg: vec![],
            url: None,
            trust: TrustArg::Workspace,
            output: OutputFormat::Json,
        });
        assert_eq!(code, 2);
    }
}
