//! `SkillPack`: named, versioned prompt fragments loadable from
//! `~/.sakha/skills/` and workspace `.sakha/skills/`. See spec
//! `modules/21-prompt-system.md`.
//!
//! On-disk layout: each skill pack is a directory (or a single `.toml` file)
//! under the skills root containing a `skill.toml`:
//!
//! ```toml
//! name = "review"
//! version = "1"
//! required_tools = ["git.diff"]
//!
//! [[sections]]
//! id = "review_policy"
//! priority = 120
//! volatile = false
//! content = "Review diffs for correctness before style."
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use sakha_core::SakhaResult;

use super::assembler::PromptSection;

/// A named, versioned bundle of prompt sections plus the tools it requires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPack {
    pub name: String,
    pub version: String,
    pub sections: Vec<PromptSection>,
    pub required_tools: Vec<String>,
}

/// Where a loaded `SkillPack` came from, for override precedence
/// (workspace overrides user overrides builtin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPackOrigin {
    Builtin,
    User,
    Workspace,
}

/// Non-fatal problem encountered while loading skill packs from disk. Per
/// spec "Missing skill pack -> warn and continue"; the same policy applies to
/// a skill pack directory that exists but fails to parse. Collected by
/// `SkillPackLoader::load_from_dir` rather than returned as an `Err`, so one
/// bad pack never blocks loading the rest.
#[derive(Debug, Clone)]
pub struct SkillPackLoadWarning {
    pub path: PathBuf,
    pub message: String,
}

/// Loads and resolves skill packs by name, applying override precedence.
#[derive(Default)]
pub struct SkillPackLoader {
    packs: Vec<(SkillPackOrigin, SkillPack)>,
}

impl SkillPackLoader {
    pub fn new() -> Self {
        Self { packs: Vec::new() }
    }

    pub fn register(&mut self, origin: SkillPackOrigin, pack: SkillPack) {
        self.packs.push((origin, pack));
    }

    /// Resolves the effective `SkillPack` for `name`: highest-precedence
    /// origin wins (workspace > user > builtin). Missing skill pack returns
    /// `Ok(None)` — per spec, a missing skill pack must warn and continue,
    /// never fail the turn.
    pub fn resolve(&self, name: &str) -> SakhaResult<Option<&SkillPack>> {
        let mut best: Option<&(SkillPackOrigin, SkillPack)> = None;
        for entry in &self.packs {
            if entry.1.name != name {
                continue;
            }
            if best.map(|b| entry.0 > b.0).unwrap_or(true) {
                best = Some(entry);
            }
        }
        Ok(best.map(|(_, pack)| pack))
    }

    /// All distinct skill pack names currently registered (any origin).
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.packs.iter().map(|(_, p)| p.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Loads every skill pack directory found under `root` (non-recursive:
    /// one level of subdirectories, each expected to contain `skill.toml`;
    /// also accepts loose `*.toml` files directly under `root`) and
    /// registers them with `origin`. Returns warnings for any entry that
    /// could not be parsed — it never fails the whole load, per spec.
    pub fn load_from_dir(&mut self, root: &Path, origin: SkillPackOrigin) -> Vec<SkillPackLoadWarning> {
        let mut warnings = Vec::new();
        if !root.is_dir() {
            return warnings;
        }

        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) => {
                warnings.push(SkillPackLoadWarning { path: root.to_path_buf(), message: err.to_string() });
                return warnings;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let toml_path = if path.is_dir() {
                path.join("skill.toml")
            } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                path.clone()
            } else {
                continue;
            };
            if !toml_path.is_file() {
                continue;
            }
            match load_skill_pack_file(&toml_path) {
                Ok(pack) => self.register(origin, pack),
                Err(message) => warnings.push(SkillPackLoadWarning { path: toml_path, message }),
            }
        }

        warnings
    }

    /// Loads builtin, then user (`~/.sakha/skills/`), then workspace
    /// (`<workspace_root>/.sakha/skills/`) skill packs, in that order, so
    /// later registrations naturally win ties at equal precedence while
    /// `resolve` still enforces workspace > user > builtin regardless of
    /// load order. `workspace_root` is the project root Sakha is operating
    /// on (not the current working directory of the daemon process).
    pub fn load_standard_locations(&mut self, workspace_root: Option<&Path>) -> Vec<SkillPackLoadWarning> {
        let mut warnings = Vec::new();
        if let Some(home) = dirs::home_dir() {
            warnings.extend(self.load_from_dir(&home.join(".sakha").join("skills"), SkillPackOrigin::User));
        }
        if let Some(root) = workspace_root {
            warnings.extend(self.load_from_dir(&root.join(".sakha").join("skills"), SkillPackOrigin::Workspace));
        }
        warnings
    }
}

