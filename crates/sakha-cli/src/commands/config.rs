//! `sakha config get/set`: reads/writes `~/.sakha/config.toml`.

use clap::{Args, Subcommand};

use crate::config::{default_config_path, get_key, load_config, save_config, set_key};
use crate::output::{print_error, OutputFormat};

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Get(ConfigGetArgs),
    Set(ConfigSetArgs),
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    pub key: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    pub key: String,
    pub value: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

pub fn execute(command: ConfigCommand) -> i32 {
    let path = default_config_path();
    match command {
        ConfigCommand::Get(args) => {
            let config = match load_config(&path) {
                Ok(c) => c,
                Err(err) => {
                    print_error(&format!("failed to load config: {err}"), args.output);
                    return 2;
                }
            };
            match get_key(&config, &args.key) {
                Some(value) => {
                    match args.output {
                        OutputFormat::Json => {
                            let payload = serde_json::json!({ "key": args.key, "value": value });
                            println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                        }
                        OutputFormat::Text => println!("{value}"),
                    }
                    0
                }
                None => {
                    print_error(&format!("key not found: {}", args.key), args.output);
                    3
                }
            }
        }
        ConfigCommand::Set(args) => {
            let mut config = match load_config(&path) {
                Ok(c) => c,
                Err(err) => {
                    print_error(&format!("failed to load config: {err}"), args.output);
                    return 2;
                }
            };
            if let Err(err) = set_key(&mut config, &args.key, &args.value) {
                print_error(&err.to_string(), args.output);
                return 1;
            }
            if let Err(err) = save_config(&path, &config) {
                print_error(&format!("failed to save config: {err}"), args.output);
                return 1;
            }
            match args.output {
                OutputFormat::Json => {
                    let payload = serde_json::json!({ "key": args.key, "value": args.value, "path": path.display().to_string() });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
                }
                OutputFormat::Text => println!("set {} = {} ({})", args.key, args.value, path.display()),
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_home<T>(f: impl FnOnce() -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        let prev_profile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());
        let result = f();
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        result
    }

    #[test]
    fn set_then_get_round_trips() {
        with_temp_home(|| {
            let set_code = execute(ConfigCommand::Set(ConfigSetArgs {
                key: "provider.model".into(),
                value: "gpt-4o-mini".into(),
                output: OutputFormat::Json,
            }));
            assert_eq!(set_code, 0);

            let get_code = execute(ConfigCommand::Get(ConfigGetArgs {
                key: "provider.model".into(),
                output: OutputFormat::Json,
            }));
            assert_eq!(get_code, 0);
        });
    }

    #[test]
    fn get_missing_key_returns_not_found_code() {
        with_temp_home(|| {
            let code = execute(ConfigCommand::Get(ConfigGetArgs {
                key: "nonexistent.key".into(),
                output: OutputFormat::Json,
            }));
            assert_eq!(code, 3);
        });
    }
}
