//! sakha-cli: command-line entry point for the Sakha Coding Agent.
//!
//! Public API — see spec `modules/13-ui-cli-tui-web-desktop.md`
//! "CLI Commands" and `crates/crate-work-breakdown.md`.

mod commands;
mod config;
mod output;
mod runtime;
#[cfg(test)]
mod test_support;

use clap::{Parser, Subcommand};

use commands::chat::ChatArgs;
use commands::config::ConfigCommand;
use commands::daemon::DaemonCommand;
use commands::eval::EvalRunArgs;
use commands::loops::LoopCommand;
use commands::mcp::McpAddArgs;
use commands::memory::MemorySearchArgs;
use commands::providers::ProvidersListArgs;
use commands::run::RunArgs;
use commands::session::SessionCommand;
use commands::tools::ToolsListArgs;
use output::OutputFormat;

/// Sakha Coding Agent command-line interface.
#[derive(Debug, Parser)]
#[command(name = "sakha", version, about = "Sakha Coding Agent CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a single non-interactive prompt.
    Run(RunArgs),
    /// Start an interactive chat session.
    Chat(ChatArgs),
    /// Goal/session inspection commands.
    #[command(subcommand)]
    Goal(SessionCommand),
    /// Session inspection commands (list/show/resume).
    #[command(subcommand)]
    Session(SessionCommand),
    /// Long-running loop management.
    #[command(subcommand)]
    Loop(LoopCommand),
    /// List configured providers and their capabilities/health.
    Providers {
        #[command(subcommand)]
        command: ProvidersCommand,
    },
    /// List available tools.
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    /// Search durable memory.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Manage MCP server connections.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Read/write local configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Run evaluation suites.
    Eval {
        #[command(subcommand)]
        command: EvalCommand,
    },
    /// Local daemon management.
    #[command(subcommand)]
    Daemon(DaemonCommand),
    /// Diagnose the local Sakha installation.
    Doctor {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        output: OutputFormat,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProvidersCommand {
    List(ProvidersListArgs),
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    List(ToolsListArgs),
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    Search(MemorySearchArgs),
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    Add(McpAddArgs),
}

#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    Run(EvalRunArgs),
}

fn main() {
    let _ = tracing_subscriber::fmt::try_init();
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Command::Run(args) => commands::run::execute(args),
        Command::Chat(args) => commands::chat::execute(args),
        Command::Goal(cmd) => commands::session::execute(cmd),
        Command::Session(cmd) => commands::session::execute(cmd),
        Command::Loop(cmd) => commands::loops::execute(cmd),
        Command::Providers { command } => match command {
            ProvidersCommand::List(args) => commands::providers::execute(args),
        },
        Command::Tools { command } => match command {
            ToolsCommand::List(args) => commands::tools::execute(args),
        },
        Command::Memory { command } => match command {
            MemoryCommand::Search(args) => commands::memory::execute(args),
        },
        Command::Mcp { command } => match command {
            McpCommand::Add(args) => commands::mcp::execute(args),
        },
        Command::Config(cmd) => commands::config::execute(cmd),
        Command::Eval { command } => match command {
            EvalCommand::Run(args) => commands::eval::execute(args),
        },
        Command::Daemon(cmd) => commands::daemon::execute(cmd),
        Command::Doctor { output } => commands::doctor::execute(output),
    };

    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_command_parses_prompt_argument() {
        let cli = Cli::try_parse_from(["sakha", "run", "fix the bug"]).unwrap();
        match cli.command {
            Command::Run(args) => assert_eq!(args.prompt, "fix the bug"),
            _ => panic!("expected Run command"),
        }
    }

    #[test]
    fn chat_command_parses_optional_session_id() {
        let cli = Cli::try_parse_from(["sakha", "chat", "--session-id", "abc-123"]).unwrap();
        match cli.command {
            Command::Chat(args) => assert_eq!(args.session_id.as_deref(), Some("abc-123")),
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn session_list_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "session", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Session(SessionCommand::List(_))));
    }

    #[test]
    fn loop_start_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "loop", "start", "ship it"]).unwrap();
        match cli.command {
            Command::Loop(LoopCommand::Start(args)) => assert_eq!(args.objective, "ship it"),
            _ => panic!("expected Loop Start command"),
        }
    }

    #[test]
    fn config_get_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "config", "get", "provider.model"]).unwrap();
        match cli.command {
            Command::Config(ConfigCommand::Get(args)) => assert_eq!(args.key, "provider.model"),
            _ => panic!("expected Config Get command"),
        }
    }

    #[test]
    fn providers_list_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "providers", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Providers { .. }));
    }

    #[test]
    fn tools_list_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "tools", "list"]).unwrap();
        assert!(matches!(cli.command, Command::Tools { .. }));
    }

    #[test]
    fn memory_search_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "memory", "search", "auth bug"]).unwrap();
        match cli.command {
            Command::Memory { command: MemoryCommand::Search(args) } => assert_eq!(args.query, "auth bug"),
            _ => panic!("expected Memory Search command"),
        }
    }

    #[test]
    fn mcp_add_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "mcp", "add", "server1", "--url", "http://localhost:9000"]).unwrap();
        match cli.command {
            Command::Mcp { command: McpCommand::Add(args) } => assert_eq!(args.name, "server1"),
            _ => panic!("expected Mcp Add command"),
        }
    }

    #[test]
    fn eval_run_subcommand_parses_default_suite() {
        let cli = Cli::try_parse_from(["sakha", "eval", "run"]).unwrap();
        match cli.command {
            Command::Eval { command: EvalCommand::Run(args) } => assert_eq!(args.suite, "smoke"),
            _ => panic!("expected Eval Run command"),
        }
    }

    #[test]
    fn daemon_start_subcommand_parses() {
        let cli = Cli::try_parse_from(["sakha", "daemon", "start"]).unwrap();
        assert!(matches!(cli.command, Command::Daemon(DaemonCommand::Start(_))));
    }

    #[test]
    fn doctor_command_parses_with_default_output() {
        let cli = Cli::try_parse_from(["sakha", "doctor"]).unwrap();
        match cli.command {
            Command::Doctor { output } => assert_eq!(output, OutputFormat::Text),
            _ => panic!("expected Doctor command"),
        }
    }

    #[test]
    fn run_command_json_output_flag_parses() {
        let cli = Cli::try_parse_from(["sakha", "run", "do it", "--output", "json"]).unwrap();
        match cli.command {
            Command::Run(args) => assert_eq!(args.output, OutputFormat::Json),
            _ => panic!("expected Run command"),
        }
    }
}
