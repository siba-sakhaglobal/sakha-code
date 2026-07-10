//! Verification loop: runs checks, feeds failures back, retries. See spec
//! "Loop Layers" -> "Verification loop", `04-core-domain-model.md` `Verifier`,
//! and `modules/12-verification-evals.md` "Command verifier" / "Core Checks".

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use sakha_core::{ArtifactRef, SakhaResult};

/// A reference to something that can be verified (a diff, file, or built
/// artifact).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRefWrapper(pub ArtifactRef);

/// A rubric describing acceptance criteria for `verify_artifact`. See module
/// 12 "Rubric verifier: deterministic checklist".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rubric {
    pub criteria: Vec<String>,
}

impl Rubric {
    pub fn is_empty(&self) -> bool {
        self.criteria.is_empty()
    }
}

/// A single deterministic check to run against a diff (lint, test, build).
/// Module 12 "Command verifier: run tests/lint/build".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckSpec {
    pub name: String,
    pub command: Vec<String>,
    /// Working directory the command runs in; defaults to the current
    /// process working directory when `None`.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Wall-clock timeout in seconds; a check that overruns this is treated
    /// as failed (not hung forever), per loop "max wall-clock time" rule.
    #[serde(default = "default_check_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_check_timeout_secs() -> u64 {
    120
}

impl CheckSpec {
    pub fn new(name: impl Into<String>, command: Vec<String>) -> Self {
        Self { name: name.into(), command, working_dir: None, timeout_secs: default_check_timeout_secs() }
    }
}

/// The outcome of a single deterministic check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

/// The outcome of a verification pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationResult {
    pub passed: bool,
    pub failed_checks: Vec<String>,
    pub notes: String,
    #[serde(default)]
    pub check_results: Vec<CheckResult>,
}

/// A numeric/qualitative grade for a completed turn.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Grade {
    pub score: f32,
}

/// Checks agent work. See `04-core-domain-model.md` `Verifier`.
#[async_trait]
pub trait Verifier: Send + Sync {
    async fn verify_artifact(&self, artifact: ArtifactRef, rubric: Rubric) -> SakhaResult<VerificationResult>;
    async fn verify_diff(&self, diff: ArtifactRef, checks: Vec<CheckSpec>) -> SakhaResult<VerificationResult>;
    async fn grade_turn(&self, turn_id: sakha_core::TurnId) -> SakhaResult<Grade>;
}

/// A verifier that always reports "not yet verified" rather than fabricating
/// a pass. Safe default until real check runners are wired for a given
/// caller.
#[derive(Debug, Default)]
pub struct NullVerifier;

#[async_trait]
impl Verifier for NullVerifier {
    async fn verify_artifact(&self, _artifact: ArtifactRef, _rubric: Rubric) -> SakhaResult<VerificationResult> {
        Ok(VerificationResult { passed: false, failed_checks: Vec::new(), notes: "verification not yet implemented".into(), check_results: Vec::new() })
    }

    async fn verify_diff(&self, _diff: ArtifactRef, checks: Vec<CheckSpec>) -> SakhaResult<VerificationResult> {
        Ok(VerificationResult {
            passed: false,
            failed_checks: checks.into_iter().map(|c| c.name).collect(),
            notes: "verification not yet implemented".into(),
            check_results: Vec::new(),
        })
    }

    async fn grade_turn(&self, _turn_id: sakha_core::TurnId) -> SakhaResult<Grade> {
        Ok(Grade::default())
    }
}

/// Caps stdout/stderr tail captured per check, to keep verification results
/// small and model/UI-friendly.
const MAX_TAIL_BYTES: usize = 4096;

fn tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX_TAIL_BYTES {
        text.to_string()
    } else {
        text[text.len() - MAX_TAIL_BYTES..].to_string()
    }
}

