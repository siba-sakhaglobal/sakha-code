//! `sakha skills list` / `sakha skills show <name>`: inspect discovered
//! agent skills (`SKILL.md` instruction packs). See `crate::skills` for the
//! discovery/parsing logic these commands wrap.

use clap::Args;
use serde::Serialize;

use crate::output::{print_error, print_output, OutputFormat};
use crate::skills::discover_skills;

#[derive(Debug, Args)]
pub struct SkillsListArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SkillsShowArgs {
    /// The skill's name (as shown by `sakha skills list`).
    pub name: String,
}

#[derive(Debug, Serialize)]
struct SkillView {
    name: String,
    tier: String,
    dir: String,
    description: String,
}

/// Lists every discovered skill: name, tier, directory, and the first line
/// of its description. Text mode renders a simple table; JSON mode emits the
/// full `SkillView` list.
pub fn execute_list(args: SkillsListArgs) -> i32 {
    let cwd = std::env::current_dir().ok();
    let (skills, warnings) = discover_skills(cwd.as_deref());

    for warning in &warnings {
        eprintln!("warning: {} ({})", warning.message, warning.path.display());
    }

    let views: Vec<SkillView> = skills
        .iter()
        .map(|s| SkillView {
            name: s.name.clone(),
            tier: s.tier.to_string(),
            dir: s.dir_path.display().to_string(),
            description: s.description.lines().next().unwrap_or("").to_string(),
        })
        .collect();

    match args.output {
        OutputFormat::Json => print_output(&views, OutputFormat::Json),
        OutputFormat::Text => {
            if views.is_empty() {
                println!("no skills discovered");
            } else {
                for view in &views {
                    println!("{}\t[{}]\t{}\t{}", view.name, view.tier, view.dir, view.description);
                }
            }
        }
    }

    0
}

/// Prints a skill's full `SKILL.md` body to stdout.
pub fn execute_show(args: SkillsShowArgs) -> i32 {
    let cwd = std::env::current_dir().ok();
    let (skills, _warnings) = discover_skills(cwd.as_deref());

    match skills.iter().find(|s| s.name == args.name) {
        Some(skill) => {
            println!("{}", skill.body);
            0
        }
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            print_error(&format!("unknown skill '{}'; available: [{}]", args.name, available.join(", ")), OutputFormat::Text);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_list_args_parse_default_text_output() {
        use clap::Parser;
        #[derive(Debug, Parser)]
        struct TestCli {
            #[command(flatten)]
            args: SkillsListArgs,
        }
        let cli = TestCli::try_parse_from(["sakha"]).unwrap();
        assert_eq!(cli.args.output, OutputFormat::Text);
    }

    #[test]
    fn execute_list_returns_success_with_no_skills() {
        let _home = crate::test_support::TempHome::new();
        let code = execute_list(SkillsListArgs { output: OutputFormat::Json });
        assert_eq!(code, 0);
    }

    #[test]
    fn execute_show_returns_error_for_unknown_skill() {
        let _home = crate::test_support::TempHome::new();
        let code = execute_show(SkillsShowArgs { name: "does-not-exist".into() });
        assert_eq!(code, 1);
    }

    #[test]
    fn execute_show_prints_body_for_known_skill() {
        let home = crate::test_support::TempHome::new();
        let skill_dir = home.path().join(".sakha").join("skills").join("demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: demo\ndescription: A demo skill.\n---\n\nDemo body text.\n").unwrap();

        let code = execute_show(SkillsShowArgs { name: "demo".into() });
        assert_eq!(code, 0);
    }
}
