//! Streaming tool-call assembly: providers stream tool call arguments as
//! fragmented deltas that must be assembled into complete calls before
//! dispatch to the tool runtime.

use serde::{Deserialize, Serialize};

/// A single fragment of a tool call as streamed by a provider. `index`
/// identifies which parallel tool call this delta belongs to within one
/// streamed response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    /// Partial JSON-encoded arguments; must be concatenated across deltas
    /// with the same `index` before parsing.
    pub arguments_fragment: String,
    /// Provider-specific opaque payload attached to the tool call (e.g.
    /// Gemini's `extra_content.google.thought_signature`). Preserved verbatim
    /// and echoed back when the assistant turn is replayed — some providers
    /// hard-reject follow-up requests without it.
    #[serde(default)]
    pub extra_content: Option<serde_json::Value>,
}

/// A fully assembled tool call, ready for schema validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
    /// See `ToolCallDelta::extra_content` — carried through assembly so the
    /// request builder can echo it back to the provider.
    #[serde(default)]
    pub extra_content: Option<serde_json::Value>,
}

/// Accumulates `ToolCallDelta`s by index and assembles them into complete
/// tool calls once a response finishes.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    partials: std::collections::BTreeMap<u32, Partial>,
}

#[derive(Debug, Default)]
struct Partial {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    extra_content: Option<serde_json::Value>,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, delta: ToolCallDelta) {
        // Some providers (e.g. Gemini's OpenAI-compat layer) omit the
        // per-call `index` and stream each tool call complete with a unique
        // `id` — under the OpenAI default of `index = 0`, multiple calls
        // would merge into one entry and their argument JSON would
        // concatenate into garbage. If a delta carries an id that differs
        // from the id already occupying its index, treat it as a NEW call at
        // the next free index. (Providers that do send proper indexes never
        // trigger this: their ids only appear on the first fragment of each
        // distinct index.)
        let mut index = delta.index;
        if let Some(new_id) = &delta.id {
            if let Some(existing) = self.partials.get(&index) {
                if existing.id.as_deref().is_some_and(|id| id != new_id) {
                    index = self.partials.keys().next_back().copied().unwrap_or(0) + 1;
                }
            }
        }
        let entry = self.partials.entry(index).or_default();
        if delta.id.is_some() {
            entry.id = delta.id;
        }
        if delta.name.is_some() {
            entry.name = delta.name;
        }
        if delta.extra_content.is_some() {
            entry.extra_content = delta.extra_content;
        }
        entry.arguments.push_str(&delta.arguments_fragment);
    }

    /// Finalizes accumulated deltas into assembled tool calls, in index order.
    /// Entries missing an `id` or `name` are skipped (malformed stream).
    pub fn finish(self) -> Vec<AssembledToolCall> {
        self.partials
            .into_values()
            .filter_map(|p| match (p.id, p.name) {
                (Some(id), Some(name)) => Some(AssembledToolCall {
                    id,
                    name,
                    arguments_json: p.arguments,
                    extra_content: p.extra_content,
                }),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_call(id: &str, name: &str, args: &str) -> ToolCallDelta {
        ToolCallDelta {
            index: 0,
            id: Some(id.to_string()),
            name: Some(name.to_string()),
            arguments_fragment: args.to_string(),
            extra_content: None,
        }
    }

    /// Gemini-compat regression: multiple complete tool calls all streamed
    /// with `index = 0` (no per-call index) must assemble into distinct
    /// calls, not merge into one entry with concatenated argument JSON.
    #[test]
    fn indexless_parallel_calls_do_not_merge() {
        let mut assembler = ToolCallAssembler::new();
        assembler.push(complete_call("call_a", "file.write", r#"{"path":"a.txt","content":"1"}"#));
        assembler.push(complete_call("call_b", "file.write", r#"{"path":"b.txt","content":"2"}"#));
        assembler.push(complete_call("call_c", "file.read", r#"{"path":"c.txt"}"#));

        let calls = assembler.finish();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[1].id, "call_b");
        assert_eq!(calls[2].id, "call_c");
        for call in &calls {
            assert!(serde_json::from_str::<serde_json::Value>(&call.arguments_json).unwrap().is_object());
        }
    }

    /// OpenAI-style fragmented streaming (id only on the first fragment of
    /// each distinct index) must keep concatenating fragments as before.
    #[test]
    fn fragmented_calls_with_proper_indexes_still_concatenate() {
        let mut assembler = ToolCallAssembler::new();
        assembler.push(ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            name: Some("calc".into()),
            arguments_fragment: r#"{"expr":"#.to_string(),
            extra_content: None,
        });
        assembler.push(ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments_fragment: r#""2+2"}"#.to_string(),
            extra_content: None,
        });

        let calls = assembler.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments_json, r#"{"expr":"2+2"}"#);
    }

    /// `extra_content` (e.g. Gemini thought signatures) survives assembly.
    #[test]
    fn extra_content_is_carried_through() {
        let mut assembler = ToolCallAssembler::new();
        let mut delta = complete_call("call_1", "calc", "{}");
        delta.extra_content = Some(serde_json::json!({"google": {"thought_signature": "sig"}}));
        assembler.push(delta);

        let calls = assembler.finish();
        assert_eq!(calls[0].extra_content.as_ref().unwrap()["google"]["thought_signature"], "sig");
    }
}