fn load_skill_pack_file(path: &Path) -> Result<SkillPack, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    toml::from_str::<SkillPack>(&text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::assembler::SectionId;

    #[test]
    fn workspace_skill_pack_overrides_user_and_builtin() {
        let mut loader = SkillPackLoader::new();
        loader.register(
            SkillPackOrigin::Builtin,
            SkillPack { name: "review".into(), version: "1".into(), sections: vec![], required_tools: vec![] },
        );
        loader.register(
            SkillPackOrigin::User,
            SkillPack { name: "review".into(), version: "2".into(), sections: vec![], required_tools: vec![] },
        );
        loader.register(
            SkillPackOrigin::Workspace,
            SkillPack { name: "review".into(), version: "3".into(), sections: vec![], required_tools: vec![] },
        );
        let resolved = loader.resolve("review").unwrap().unwrap();
        assert_eq!(resolved.version, "3");
    }

    #[test]
    fn missing_skill_pack_returns_none_not_error() {
        let loader = SkillPackLoader::new();
        let resolved = loader.resolve("does-not-exist").unwrap();
        assert!(resolved.is_none());
    }

    #[test]
    fn load_from_dir_parses_directory_style_pack() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("review");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(
            pack_dir.join("skill.toml"),
            r#"
                name = "review"
                version = "1"
                required_tools = ["git.diff"]

                [[sections]]
                id = "review_policy"
                priority = 120
                volatile = false
                content = "Review diffs for correctness before style."
            "#,
        )
        .unwrap();

        let mut loader = SkillPackLoader::new();
        let warnings = loader.load_from_dir(dir.path(), SkillPackOrigin::Workspace);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

        let resolved = loader.resolve("review").unwrap().unwrap();
        assert_eq!(resolved.version, "1");
        assert_eq!(resolved.required_tools, vec!["git.diff".to_string()]);
        assert_eq!(resolved.sections.len(), 1);
        assert_eq!(resolved.sections[0].id, SectionId::new("review_policy"));
    }

    #[test]
    fn load_from_dir_parses_loose_toml_file_pack() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("triage.toml"),
            r#"
                name = "triage"
                version = "1"
                required_tools = []
                sections = []
            "#,
        )
        .unwrap();

        let mut loader = SkillPackLoader::new();
        let warnings = loader.load_from_dir(dir.path(), SkillPackOrigin::User);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert!(loader.resolve("triage").unwrap().is_some());
    }

    #[test]
    fn load_from_dir_warns_but_does_not_panic_on_malformed_pack() {
        let dir = tempfile::tempdir().unwrap();
        let pack_dir = dir.path().join("broken");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(pack_dir.join("skill.toml"), "not valid toml {{{").unwrap();

        let mut loader = SkillPackLoader::new();
        let warnings = loader.load_from_dir(dir.path(), SkillPackOrigin::Workspace);
        assert_eq!(warnings.len(), 1);
        assert!(loader.resolve("broken").unwrap().is_none());
    }

    #[test]
    fn load_from_dir_on_missing_root_returns_no_warnings_and_no_panic() {
        let mut loader = SkillPackLoader::new();
        let warnings = loader.load_from_dir(Path::new("/does/not/exist/at/all"), SkillPackOrigin::User);
        assert!(warnings.is_empty());
    }

    #[test]
    fn names_lists_distinct_registered_pack_names() {
        let mut loader = SkillPackLoader::new();
        loader.register(
            SkillPackOrigin::Builtin,
            SkillPack { name: "review".into(), version: "1".into(), sections: vec![], required_tools: vec![] },
        );
        loader.register(
            SkillPackOrigin::User,
            SkillPack { name: "review".into(), version: "2".into(), sections: vec![], required_tools: vec![] },
        );
        loader.register(
            SkillPackOrigin::Builtin,
            SkillPack { name: "triage".into(), version: "1".into(), sections: vec![], required_tools: vec![] },
        );
        assert_eq!(loader.names(), vec!["review", "triage"]);
    }
}
