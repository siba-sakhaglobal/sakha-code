//! `sakha tools list`: lists the built-in tool registry.

use clap::Args;
use serde::Serialize;

use sakha_tools::ToolSpec;

use crate::output::{print_output, OutputFormat};
use crate::runtime::build_tool_registry;

#[derive(Debug, Args)]
pub struct ToolsListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct ToolView {
    name: String,
    description: String,
    idempotency_policy: String,
}

impl From<ToolSpec> for ToolView {
    fn from(spec: ToolSpec) -> Self {
        Self {
            name: spec.name.0,
            description: spec.description,
            idempotency_policy: format!("{:?}", spec.idempotency_policy),
        }
    }
}

pub fn execute(args: ToolsListArgs) -> i32 {
    let registry = build_tool_registry();
    let mut views: Vec<ToolView> = registry.specs().into_iter().map(ToolView::from).collect();
    views.sort_by(|a, b| a.name.cmp(&b.name));
    print_output(&views, args.output);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_includes_builtins() {
        let registry = build_tool_registry();
        let views: Vec<ToolView> = registry.specs().into_iter().map(ToolView::from).collect();
        assert!(views.iter().any(|v| v.name == "file.read"));
        assert!(views.iter().any(|v| v.name == "shell.run"));
    }

    #[test]
    fn execute_returns_success() {
        assert_eq!(execute(ToolsListArgs { output: OutputFormat::Json }), 0);
    }
}
