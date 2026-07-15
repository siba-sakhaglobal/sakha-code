# Module 12: Verification and Evals

## Responsibility

Verify agent work, evaluate regressions, and provide objective signals for loop continuation.

## Rust Crate

`crates/sakha-evals`

## Main Structs

- `VerifierRegistry`
- `Verifier`
- `CheckSpec`
- `CheckResult`
- `VerificationPlan`
- `VerificationResult`
- `Rubric`
- `EvalCase`
- `EvalRun`
- `EvalReport`
- `RegressionSuite`

## Verifier Types

- Command verifier: run tests/lint/build.
- Static verifier: inspect files/diffs.
- Schema verifier: validate JSON/YAML/config.
- Rubric verifier: deterministic checklist.
- LLM judge verifier: optional and marked nondeterministic.
- Web source verifier: check citations against evidence.
- Security verifier: detect risky commands/secrets.

## Core Checks

- Build passes.
- Tests pass.
- Lint passes.
- Typecheck passes.
- Diff scoped to task.
- No secrets added.
- No destructive unrelated changes.
- Documentation updated.
- New files follow structure.
- User acceptance criteria satisfied.

## Eval Suites

- Provider streaming evals.
- Tool-call parsing evals.
- Patch application evals.
- Compression preservation evals.
- Research citation evals.
- Loop termination evals.
- Sandbox policy evals.
- Sub-agent handoff evals.

## Implementation Tasks

1. Define verifier trait.
2. Implement command verifier.
3. Implement diff verifier.
4. Implement citation verifier.
5. Implement compression eval harness.
6. Implement loop eval harness.
7. Implement test fixture format.
8. Implement reports.
9. Add CI integration.
10. Add regression dashboard endpoint.

## Tests

- Failing test blocks completion.
- Passing checks complete goal.
- Unrelated diff is flagged.
- Citation missing source is flagged.
- Compression eval detects lost key fact.
- Loop eval detects repeated action.

