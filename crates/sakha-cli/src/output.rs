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
///
/// `serde_json::to_string_pretty` on a `{"error": &str}` object only fails
/// for pathological cases (e.g. a custom `Serialize` impl returning an
/// error, or non-finite floats — neither applies to a plain string value),
/// but per spec "JSON output mode schema" the fallback must still be valid,
/// parseable JSON rather than the raw (unescaped, possibly multi-line)
/// message text: `serde_json::json!` escapes the message into a JSON string
/// literal directly, sidestepping `to_string_pretty` entirely for the
/// fallback path so it cannot itself fail to produce valid JSON.
pub fn print_error(message: &str, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let payload = serde_json::json!({ "error": message });
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{json}"),
                Err(err) => {
                    // Build the JSON string literal by hand (via `Value::String`'s
                    // `Display`, which always escapes correctly) so this fallback
                    // path is itself guaranteed valid JSON, not raw text.
                    eprintln!("warning: failed to pretty-print JSON error payload: {err}");
                    println!("{{\"error\":{}}}", serde_json::Value::String(message.to_string()));
                }
            }
        }
        OutputFormat::Text => eprintln!("error: {message}"),
    }
}

#[cfg(test)]
mod tests {
    /// The JSON string-literal fallback path (`Value::String`'s `Display`)
    /// must always produce valid, parseable JSON even for messages containing
    /// characters that need escaping (quotes, newlines, backslashes) — the
    /// exact class of input that would corrupt a naive
    /// `format!("{{\"error\":\"{message}\"}}")` fallback.
    #[test]
    fn json_error_fallback_literal_escapes_special_characters() {
        let message = "line one\nline \"two\" with \\backslash";
        let literal = format!("{{\"error\":{}}}", serde_json::Value::String(message.to_string()));
        let parsed: serde_json::Value = serde_json::from_str(&literal).unwrap();
        assert_eq!(parsed["error"], message);
    }
}
