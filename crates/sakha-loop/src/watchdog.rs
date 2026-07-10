//! `LoopWatchdog`: detects stalls (repeated tool calls/prompts, no
//! progress) and budget exhaustion. See spec "Infinite Loop Prevention" and
//! "Loop Control Rules" -> "every loop must define max ... cost".

use serde::{Deserialize, Serialize};

use sakha_core::{Budget, BudgetDimension, BudgetLedger, LoopId};

/// A report describing why a loop is considered stalled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StallReport {
    pub stalled: bool,
    pub reasons: Vec<String>,
}

impl StallReport {
    fn merge(mut self, other: StallReport) -> Self {
        if other.stalled {
            self.stalled = true;
            self.reasons.extend(other.reasons);
        }
        self
    }
}

/// Per-loop history the watchdog tracks to detect the "Infinite Loop
/// Prevention" patterns from the spec: repeated state hashes, repeated tool
/// calls, repeated prompts, and recurring errors.
#[derive(Debug, Default)]
struct LoopHistory {
    state_hashes: Vec<String>,
    tool_calls: Vec<String>,
    prompts: Vec<String>,
    errors: Vec<String>,
}

/// Detects stalls by comparing state hashes and tool-call/prompt repetition
/// across ticks, plus budget-driven stop conditions. See spec "Infinite Loop
/// Prevention": state hash comparison, repeated tool-call detector, repeated
/// prompt detector, same-error recurrence detector.
pub struct LoopWatchdog {
    pub max_repeated_tool_calls: u32,
    pub max_repeated_prompts: u32,
    pub max_repeated_errors: u32,
    history: std::collections::HashMap<LoopId, LoopHistory>,
}

impl LoopWatchdog {
    pub fn new(max_repeated_tool_calls: u32) -> Self {
        Self {
            max_repeated_tool_calls,
            max_repeated_prompts: max_repeated_tool_calls,
            max_repeated_errors: max_repeated_tool_calls,
            history: std::collections::HashMap::new(),
        }
    }

    /// Counts how many times `needle` appears among the last
    /// `threshold + 1` entries of `haystack`.
    fn repeat_count(haystack: &[String], needle: &str, threshold: u32) -> u32 {
        haystack.iter().rev().take(threshold as usize + 1).filter(|h| h.as_str() == needle).count() as u32
    }

    /// Records a state hash for `loop_id` and returns a `StallReport`
    /// reflecting whether the same hash has repeated beyond the threshold.
    pub fn observe(&mut self, loop_id: LoopId, state_hash: String) -> StallReport {
        let history = self.history.entry(loop_id).or_default();
        history.state_hashes.push(state_hash.clone());
        let repeat_count = Self::repeat_count(&history.state_hashes, &state_hash, self.max_repeated_tool_calls);
        if repeat_count > self.max_repeated_tool_calls {
            StallReport { stalled: true, reasons: vec![format!("state hash repeated {repeat_count} times")] }
        } else {
            StallReport::default()
        }
    }

    /// Records a tool-call signature (e.g. `"{tool_name}:{input_json}"`) and
    /// flags a stall if the same call has repeated too many times in a row.
    /// Spec "Infinite Loop Prevention" -> "Repeated tool-call detector".
    pub fn observe_tool_call(&mut self, loop_id: LoopId, tool_call_signature: String) -> StallReport {
        let history = self.history.entry(loop_id).or_default();
        history.tool_calls.push(tool_call_signature.clone());
        let repeat_count = Self::repeat_count(&history.tool_calls, &tool_call_signature, self.max_repeated_tool_calls);
        if repeat_count > self.max_repeated_tool_calls {
            StallReport { stalled: true, reasons: vec![format!("tool call repeated {repeat_count} times: {tool_call_signature}")] }
        } else {
            StallReport::default()
        }
    }

    /// Records a prompt hash/signature and flags a stall on repetition.
    /// Spec "Infinite Loop Prevention" -> "Repeated prompt detector".
    pub fn observe_prompt(&mut self, loop_id: LoopId, prompt_signature: String) -> StallReport {
        let history = self.history.entry(loop_id).or_default();
        history.prompts.push(prompt_signature.clone());
        let repeat_count = Self::repeat_count(&history.prompts, &prompt_signature, self.max_repeated_prompts);
        if repeat_count > self.max_repeated_prompts {
            StallReport { stalled: true, reasons: vec![format!("prompt repeated {repeat_count} times")] }
        } else {
            StallReport::default()
        }
    }

    /// Records an error category (e.g. `"compile_error"`) and flags a stall
    /// when the same failure keeps recurring. Spec "Infinite Loop
    /// Prevention" -> "Same-error recurrence detector" / "Max retry per
    /// failure category".
    pub fn observe_error(&mut self, loop_id: LoopId, error_category: String) -> StallReport {
        let history = self.history.entry(loop_id).or_default();
        history.errors.push(error_category.clone());
        let repeat_count = Self::repeat_count(&history.errors, &error_category, self.max_repeated_errors);
        if repeat_count > self.max_repeated_errors {
            StallReport { stalled: true, reasons: vec![format!("error '{error_category}' recurred {repeat_count} times")] }
        } else {
            StallReport::default()
        }
    }

