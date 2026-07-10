//! `sakha eval run`: runs a deterministic offline smoke-eval against the
//! configured provider (`sakha-loop::verification_loop::Verifier`). A full
//! eval harness lives in `sakha-evals` per the domain-model crate layout;
//! until that crate exists, this exercises the real `Verifier` trait (via
//! `CommandVerifier`) against a canned artifact so the command is genuinely
//! wired, not a placeholder.

use clap::Args;
use serde::Serialize;

use sakha_loop::{CommandVerifier, Rubric, Verifier};

use crate::output::{print_error, print_output, OutputFormat};

#[derive(Debug, Args)]
pub struct EvalRunArgs {
    /// Name of the eval suite to run. Only `smoke` is available until
    /// `sakha-evals` lands.
    #[arg(default_value = "smoke")]
    pub suite: String,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct EvalRunView {
    suite: String,
    passed: bool,
    detail: String,
}

pub fn execute(args: EvalRunArgs) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(args))
}

async fn execute_async(args: EvalRunArgs) -> i32 {
    if args.suite != "smoke" {
        print_error(
            &format!("unknown eval suite '{}': only 'smoke' is available until sakha-evals lands", args.suite),
            args.output,
        );
        return 2;
    }

    // Smoke suite: an empty rubric against a canned artifact. `CommandVerifier`
    // treats an empty rubric as trivially satisfied (no criteria to fail),
    // which is enough to prove the CLI -> sakha-loop -> Verifier wiring
    // actually runs end to end.
    let verifier = CommandVerifier::new();
    let artifact = sakha_core::ArtifactRef::new(sakha_core::ArtifactKind::Text, "sha256:smoke", 0);
    let rubric = Rubric::default();

    match verifier.verify_artifact(artifact, rubric).await {
        Ok(result) => {
            let view = EvalRunView {
                suite: args.suite,
                passed: result.passed,
                detail: result.notes.clone(),
            };
            let passed = view.passed;
            print_output(&view, args.output);
            if passed {
                0
            } else {
                1
            }
        }
        Err(err) => {
            print_error(&err.to_string(), args.output);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_run_smoke_suite_succeeds() {
        let code = execute(EvalRunArgs { suite: "smoke".into(), output: OutputFormat::Json });
        assert_eq!(code, 0);
    }

    #[test]
    fn eval_run_unknown_suite_reports_error() {
        let code = execute(EvalRunArgs { suite: "nonexistent".into(), output: OutputFormat::Json });
        assert_eq!(code, 2);
    }
}