/// Runs one `CheckSpec` as a real subprocess with a timeout, per module 12
/// "Command verifier: run tests/lint/build" and loop rule "every loop must
/// define max wall-clock time".
pub async fn run_check(spec: &CheckSpec) -> CheckResult {
    let Some((program, args)) = spec.command.split_first() else {
        return CheckResult {
            name: spec.name.clone(),
            passed: false,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: "empty command".into(),
        };
    };

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    if let Some(dir) = &spec.working_dir {
        cmd.current_dir(dir);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let run = async {
        let child = cmd.output().await;
        child
    };

    match tokio::time::timeout(std::time::Duration::from_secs(spec.timeout_secs), run).await {
        Ok(Ok(output)) => CheckResult {
            name: spec.name.clone(),
            passed: output.status.success(),
            exit_code: output.status.code(),
            stdout_tail: tail(&output.stdout),
            stderr_tail: tail(&output.stderr),
        },
        Ok(Err(e)) => CheckResult {
            name: spec.name.clone(),
            passed: false,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: format!("failed to spawn: {e}"),
        },
        Err(_) => CheckResult {
            name: spec.name.clone(),
            passed: false,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: format!("timed out after {}s", spec.timeout_secs),
        },
    }
}

/// A `Verifier` that runs real deterministic checks (command exit codes) as
/// subprocesses. `verify_artifact` treats each rubric criterion as satisfied
/// only when explicitly marked via `always_pass_rubric` (rubric grading
/// beyond deterministic commands needs an LLM judge, out of scope for this
/// crate — module 12 marks that verifier type "optional and
/// nondeterministic").
pub struct CommandVerifier {
    /// When true, a non-empty rubric with no configured judge is reported as
    /// passed (used by callers that only care about command checks). When
    /// false (default), rubric-only verification is reported as failed with
    /// an explanatory note so callers don't silently treat "no rubric judge"
    /// as success.
    pub always_pass_rubric: bool,
}

impl CommandVerifier {
    pub fn new() -> Self {
        Self { always_pass_rubric: false }
    }
}

impl Default for CommandVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Verifier for CommandVerifier {
    async fn verify_artifact(&self, _artifact: ArtifactRef, rubric: Rubric) -> SakhaResult<VerificationResult> {
        if rubric.is_empty() || self.always_pass_rubric {
            return Ok(VerificationResult {
                passed: true,
                failed_checks: Vec::new(),
                notes: "no rubric criteria configured".into(),
                check_results: Vec::new(),
            });
        }
        Ok(VerificationResult {
            passed: false,
            failed_checks: rubric.criteria,
            notes: "rubric verification requires a judge verifier (not implemented in sakha-loop)".into(),
            check_results: Vec::new(),
        })
    }

    async fn verify_diff(&self, _diff: ArtifactRef, checks: Vec<CheckSpec>) -> SakhaResult<VerificationResult> {
        Ok(run_checks(&checks).await)
    }

    async fn grade_turn(&self, _turn_id: sakha_core::TurnId) -> SakhaResult<Grade> {
        Ok(Grade::default())
    }
}

/// Runs every `CheckSpec` in order, aggregating into one `VerificationResult`.
/// A check spec list run between ticks per spec "Verification loop: run
/// checks, feed failures back, retry".
pub async fn run_checks(checks: &[CheckSpec]) -> VerificationResult {
    let mut results = Vec::with_capacity(checks.len());
    let mut failed = Vec::new();
    for check in checks {
        let result = run_check(check).await;
        if !result.passed {
            failed.push(result.name.clone());
        }
        results.push(result);
    }
    let passed = failed.is_empty();
    VerificationResult {
        passed,
        notes: if passed { "all checks passed".into() } else { format!("{} check(s) failed", failed.len()) },
        failed_checks: failed,
        check_results: results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_ok_check() -> CheckSpec {
        if cfg!(windows) {
            CheckSpec::new("ok", vec!["cmd".into(), "/C".into(), "exit 0".into()])
        } else {
            CheckSpec::new("ok", vec!["true".into()])
        }
    }

    fn echo_fail_check() -> CheckSpec {
        if cfg!(windows) {
            CheckSpec::new("fail", vec!["cmd".into(), "/C".into(), "exit 1".into()])
        } else {
            CheckSpec::new("fail", vec!["false".into()])
        }
    }

    #[tokio::test]
    async fn passing_command_check_reports_success() {
        let result = run_check(&echo_ok_check()).await;
        assert!(result.passed);
        assert_eq!(result.exit_code, Some(0));
    }

    #[tokio::test]
    async fn failing_command_check_reports_exit_code() {
        let result = run_check(&echo_fail_check()).await;
        assert!(!result.passed);
        assert_ne!(result.exit_code, Some(0));
    }

    #[tokio::test]
    async fn run_checks_aggregates_pass_and_fail() {
        let result = run_checks(&[echo_ok_check(), echo_fail_check()]).await;
        assert!(!result.passed);
        assert_eq!(result.failed_checks, vec!["fail".to_string()]);
        assert_eq!(result.check_results.len(), 2);
    }

    #[tokio::test]
    async fn failing_test_blocks_completion_via_command_verifier() {
        let verifier = CommandVerifier::new();
        let diff = ArtifactRef::new(sakha_core::ArtifactKind::Diff, "hash", 0);
        let result = verifier.verify_diff(diff, vec![echo_fail_check()]).await.unwrap();
        assert!(!result.passed);
    }

    #[tokio::test]
    async fn passing_checks_complete_goal_via_command_verifier() {
        let verifier = CommandVerifier::new();
        let diff = ArtifactRef::new(sakha_core::ArtifactKind::Diff, "hash", 0);
        let result = verifier.verify_diff(diff, vec![echo_ok_check()]).await.unwrap();
        assert!(result.passed);
    }

    #[tokio::test]
    async fn timeout_is_treated_as_failure_not_hang() {
        let mut check = if cfg!(windows) {
            CheckSpec::new("slow", vec!["cmd".into(), "/C".into(), "ping".into(), "-n".into(), "5".into(), "127.0.0.1".into()])
        } else {
            CheckSpec::new("slow", vec!["sleep".into(), "5".into()])
        };
        check.timeout_secs = 1;
        let result = run_check(&check).await;
        assert!(!result.passed);
        assert!(result.stderr_tail.contains("timed out"));
    }

    #[tokio::test]
    async fn empty_rubric_passes_by_default() {
        let verifier = CommandVerifier::new();
        let artifact = ArtifactRef::new(sakha_core::ArtifactKind::Text, "hash", 0);
        let result = verifier.verify_artifact(artifact, Rubric::default()).await.unwrap();
        assert!(result.passed);
    }

    #[tokio::test]
    async fn non_empty_rubric_without_judge_is_not_fabricated_as_passed() {
        let verifier = CommandVerifier::new();
        let artifact = ArtifactRef::new(sakha_core::ArtifactKind::Text, "hash", 0);
        let rubric = Rubric { criteria: vec!["docs updated".into()] };
        let result = verifier.verify_artifact(artifact, rubric).await.unwrap();
        assert!(!result.passed);
    }
}