    /// Checks a `BudgetLedger` against its configured `Budget` and reports a
    /// stall (with a budget-specific reason) for every exhausted dimension.
    /// Spec "Loop Control Rules" -> "every loop must define max ... cost" /
    /// "max wall-clock time" / "max iterations", enforced here as a stop
    /// condition alongside stall detection.
    pub fn check_budget(&self, ledger: &BudgetLedger) -> StallReport {
        let limits: Budget = ledger.limits();
        let mut reasons = Vec::new();
        let dims = [
            (BudgetDimension::InputTokens, "input tokens", limits.max_input_tokens),
            (BudgetDimension::OutputTokens, "output tokens", limits.max_output_tokens),
            (BudgetDimension::CostMicros, "cost", limits.max_cost_micros),
            (BudgetDimension::ToolCalls, "tool calls", limits.max_tool_calls),
            (BudgetDimension::LoopIterations, "loop iterations", limits.max_loop_iterations),
            (BudgetDimension::FilesystemWrites, "filesystem writes", limits.max_filesystem_writes),
        ];
        for (dim, label, limit) in dims {
            if limit.is_some() && ledger.is_exhausted(dim) {
                reasons.push(format!("budget exhausted: {label} (used={}, limit={})", ledger.used(dim), limit.unwrap()));
            }
        }
        StallReport { stalled: !reasons.is_empty(), reasons }
    }

    /// Convenience: runs `observe` and `check_budget` together, merging
    /// results into a single report.
    pub fn observe_with_budget(&mut self, loop_id: LoopId, state_hash: String, ledger: &BudgetLedger) -> StallReport {
        self.observe(loop_id, state_hash).merge(self.check_budget(ledger))
    }

    /// Clears tracked history for a loop, e.g. after a successful
    /// checkpoint/handoff resets progress tracking for the next run.
    pub fn reset(&mut self, loop_id: LoopId) {
        self.history.remove(&loop_id);
    }
}

impl Default for LoopWatchdog {
    fn default() -> Self {
        Self::new(3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_repeated_state_hash_as_stall() {
        let mut watchdog = LoopWatchdog::new(2);
        let loop_id = LoopId::new();
        watchdog.observe(loop_id, "hash-a".into());
        watchdog.observe(loop_id, "hash-a".into());
        let report = watchdog.observe(loop_id, "hash-a".into());
        assert!(report.stalled);
    }

    #[test]
    fn distinct_hashes_do_not_stall() {
        let mut watchdog = LoopWatchdog::new(2);
        let loop_id = LoopId::new();
        watchdog.observe(loop_id, "a".into());
        let report = watchdog.observe(loop_id, "b".into());
        assert!(!report.stalled);
    }

    #[test]
    fn flags_repeated_tool_call() {
        let mut watchdog = LoopWatchdog::new(1);
        let loop_id = LoopId::new();
        watchdog.observe_tool_call(loop_id, "shell.run:{cmd:ls}".into());
        let report = watchdog.observe_tool_call(loop_id, "shell.run:{cmd:ls}".into());
        assert!(report.stalled);
    }

    #[test]
    fn flags_repeated_prompt() {
        let mut watchdog = LoopWatchdog::new(1);
        let loop_id = LoopId::new();
        watchdog.observe_prompt(loop_id, "same prompt".into());
        let report = watchdog.observe_prompt(loop_id, "same prompt".into());
        assert!(report.stalled);
    }

    #[test]
    fn flags_recurring_error_category() {
        let mut watchdog = LoopWatchdog::new(1);
        let loop_id = LoopId::new();
        watchdog.observe_error(loop_id, "compile_error".into());
        let report = watchdog.observe_error(loop_id, "compile_error".into());
        assert!(report.stalled);
    }

    #[test]
    fn budget_stops_loop_when_exhausted() {
        let ledger = BudgetLedger::new(Budget { max_loop_iterations: Some(2), ..Budget::unlimited() });
        ledger.debit(BudgetDimension::LoopIterations, 2).unwrap();
        let watchdog = LoopWatchdog::new(3);
        let report = watchdog.check_budget(&ledger);
        assert!(report.stalled);
        assert!(report.reasons[0].contains("loop iterations"));
    }

    #[test]
    fn budget_within_limits_does_not_stop_loop() {
        let ledger = BudgetLedger::new(Budget { max_loop_iterations: Some(10), ..Budget::unlimited() });
        ledger.debit(BudgetDimension::LoopIterations, 2).unwrap();
        let watchdog = LoopWatchdog::new(3);
        let report = watchdog.check_budget(&ledger);
        assert!(!report.stalled);
    }

    #[test]
    fn reset_clears_history() {
        let mut watchdog = LoopWatchdog::new(1);
        let loop_id = LoopId::new();
        watchdog.observe(loop_id, "a".into());
        watchdog.observe(loop_id, "a".into());
        watchdog.reset(loop_id);
        let report = watchdog.observe(loop_id, "a".into());
        assert!(!report.stalled);
    }
}
