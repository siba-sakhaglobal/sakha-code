//! `sakha search`, `sakha fetch`, `sakha search-backends`: direct terminal
//! access to the web search/fetch pool without going through the agent
//! loop. See `docs/web-search.md`.

use clap::Args;
use serde::Serialize;

use crate::config::{default_config_path, load_config};
use crate::output::{print_error, print_output, OutputFormat};
use crate::runtime::{build_fetch_pool, build_search_pool};

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// The search query.
    pub query: String,

    /// Max number of hits to return (1-10).
    #[arg(long, default_value_t = 5)]
    pub limit: u32,

    /// Restrict to a single backend id (e.g. `brave`), skipping the
    /// priority/failover chain.
    #[arg(long)]
    pub backend: Option<String>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct FetchArgs {
    /// The URL to fetch as clean markdown.
    pub url: String,

    /// Truncate output to this many characters.
    #[arg(long, default_value_t = 12_000)]
    pub max_chars: usize,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SearchBackendsArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct SearchHitView {
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Serialize)]
struct SearchOutputView {
    backend: String,
    hits: Vec<SearchHitView>,
}

pub fn execute_search(args: SearchArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            print_error(&format!("failed to start async runtime: {err}"), args.output);
            return 1;
        }
    };
    rt.block_on(execute_search_async(args))
}

async fn execute_search_async(args: SearchArgs) -> i32 {
    let config = match load_config(&default_config_path()) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };

    let limit = args.limit.clamp(1, 10);
    let pool = build_search_pool(&config);

    let result = if let Some(backend_id) = &args.backend {
        pool.search_with_backend(backend_id, &args.query, limit).await
    } else {
        pool.search(&args.query, limit).await
    };

    match result {
        Ok(result) => {
            let view = SearchOutputView {
                backend: result.backend,
                hits: result.hits.into_iter().map(|h| SearchHitView { title: h.title, url: h.url, snippet: h.snippet }).collect(),
            };
            match args.output {
                OutputFormat::Json => print_output(&view, args.output),
                OutputFormat::Text => {
                    println!("backend: {}", view.backend);
                    println!("{:<60} {:<40} SNIPPET", "URL", "TITLE");
                    for hit in &view.hits {
                        println!("{:<60} {:<40} {}", hit.url, hit.title, hit.snippet);
                    }
                }
            }
            0
        }
        Err(err) => {
            print_error(&err.to_string(), args.output);
            1
        }
    }
}

pub fn execute_fetch(args: FetchArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            print_error(&format!("failed to start async runtime: {err}"), args.output);
            return 1;
        }
    };
    rt.block_on(execute_fetch_async(args))
}

async fn execute_fetch_async(args: FetchArgs) -> i32 {
    let pool = build_fetch_pool();
    match pool.fetch(&args.url, args.max_chars).await {
        Ok(outcome) => {
            match args.output {
                OutputFormat::Json => {
                    let payload = serde_json::json!({
                        "url": args.url,
                        "backend": outcome.backend,
                        "content_markdown": outcome.content_markdown,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                }
                OutputFormat::Text => {
                    println!("{}", outcome.content_markdown);
                }
            }
            0
        }
        Err(err) => {
            print_error(&err.to_string(), args.output);
            1
        }
    }
}

#[derive(Debug, Serialize)]
struct BackendStatusView {
    id: String,
    env_var: String,
    configured: bool,
    cooling_down: bool,
    key_portal_url: String,
}

pub fn execute_search_backends(args: SearchBackendsArgs) -> i32 {
    let config = match load_config(&default_config_path()) {
        Ok(c) => c,
        Err(err) => {
            print_error(&format!("failed to load config: {err}"), args.output);
            return 2;
        }
    };

    let pool = build_search_pool(&config);
    let views: Vec<BackendStatusView> = pool
        .statuses()
        .into_iter()
        .map(|s| BackendStatusView {
            id: s.id.to_string(),
            env_var: s.env_var.unwrap_or("-").to_string(),
            configured: s.configured,
            cooling_down: s.cooling_down,
            key_portal_url: s.key_portal_url.to_string(),
        })
        .collect();

    match args.output {
        OutputFormat::Json => print_output(&views, args.output),
        OutputFormat::Text => {
            println!("{:<10} {:<20} {:<11} {:<8} KEY PORTAL", "ID", "ENV VAR", "CONFIGURED", "COOLING");
            for v in &views {
                println!(
                    "{:<10} {:<20} {:<11} {:<8} {}",
                    v.id,
                    v.env_var,
                    if v.configured { "yes" } else { "no" },
                    if v.cooling_down { "yes" } else { "no" },
                    v.key_portal_url,
                );
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_args_parse_query_and_defaults() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: SearchArgs,
        }
        let cli = TestCli::try_parse_from(["sakha", "rust tokio tutorial"]).unwrap();
        assert_eq!(cli.args.query, "rust tokio tutorial");
        assert_eq!(cli.args.limit, 5);
        assert!(cli.args.backend.is_none());
    }

    #[test]
    fn search_args_parse_limit_and_backend_overrides() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: SearchArgs,
        }
        let cli = TestCli::try_parse_from(["sakha", "query", "--limit", "3", "--backend", "brave"]).unwrap();
        assert_eq!(cli.args.limit, 3);
        assert_eq!(cli.args.backend.as_deref(), Some("brave"));
    }

    #[test]
    fn fetch_args_parse_url_and_default_max_chars() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: FetchArgs,
        }
        let cli = TestCli::try_parse_from(["sakha", "https://example.com"]).unwrap();
        assert_eq!(cli.args.url, "https://example.com");
        assert_eq!(cli.args.max_chars, 12_000);
    }

    #[test]
    fn search_backends_reports_all_known_backends() {
        let _home = crate::test_support::TempHome::new();
        let code = execute_search_backends(SearchBackendsArgs { output: OutputFormat::Json });
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn search_with_unconfigured_backend_reports_error() {
        let _home = crate::test_support::TempHome::new();
        std::env::remove_var("BRAVE_API_KEY");
        // Calls the `_async` entry point directly (not `execute_search`,
        // which spins up its own `tokio::runtime::Runtime` via `block_on` —
        // doing that from inside a `#[tokio::test]` async fn panics with
        // "Cannot start a runtime from within a runtime").
        let code =
            execute_search_async(SearchArgs { query: "test".into(), limit: 3, backend: Some("brave".into()), output: OutputFormat::Json })
                .await;
        // brave is not configured without BRAVE_API_KEY, so this must fail
        // cleanly (exit 1), never panic.
        assert_eq!(code, 1);
    }
}
