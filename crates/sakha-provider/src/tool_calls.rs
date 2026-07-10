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
}

/// A fully assembled tool call, ready for schema validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

/// Accumulates `ToolCallDelta`s by index and assembles them into complete
/// tool calls once a response finishes.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    partials: std::collections::BTreeMap<u32, (Option<String>, Option<String>, String)>,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, delta: ToolCallDelta) {
        let entry = self.partials.entry(delta.index).or_insert((None, None, String::new()));
        if delta.id.is_some() {
            entry.0 = delta.id;
        }
        if delta.name.is_some() {
            entry.1 = delta.name;
        }
        entry.2.push_str(&delta.arguments_fragment);
    }

    /// Finalizes accumulated deltas into assembled tool calls, in index order.
    /// Entries missing an `id` or `name` are skipped (malformed stream).
    pub fn finish(self) -> Vec<AssembledToolCall> {
        self.partials
            .into_values()
            .filter_map(|(id, name, args)| match (id, name) {
                (Some(id), Some(name)) => Some(AssembledToolCall {
                    id,
                    name,
                    arguments_json: args,
                }),
                _ => None,
            })
            .collect()
    }
}
