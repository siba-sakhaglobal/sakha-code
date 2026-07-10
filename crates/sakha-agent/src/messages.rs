//! `MessageGraph` / `TurnContext`: assembles the message history sent to the
//! provider for one turn.

use serde::{Deserialize, Serialize};

use sakha_core::{SessionId, TurnId};
use sakha_provider::{ModelMessage, ToolDefinition};

/// The provider-neutral message history plus available tools for one turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageGraph {
    pub messages: Vec<ModelMessage>,
}

impl MessageGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, message: ModelMessage) {
        self.messages.push(message);
    }
}

/// Ambient context for a single turn: session/turn ids, message graph, and
/// the tool definitions offered to the model.
#[derive(Debug, Clone)]
pub struct TurnContext {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub messages: MessageGraph,
    pub tools: Vec<ToolDefinition>,
}

impl TurnContext {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            turn_id: TurnId::new(),
            messages: MessageGraph::new(),
            tools: Vec::new(),
        }
    }
}
