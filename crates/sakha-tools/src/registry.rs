//! `ToolRegistry`: the set of tools available to an agent session.

use std::collections::HashMap;
use std::sync::Arc;

use sakha_core::{SakhaError, SakhaResult};

use crate::tool::{Tool, ToolName, ToolSpec};

/// Holds registered tools by name, and answers spec-listing queries used to
/// build the model's tool-use prompt.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<ToolName, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.spec().name;
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn get_or_err(&self, name: &ToolName) -> SakhaResult<Arc<dyn Tool>> {
        self.get(name)
            .ok_or_else(|| SakhaError::invalid_input("sakha-tools", format!("unknown tool: {name}")))
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
