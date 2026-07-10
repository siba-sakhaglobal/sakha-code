//! `sakha providers list`: shows the configured provider and its reported
//! capabilities/health.

use clap::Args;
use serde::Serialize;

use crate::config::default_config_path;
use crate::output::{print_error, print_output, OutputFormat};
use crate::runtime::build_provider;

#[derive(Debug, Args)]
pub struct ProvidersListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct ProviderView {
    selection: String,
    model: String,
    base_url: String,
    health: String,
    capabilities: sakha_provider::ProviderCapabilities,
}

pub fn execute(args: ProvidersListArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(args))
}

async fn execute_async(args: ProvidersListArgs) -> i32 {
    let config = match crate::config::load_config(&default_config_path()) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };
    let provider = build_provider(&config);
    let health = provider.health().await.map(|h| format!("{h:?}")).unwrap_or_else(|e| e.to_string());

    let view = ProviderView {
        selection: format!("{:?}", config.provider.selection),
        model: config.provider.model.clone(),
        base_url: config.provider.base_url.clone(),
        health,
        capabilities: provider.capabilities(),
    };
    print_output(&vec![view], args.output);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_list_reports_configured_provider() {
        let code = execute(ProvidersListArgs { output: OutputFormat::Json });
        assert_eq!(code, 0);
    }
}
