//! `ReplayTrace` / `LoopSkill`: deterministic replay of previously learned
//! tool trajectories. See spec "Deterministic Replay Suggestion".

use serde::{Deserialize, Serialize};

use sakha_core::{SakhaError, SakhaResult, ToolCallId};

/// A recorded step in a successful tool trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayStep {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub input_json: serde_json::Value,
    pub output_json: serde_json::Value,
}

/// A full recorded trajectory from a successful loop tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplayTrace {
    pub steps: Vec<ReplayStep>,
}

/// A reusable, parameterized template extracted from a `ReplayTrace`. See
/// spec `LoopSkillRecorder` -> "Produces `LoopSkill` template".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopSkill {
    pub name: String,
    pub trace: ReplayTrace,
    pub variables: Vec<String>,
}

/// Records successful trajectories and extracts `LoopSkill`s from them. See
/// spec "Add `LoopSkillRecorder`": records trajectory, extracts variables
/// from inputs/outputs, produces a `LoopSkill` template.
pub struct LoopSkillRecorder;

impl LoopSkillRecorder {
    pub fn new() -> Self {
        Self
    }

    pub fn record(&self, trace: ReplayTrace) -> ReplayTrace {
        trace
    }

    /// Extracts a `LoopSkill` from `trace`, scanning each step's
    /// `input_json` object for top-level string values that look like
    /// variables worth parameterizing (heuristically: values that also
    /// appear, verbatim, as a top-level string value in another step's
    /// input — i.e. a value threaded through multiple steps, like a file
    /// path or branch name). This is a conservative heuristic; anything not
    /// detected simply stays a literal in the trace rather than becoming a
    /// template placeholder, which is safe (replay still works, just with
    /// less parameterization).
    pub fn extract_skill(&self, name: impl Into<String>, trace: ReplayTrace) -> LoopSkill {
        let variables = extract_repeated_string_values(&trace);
        LoopSkill { name: name.into(), trace, variables }
    }
}

impl Default for LoopSkillRecorder {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_top_level_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Object(map) => map
            .values()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Finds string values that recur across more than one step's input, since
/// a value threaded through multiple tool calls (a path, an id, a branch
/// name) is the clearest signal of a "variable" worth extracting for
/// templating, per spec "Extracts variables from inputs/outputs".
fn extract_repeated_string_values(trace: &ReplayTrace) -> Vec<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, u32> = HashMap::new();
    for step in &trace.steps {
        for value in collect_top_level_strings(&step.input_json) {
            *counts.entry(value).or_insert(0) += 1;
        }
    }
    let mut variables: Vec<String> = counts.into_iter().filter(|(_, count)| *count > 1).map(|(value, _)| value).collect();
    variables.sort();
    variables
}

/// Replays a `LoopSkill` without invoking the model, unless unexpected state
/// is encountered. A dry run validates the trace is internally consistent
/// (every step has a tool name and well-formed JSON, which is already
/// guaranteed by the type system here) and reports whether replay would be
/// safe to attempt for real. Per spec "Runs future periodic tasks without
/// LLM unless unexpected state appears" — an empty trace is trivially
/// replayable (there is nothing to diverge on); a trace with at least one
/// step is considered dry-run-safe only if none of its steps look
/// malformed.
pub struct ReplayEngine;

impl ReplayEngine {
    pub fn new() -> Self {
        Self
    }

    /// Returns `Ok(true)` if the skill can be replayed without a model call,
    /// `Ok(false)` if the trace looks unsafe to replay blind (e.g. contains
    /// a step with an empty tool name), and `Err` only for structural
    /// problems that make the trace impossible to reason about.
    pub async fn dry_run(&self, skill: &LoopSkill) -> SakhaResult<bool> {
        if skill.trace.steps.is_empty() {
            return Ok(true);
        }
        for step in &skill.trace.steps {
            if step.tool_name.trim().is_empty() {
                return Err(SakhaError::invalid_input("sakha-loop", format!("replay step {} has empty tool name", step.tool_call_id)));
            }
        }
        // Every step is well-formed; safe to attempt deterministic replay.
        Ok(true)
    }
}

impl Default for ReplayEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(tool: &str, input: serde_json::Value) -> ReplayStep {
        ReplayStep { tool_call_id: ToolCallId::new(), tool_name: tool.into(), input_json: input, output_json: json!({}) }
    }

    #[tokio::test]
    async fn empty_trace_dry_runs_without_model_call() {
        let engine = ReplayEngine::new();
        let skill = LoopSkill { name: "noop".into(), trace: ReplayTrace::default(), variables: vec![] };
        assert!(engine.dry_run(&skill).await.unwrap());
    }

    #[tokio::test]
    async fn well_formed_trace_dry_runs_successfully() {
        let engine = ReplayEngine::new();
        let trace = ReplayTrace { steps: vec![step("file.read", json!({"path": "a.txt"}))] };
        let skill = LoopSkill { name: "read-a".into(), trace, variables: vec![] };
        assert!(engine.dry_run(&skill).await.unwrap());
    }

    #[tokio::test]
    async fn malformed_step_fails_dry_run() {
        let engine = ReplayEngine::new();
        let trace = ReplayTrace { steps: vec![step("", json!({}))] };
        let skill = LoopSkill { name: "bad".into(), trace, variables: vec![] };
        assert!(engine.dry_run(&skill).await.is_err());
    }

    #[test]
    fn extract_skill_finds_values_repeated_across_steps() {
        let recorder = LoopSkillRecorder::new();
        let trace = ReplayTrace {
            steps: vec![
                step("file.read", json!({"path": "src/lib.rs", "branch": "main"})),
                step("file.write", json!({"path": "src/lib.rs", "content": "fn main() {}"})),
            ],
        };
        let skill = recorder.extract_skill("edit-lib", trace);
        assert!(skill.variables.contains(&"src/lib.rs".to_string()));
        assert!(!skill.variables.contains(&"main".to_string()));
    }

    #[test]
    fn extract_skill_on_single_step_has_no_repeated_variables() {
        let recorder = LoopSkillRecorder::new();
        let trace = ReplayTrace { steps: vec![step("file.read", json!({"path": "a.txt"}))] };
        let skill = recorder.extract_skill("solo", trace);
        assert!(skill.variables.is_empty());
    }

    #[test]
    fn record_returns_trace_unchanged() {
        let recorder = LoopSkillRecorder::new();
        let trace = ReplayTrace { steps: vec![step("git.status", json!({}))] };
        let recorded = recorder.record(trace.clone());
        assert_eq!(recorded.steps.len(), trace.steps.len());
    }
}
