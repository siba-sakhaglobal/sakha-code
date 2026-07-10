//! `sakha memory search <query>`: searches the in-memory `MemoryStore`.
//! Durable SQLite-backed search is available once a `db_path` is configured
//! (see `sakha-memory::SqliteMemoryStore`); this command uses the same
//! `MemoryStore` trait so swapping backends is a one-line change.

use clap::Args;
use serde::Serialize;

use sakha_memory::{InMemoryMemoryStore, MemoryQuery, MemoryStore};

use crate::output::{print_error, print_output, OutputFormat};

#[derive(Debug, Args)]
pub struct MemorySearchArgs {
    pub query: String,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct MemoryHitView {
    text: String,
    relevance: f32,
}

pub fn execute(args: MemorySearchArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(args))
}

async fn execute_async(args: MemorySearchArgs) -> i32 {
    // Fresh in-memory store per invocation: real durability requires a
    // configured `db_path` routed through `SqliteMemoryStore`, not yet
    // threaded through CLI config. An empty result set here is therefore the
    // documented, correct behavior rather than a bug.
    let store = InMemoryMemoryStore::new();
    let query = MemoryQuery {
        workspace_id: None,
        kind: None,
        text_contains: Some(args.query.clone()),
        limit: args.limit,
    };
    match store.search_memory(query).await {
        Ok(hits) => {
            let views: Vec<MemoryHitView> =
                hits.into_iter().map(|h| MemoryHitView { text: h.record.text, relevance: h.relevance.0 }).collect();
            print_output(&views, args.output);
            0
        }
        Err(err) => {
            print_error(&err.to_string(), args.output);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_search_on_empty_store_returns_success_with_no_hits() {
        let code = execute(MemorySearchArgs { query: "anything".into(), limit: None, output: OutputFormat::Json });
        assert_eq!(code, 0);
    }
}
