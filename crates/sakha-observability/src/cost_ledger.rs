//! `CostLedger`/`TokenAccounting`: accumulates cost and token usage per
//! session and per loop. See spec "Cost ledger", "Metrics" (tokens in/out,
//! cost per goal) and `04-core-domain-model.md` `UsageRecord`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use sakha_core::{LoopId, SessionId};

/// Recovers from a poisoned mutex by taking the inner data anyway.
///
/// Cost tracking must never bring down the caller: even if some other
/// thread panicked while holding the lock, the ledger's (structurally
/// valid) map is still usable, so we recover it via `into_inner` rather
/// than propagating the panic to every future ledger call.
fn recover<'a, T>(result: Result<std::sync::MutexGuard<'a, T>, std::sync::PoisonError<std::sync::MutexGuard<'a, T>>>) -> std::sync::MutexGuard<'a, T> {
    match result {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Accumulated token counts for a scope.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenAccounting {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenAccounting {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// One accumulated ledger entry: total cost plus token accounting.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub cost_micros: u64,
    pub tokens: TokenAccounting,
    /// Number of `record` calls folded into this entry (i.e. request count).
    pub request_count: u64,
}

/// Tracks cost (micro-USD) and token usage per session and per loop.
/// Thread-safe via internal mutexes so it can be shared across concurrent
/// tasks (e.g. multiple in-flight tool calls or loop ticks).
///
/// Session and loop scopes are tracked independently: a loop's cost is not
/// automatically folded into the owning session's total, since a loop may
/// span (or outlive) many sessions. Callers that want a "cost per goal"
/// rollup should record into both scopes when both IDs are known.
#[derive(Default)]
pub struct CostLedger {
    per_session: Mutex<HashMap<SessionId, LedgerEntry>>,
    per_loop: Mutex<HashMap<LoopId, LedgerEntry>>,
}

fn accumulate(entry: &mut LedgerEntry, cost_micros: u64, input_tokens: u64, output_tokens: u64) {
    entry.cost_micros += cost_micros;
    entry.tokens.input_tokens += input_tokens;
    entry.tokens.output_tokens += output_tokens;
    entry.request_count += 1;
}

impl CostLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records usage against a session scope.
    pub fn record(&self, session_id: SessionId, cost_micros: u64, input_tokens: u64, output_tokens: u64) {
        let mut map = recover(self.per_session.lock());
        let entry = map.entry(session_id).or_default();
        accumulate(entry, cost_micros, input_tokens, output_tokens);
    }

    /// Records usage against a loop scope (e.g. one loop tick's model
    /// requests). Independent of any session-scoped recording.
    pub fn record_loop(&self, loop_id: LoopId, cost_micros: u64, input_tokens: u64, output_tokens: u64) {
        let mut map = recover(self.per_loop.lock());
        let entry = map.entry(loop_id).or_default();
        accumulate(entry, cost_micros, input_tokens, output_tokens);
    }

    pub fn total_cost_micros(&self, session_id: SessionId) -> u64 {
        recover(self.per_session.lock()).get(&session_id).map(|e| e.cost_micros).unwrap_or(0)
    }

    pub fn loop_total_cost_micros(&self, loop_id: LoopId) -> u64 {
        recover(self.per_loop.lock()).get(&loop_id).map(|e| e.cost_micros).unwrap_or(0)
    }

    pub fn token_accounting(&self, session_id: SessionId) -> TokenAccounting {
        recover(self.per_session.lock()).get(&session_id).map(|e| e.tokens).unwrap_or_default()
    }

    pub fn loop_token_accounting(&self, loop_id: LoopId) -> TokenAccounting {
        recover(self.per_loop.lock()).get(&loop_id).map(|e| e.tokens).unwrap_or_default()
    }

    /// Full ledger entry (cost + tokens + request count) for a session.
    pub fn session_entry(&self, session_id: SessionId) -> LedgerEntry {
        recover(self.per_session.lock()).get(&session_id).copied().unwrap_or_default()
    }

    /// Full ledger entry (cost + tokens + request count) for a loop.
    pub fn loop_entry(&self, loop_id: LoopId) -> LedgerEntry {
        recover(self.per_loop.lock()).get(&loop_id).copied().unwrap_or_default()
    }

    /// Total cost (in micro-USD) summed across every tracked session.
    pub fn grand_total_session_cost_micros(&self) -> u64 {
        recover(self.per_session.lock()).values().map(|e| e.cost_micros).sum()
    }

    /// Total cost (in micro-USD) summed across every tracked loop.
    pub fn grand_total_loop_cost_micros(&self) -> u64 {
        recover(self.per_loop.lock()).values().map(|e| e.cost_micros).sum()
    }
}

/// A standalone token-only ledger, per spec `modules/15-observability-audit.md`
/// "Main Structs" (`TokenLedger` listed distinctly from `TokenAccounting`,
/// which is the plain data record it accumulates). Useful for callers that
/// want to track tokens in/out per session without also paying for cost
/// tracking (e.g. a provider with no per-token pricing configured yet).
#[derive(Default)]
pub struct TokenLedger {
    per_session: Mutex<HashMap<SessionId, TokenAccounting>>,
}

impl TokenLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `input_tokens`/`output_tokens` against a session scope.
    pub fn record(&self, session_id: SessionId, input_tokens: u64, output_tokens: u64) {
        let mut map = recover(self.per_session.lock());
        let entry = map.entry(session_id).or_default();
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
    }

    pub fn accounting(&self, session_id: SessionId) -> TokenAccounting {
        recover(self.per_session.lock()).get(&session_id).copied().unwrap_or_default()
    }

    /// Total tokens (input + output) summed across every tracked session.
    pub fn grand_total_tokens(&self) -> u64 {
        recover(self.per_session.lock()).values().map(|t| t.total_tokens()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_ledger_totals_across_session() {
        let ledger = CostLedger::new();
        let session = SessionId::new();
        ledger.record(session, 100, 10, 5);
        ledger.record(session, 50, 3, 2);
        assert_eq!(ledger.total_cost_micros(session), 150);
        let tokens = ledger.token_accounting(session);
        assert_eq!(tokens.input_tokens, 13);
        assert_eq!(tokens.output_tokens, 7);
    }

    #[test]
    fn cost_ledger_tracks_loops_independently_of_sessions() {
        let ledger = CostLedger::new();
        let session = SessionId::new();
        let loop_id = LoopId::new();
        ledger.record(session, 100, 10, 5);
        ledger.record_loop(loop_id, 40, 4, 1);
        assert_eq!(ledger.total_cost_micros(session), 100);
        assert_eq!(ledger.loop_total_cost_micros(loop_id), 40);
        assert_eq!(ledger.grand_total_session_cost_micros(), 100);
        assert_eq!(ledger.grand_total_loop_cost_micros(), 40);
    }

    #[test]
    fn unrecorded_scope_defaults_to_zero() {
        let ledger = CostLedger::new();
        assert_eq!(ledger.total_cost_micros(SessionId::new()), 0);
        assert_eq!(ledger.loop_total_cost_micros(LoopId::new()), 0);
    }

    #[test]
    fn session_entry_tracks_request_count() {
        let ledger = CostLedger::new();
        let session = SessionId::new();
        ledger.record(session, 10, 1, 1);
        ledger.record(session, 10, 1, 1);
        ledger.record(session, 10, 1, 1);
        assert_eq!(ledger.session_entry(session).request_count, 3);
    }

    #[test]
    fn token_accounting_total_tokens_sums_in_and_out() {
        let tokens = TokenAccounting { input_tokens: 7, output_tokens: 3 };
        assert_eq!(tokens.total_tokens(), 10);
    }

    #[test]
    fn token_ledger_accumulates_per_session() {
        let ledger = TokenLedger::new();
        let session = SessionId::new();
        ledger.record(session, 10, 5);
        ledger.record(session, 3, 2);
        let accounting = ledger.accounting(session);
        assert_eq!(accounting.input_tokens, 13);
        assert_eq!(accounting.output_tokens, 7);
        assert_eq!(ledger.grand_total_tokens(), 20);
    }

    #[test]
    fn token_ledger_unrecorded_session_defaults_to_zero() {
        let ledger = TokenLedger::new();
        assert_eq!(ledger.accounting(SessionId::new()).total_tokens(), 0);
    }
}
