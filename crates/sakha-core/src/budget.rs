//! Budget model: tracks token/cost/tool-call/time/write limits for a session,
//! turn, or loop, and a ledger that debits/credits against them atomically.
//!
//! Per `04-core-domain-model.md`, `Session`/`Goal`/`LoopSpec` all carry a
//! `Budget`. This module owns the shared representation and enforcement.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use crate::error::SakhaError;

/// Declared limits for a scope (session/turn/loop). `None` means unlimited
/// for that dimension.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Budget {
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// Cost expressed in micro-USD (1e-6 USD) to avoid floating point drift.
    pub max_cost_micros: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_wall_time: Option<Duration>,
    pub max_loop_iterations: Option<u64>,
    pub max_filesystem_writes: Option<u64>,
}

impl Budget {
    pub fn unlimited() -> Self {
        Self::default()
    }
}

/// Which dimension of the budget a debit/credit applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetDimension {
    InputTokens,
    OutputTokens,
    CostMicros,
    ToolCalls,
    /// Wall-clock time consumed, in milliseconds. Debited by callers as
    /// elapsed time accrues; compared against `Budget::max_wall_time`.
    WallTime,
    LoopIterations,
    FilesystemWrites,
}

/// Live counters tracked against a `Budget`. Uses atomics so it can be shared
/// (e.g. `Arc<BudgetLedger>`) across concurrent tool/model tasks without an
/// external mutex for the common debit/credit path.
#[derive(Debug)]
pub struct BudgetLedger {
    limits: Budget,
    input_tokens: AtomicU64,
    output_tokens: AtomicU64,
    cost_micros: AtomicU64,
    tool_calls: AtomicU64,
    wall_time_millis: AtomicU64,
    loop_iterations: AtomicU64,
    filesystem_writes: AtomicU64,
    /// Signed so credits (refunds) beyond zero don't panic; clamped at read time.
    _reserved: AtomicI64,
}

/// Upper bound on compare-exchange retry attempts for `debit`/`credit`
/// before giving up. Under real contention (a handful of concurrent
/// tool/model tasks sharing one ledger) a CAS loop converges in a handful of
/// iterations; this cap exists purely so a pathological/adversarial amount
/// of concurrent interference can never spin a thread forever, per the
/// "unbounded loops without budget checks" principle.
const MAX_CAS_ATTEMPTS: u32 = 10_000;

impl BudgetLedger {
    pub fn new(limits: Budget) -> Self {
        Self {
            limits,
            input_tokens: AtomicU64::new(0),
            output_tokens: AtomicU64::new(0),
            cost_micros: AtomicU64::new(0),
            tool_calls: AtomicU64::new(0),
            wall_time_millis: AtomicU64::new(0),
            loop_iterations: AtomicU64::new(0),
            filesystem_writes: AtomicU64::new(0),
            _reserved: AtomicI64::new(0),
        }
    }

    fn counter(&self, dim: BudgetDimension) -> &AtomicU64 {
        match dim {
            BudgetDimension::InputTokens => &self.input_tokens,
            BudgetDimension::OutputTokens => &self.output_tokens,
            BudgetDimension::CostMicros => &self.cost_micros,
            BudgetDimension::ToolCalls => &self.tool_calls,
            BudgetDimension::WallTime => &self.wall_time_millis,
            BudgetDimension::LoopIterations => &self.loop_iterations,
            BudgetDimension::FilesystemWrites => &self.filesystem_writes,
        }
    }

    fn limit(&self, dim: BudgetDimension) -> Option<u64> {
        match dim {
            BudgetDimension::InputTokens => self.limits.max_input_tokens,
            BudgetDimension::OutputTokens => self.limits.max_output_tokens,
            BudgetDimension::CostMicros => self.limits.max_cost_micros,
            BudgetDimension::ToolCalls => self.limits.max_tool_calls,
            BudgetDimension::WallTime => self.limits.max_wall_time.map(|d| d.as_millis() as u64),
            BudgetDimension::LoopIterations => self.limits.max_loop_iterations,
            BudgetDimension::FilesystemWrites => self.limits.max_filesystem_writes,
        }
    }

