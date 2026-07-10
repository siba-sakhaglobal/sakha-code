//! `OutputFormat`: shared output rendering mode for all CLI subcommands.

use clap::ValueEnum;
use serde::Serialize;

/// Output rendering mode, selectable via `--output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

impl Default for OutputFormat {
    fn default() -> Self {
        OutputFormat::Text
    }
}

/// Renders `value` per `format`, printing to stdout. JSON mode is intended
/// to be machine-parseable and schema-stable per spec "JSON output mode schema".
pub fn print_output<T: Serialize + std::fmt::Debug>(value: &T, format: OutputFormat) {
    match format {
        OutputFormat::Json => match serde_json::to_string_pretty(value) {
            Ok(json) => println!("{json}"),
            Err(err) => eprintln!("failed to serialize output: {err}"),
        },
        OutputFormat::Text => println!("{value:?}"),
    }
}

/// Renders `value` in JSON mode, or calls `text_fn` to print a human-friendly
/// rendering in text mode. Use this when the text rendering should not be the
/// `{value:?}` debug form (e.g. tables, prose). Not yet called by a built-in
/// subcommand (they currently build their own text/JSON views inline) but is
/// part of this module's public output contract for future subcommands.
#[allow(dead_code)]
pub fn print_with<T: Serialize>(value: &T, format: OutputFormat, text_fn: impl FnOnce(&T)) {
    match format {
        OutputFormat::Json => match serde_json::to_string_pretty(value) {
            Ok(json) => println!("{json}"),
            Err(err) => eprintln!("failed to serialize output: {err}"),
        },
        OutputFormat::Text => text_fn(value),
    }
}

/// Prints an error consistently across subcommands: JSON mode emits a
/// `{"error": "..."}` object on stdout (kept machine-parseable even for
/// errors), text mode writes to stderr.
pub fn print_error(message: &str, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({ "error": message });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_else(|_| message.to_string()));
        }
        OutputFormat::Text => eprintln!("error: {message}"),
    }
}
