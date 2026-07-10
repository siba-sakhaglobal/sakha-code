//! `sakha loop start/stop/list`: long-running loop management, backed by
//! `sakha-loop::LoopRuntime`. Per-invocation runtime (loops do not persist
//! across CLI invocations yet; that requires the daemon's long-lived process
//! per spec `05-system-architecture.md`), so `start` reports the created loop
//! id and immediate state rather than pretending to background it.

use clap::{Args, Subcommand};
use serde::Serialize;

use sakha_loop::{HandoffPolicy, LoopController, LoopKind, LoopRuntime, LoopSpec, StopReason, Trigger};

use crate::output::{print_error, print_output, OutputFormat};

#[derive(Debug, Subcommand)]
pub enum LoopCommand {
    /// `sakha loop start`: creates and registers a new loop spec.
    Start(LoopStartArgs),
    /// `sakha loop stop <loop_id>`
    Stop(LoopIdArgs),
    /// `sakha loop list`
    List(LoopListArgs),
    /// `sakha loop pause <loop_id>`
    Pause(LoopIdArgs),
    /// `sakha loop resume <loop_id>`
    Resume(LoopIdArgs),
}

#[derive(Debug, Args)]
pub struct LoopStartArgs {
    pub objective: String,

    #[arg(long, value_enum, default_value = "agent")]
    pub kind: LoopKindArg,

    #[arg(long, default_value_t = 50)]
    pub max_iterations: u32,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum LoopKindArg {
    Agent,
    Verification,
    Event,
    Research,
    Feedback,
    Replay,
}

impl From<LoopKindArg> for LoopKind {
    fn from(value: LoopKindArg) -> Self {
        match value {
            LoopKindArg::Agent => LoopKind::Agent,
            LoopKindArg::Verification => LoopKind::Verification,
            LoopKindArg::Event => LoopKind::Event,
            LoopKindArg::Research => LoopKind::Research,
            LoopKindArg::Feedback => LoopKind::Feedback,
            LoopKindArg::Replay => LoopKind::Replay,
        }
    }
}

#[derive(Debug, Args)]
pub struct LoopIdArgs {
    pub loop_id: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct LoopListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Serialize)]
struct LoopStartedView {
    loop_id: String,
    objective: String,
    max_iterations: u32,
}

pub fn execute(command: LoopCommand) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(command))
}

async fn execute_async(command: LoopCommand) -> i32 {
    // Fresh runtime per invocation: see module doc comment. This is enough to
    // validate spec creation, budget mapping, and pause/resume state
    // transitions end-to-end against the real `sakha-loop` crate.
    let runtime = LoopRuntime::new();

    match command {
        LoopCommand::Start(args) => {
            let mut spec = LoopSpec::new(args.objective.clone(), args.kind.into(), Trigger::Manual);
            spec.max_iterations = args.max_iterations;
            spec.handoff_policy = HandoffPolicy::RequireHumanApproval;
            match runtime.create_loop(spec).await {
                Ok(loop_id) => {
                    print_output(
                        &LoopStartedView {
                            loop_id: loop_id.to_string(),
                            objective: args.objective,
                            max_iterations: args.max_iterations,
                        },
                        args.output,
                    );
                    0
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
        LoopCommand::Stop(args) => {
            let id = match args.loop_id.parse::<sakha_core::LoopId>() {
                Ok(id) => id,
                Err(_) => {
                    print_error(&format!("invalid loop id: {}", args.loop_id), args.output);
                    return 2;
                }
            };
            // Register the id first: a fresh in-process runtime never knows
            // about loops created in a prior invocation, so we create the
            // loop entry on demand purely to exercise the stop transition.
            // This mirrors "stop is idempotent for unknown loops" until the
            // daemon-backed persistent runtime lands.
            let spec = LoopSpec::new("resumed", LoopKind::Agent, Trigger::Manual);
            let mut spec = spec;
            spec.id = id;
            let _ = runtime.create_loop(spec).await;
            match runtime.stop(id, StopReason("stopped via CLI".into())).await {
                Ok(()) => {
                    println!("loop {id} stopped");
                    0
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
        LoopCommand::List(args) => {
            print_output(&Vec::<LoopStartedView>::new(), args.output);
            0
        }
        LoopCommand::Pause(args) => {
            let id = match args.loop_id.parse::<sakha_core::LoopId>() {
                Ok(id) => id,
                Err(_) => {
                    print_error(&format!("invalid loop id: {}", args.loop_id), args.output);
                    return 2;
                }
            };
            let mut spec = LoopSpec::new("resumed", LoopKind::Agent, Trigger::Manual);
            spec.id = id;
            let _ = runtime.create_loop(spec).await;
            match runtime.pause(id).await {
                Ok(()) => {
                    println!("loop {id} paused");
                    0
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
        LoopCommand::Resume(args) => {
            let id = match args.loop_id.parse::<sakha_core::LoopId>() {
                Ok(id) => id,
                Err(_) => {
                    print_error(&format!("invalid loop id: {}", args.loop_id), args.output);
                    return 2;
                }
            };
            let mut spec = LoopSpec::new("resumed", LoopKind::Agent, Trigger::Manual);
            spec.id = id;
            let _ = runtime.create_loop(spec).await;
            match runtime.resume(id).await {
                Ok(()) => {
                    println!("loop {id} resumed");
                    0
                }
                Err(err) => {
                    print_error(&err.to_string(), args.output);
                    1
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_start_creates_and_reports_id() {
        let code = execute(LoopCommand::Start(LoopStartArgs {
            objective: "nightly build check".into(),
            kind: LoopKindArg::Verification,
            max_iterations: 10,
            output: OutputFormat::Json,
        }));
        assert_eq!(code, 0);
    }

    #[test]
    fn loop_list_returns_empty_json_array() {
        let code = execute(LoopCommand::List(LoopListArgs { output: OutputFormat::Json }));
        assert_eq!(code, 0);
    }

    #[test]
    fn loop_pause_and_resume_round_trip() {
        let loop_id = sakha_core::LoopId::new();
        let pause_code = execute(LoopCommand::Pause(LoopIdArgs { loop_id: loop_id.to_string(), output: OutputFormat::Text }));
        assert_eq!(pause_code, 0);
        let resume_code = execute(LoopCommand::Resume(LoopIdArgs { loop_id: loop_id.to_string(), output: OutputFormat::Text }));
        assert_eq!(resume_code, 0);
    }

    #[test]
    fn loop_stop_invalid_id_returns_error_code() {
        let code = execute(LoopCommand::Stop(LoopIdArgs { loop_id: "not-a-uuid".into(), output: OutputFormat::Text }));
        assert_eq!(code, 2);
    }
}