    /// Attempts to debit `amount` from `dim`. Returns `Err(SakhaError::Budget)`
    /// without mutating state if the debit would exceed the configured limit.
    ///
    /// The compare-exchange retry loop is bounded by `MAX_CAS_ATTEMPTS`: if
    /// contention prevents the CAS from ever winning within that many
    /// attempts, this returns a non-retryable `SakhaError::Fatal` rather than
    /// spinning indefinitely (see module principle "no unbounded loops
    /// without a budget/iteration check").
    pub fn debit(&self, dim: BudgetDimension, amount: u64) -> Result<(), SakhaError> {
        let counter = self.counter(dim);
        let limit = self.limit(dim);
        for _ in 0..MAX_CAS_ATTEMPTS {
            let current = counter.load(Ordering::SeqCst);
            let next = current.saturating_add(amount);
            if let Some(limit) = limit {
                if next > limit {
                    return Err(SakhaError::budget(
                        "sakha-core",
                        format!(
                            "budget exceeded for {dim:?}: current={current} amount={amount} limit={limit}"
                        ),
                    ));
                }
            }
            if counter
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(());
            }
            // CAS lost the race; retry.
        }
        Err(SakhaError::fatal(
            "sakha-core",
            format!("budget ledger debit for {dim:?} could not make progress after {MAX_CAS_ATTEMPTS} attempts (contention)"),
        ))
    }

    /// Credits (refunds) `amount` back to `dim`, e.g. when a retry is charged
    /// against the same ledger but the operation is later voided. Never goes
    /// below zero.
    ///
    /// Like `debit`, the retry loop is bounded by `MAX_CAS_ATTEMPTS`; if the
    /// CAS cannot win within that many attempts under contention, the credit
    /// is silently dropped rather than looping forever. A missed refund is
    /// recoverable (the ledger just stays slightly more conservative); an
    /// unbounded spin is not.
    pub fn credit(&self, dim: BudgetDimension, amount: u64) {
        let counter = self.counter(dim);
        for _ in 0..MAX_CAS_ATTEMPTS {
            let current = counter.load(Ordering::SeqCst);
            let next = current.saturating_sub(amount);
            if counter
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return;
            }
        }
    }

    pub fn used(&self, dim: BudgetDimension) -> u64 {
        self.counter(dim).load(Ordering::SeqCst)
    }

    pub fn remaining(&self, dim: BudgetDimension) -> Option<u64> {
        self.limit(dim).map(|limit| limit.saturating_sub(self.used(dim)))
    }

    pub fn is_exhausted(&self, dim: BudgetDimension) -> bool {
        match self.limit(dim) {
            Some(limit) => self.used(dim) >= limit,
            None => false,
        }
    }

    pub fn limits(&self) -> Budget {
        self.limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debit_within_limit_succeeds() {
        let ledger = BudgetLedger::new(Budget {
            max_tool_calls: Some(10),
            ..Budget::unlimited()
        });
        assert!(ledger.debit(BudgetDimension::ToolCalls, 3).is_ok());
        assert_eq!(ledger.used(BudgetDimension::ToolCalls), 3);
    }

    #[test]
    fn debit_beyond_limit_fails_and_does_not_mutate() {
        let ledger = BudgetLedger::new(Budget {
            max_tool_calls: Some(5),
            ..Budget::unlimited()
        });
        assert!(ledger.debit(BudgetDimension::ToolCalls, 3).is_ok());
        let result = ledger.debit(BudgetDimension::ToolCalls, 3);
        assert!(result.is_err());
        // Failed debit must not have mutated the counter.
        assert_eq!(ledger.used(BudgetDimension::ToolCalls), 3);
    }

    #[test]
    fn credit_refunds_and_never_goes_negative() {
        let ledger = BudgetLedger::new(Budget::unlimited());
        ledger.debit(BudgetDimension::InputTokens, 10).unwrap();
        ledger.credit(BudgetDimension::InputTokens, 4);
        assert_eq!(ledger.used(BudgetDimension::InputTokens), 6);
        ledger.credit(BudgetDimension::InputTokens, 100);
        assert_eq!(ledger.used(BudgetDimension::InputTokens), 0);
    }

    #[test]
    fn unlimited_dimension_never_exhausted() {
        let ledger = BudgetLedger::new(Budget::unlimited());
        ledger.debit(BudgetDimension::CostMicros, 1_000_000).unwrap();
        assert!(!ledger.is_exhausted(BudgetDimension::CostMicros));
        assert_eq!(ledger.remaining(BudgetDimension::CostMicros), None);
    }

    #[test]
    fn is_exhausted_reflects_limit() {
        let ledger = BudgetLedger::new(Budget {
            max_loop_iterations: Some(2),
            ..Budget::unlimited()
        });
        ledger.debit(BudgetDimension::LoopIterations, 2).unwrap();
        assert!(ledger.is_exhausted(BudgetDimension::LoopIterations));
        assert_eq!(ledger.remaining(BudgetDimension::LoopIterations), Some(0));
    }

    #[test]
    fn wall_time_dimension_is_tracked_against_max_wall_time() {
        let ledger = BudgetLedger::new(Budget {
            max_wall_time: Some(Duration::from_secs(2)),
            ..Budget::unlimited()
        });
        assert!(ledger.debit(BudgetDimension::WallTime, 1_500).is_ok());
        assert_eq!(ledger.used(BudgetDimension::WallTime), 1_500);
        // A further debit that would push past the 2000ms limit must fail
        // and must not mutate the counter.
        let result = ledger.debit(BudgetDimension::WallTime, 600);
        assert!(result.is_err());
        assert_eq!(ledger.used(BudgetDimension::WallTime), 1_500);
    }

    /// Spec requirement (`01-core-runtime.md` "Tests": "Budget exhaustion
    /// stops loop"): once a dimension's limit is reached, a loop driven by
    /// that ledger must halt (via an `Err` from `debit`) rather than keep
    /// iterating.
    #[test]
    fn budget_exhaustion_stops_loop() {
        let ledger = BudgetLedger::new(Budget {
            max_loop_iterations: Some(3),
            ..Budget::unlimited()
        });

        let mut completed_iterations = 0u64;
        let stop_reason = loop {
            match ledger.debit(BudgetDimension::LoopIterations, 1) {
                Ok(()) => {
                    completed_iterations += 1;
                    // Safety valve so a bug in this test can't hang the
                    // suite: a correctly-behaving ledger will error out at
                    // iteration 4, long before this fires.
                    if completed_iterations > 1000 {
                        panic!("loop did not stop when budget was exhausted");
                    }
                }
                Err(err) => break err,
            }
        };

        assert_eq!(completed_iterations, 3);
        assert!(ledger.is_exhausted(BudgetDimension::LoopIterations));
        assert_eq!(stop_reason.class, crate::error::ErrorClass::Budget);
    }

    #[test]
    fn concurrent_debits_do_not_exceed_limit() {
        use std::sync::Arc;
        use std::thread;

        let ledger = Arc::new(BudgetLedger::new(Budget {
            max_tool_calls: Some(100),
            ..Budget::unlimited()
        }));
        let mut handles = vec![];
        for _ in 0..200 {
            let ledger = Arc::clone(&ledger);
            handles.push(thread::spawn(move || {
                let _ = ledger.debit(BudgetDimension::ToolCalls, 1);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(ledger.used(BudgetDimension::ToolCalls) <= 100);
    }
}
