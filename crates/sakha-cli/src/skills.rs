//! Agent skills: Claude-Code/Gemini-CLI-compatible `SKILL.md` instruction
//! packs, discovered from standard directories, listed to the model
//! compactly, and activated on demand via the `skill.activate` tool (or
//! injected up front via `--skill`). Distinct from
//! `sakha_context::prompt::skills::SkillPack` — that type loads versioned
//! `skill.toml` prompt-section bundles for the context assembler (spec
//! `modules/21-prompt-system.md`); this module loads freeform markdown
//! instruction packs meant to be handed to the model verbatim, matching the
//! on-disk format used by Claude Code and Gemini CLI so a skill directory
//! authored for either tool works here unmodified.
//!
//! On-disk layout: a skill is a directory containing `SKILL.md` with YAML
//! frontmatter followed by a markdown body:
//!
//! ```markdown
//! ---
//! name: review
//! description: Reviews a diff for correctness before style.
//! ---
//!
//! Review diffs for correctness before style...
//! ```
//!
//! Any other files in the skill directory are resources the skill body may
//! reference; `skill.activate` lists their relative paths so the model knows
//! what is available without reading the whole tree.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use sakha_core::{SakhaError, SakhaResult};
use sakha_security::PermissionKind;
use sakha_tools::{
    IdempotencyPolicy, Tool, ToolContext, ToolInputSchema, ToolName, ToolOutputSchema, ToolPermissionSpec, ToolPlan, ToolResult,
    ToolSpec, ToolSummary, ValidatedInput,
};

const MODULE: &str = "sakha-cli::skills";

/// Where a discovered skill came from, for override precedence. Later tiers
/// win on name conflict: workspace tiers (checked in this order) beat the
/// user tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillTier {
    /// `~/.agents/skills/` (or `{SAKHA_HOME}/.agents/skills/`).
    UserAgents,
    /// `~/.sakha/skills/` (or `{SAKHA_HOME}/.sakha/skills/`).
    UserSakha,
    /// `<cwd>/.gemini/skills/`.
    WorkspaceGemini,
    /// `<cwd>/.claude/skills/`.
    WorkspaceClaude,
    /// `<cwd>/.agents/skills/`.
    WorkspaceAgents,
    /// `<cwd>/.sakha/skills/`.
    WorkspaceSakha,
}

impl SkillTier {
    fn label(self) -> &'static str {
        match self {
            SkillTier::UserAgents => "user:.agents/skills",
            SkillTier::UserSakha => "user:.sakha/skills",
            SkillTier::WorkspaceGemini => "workspace:.gemini/skills",
            SkillTier::WorkspaceClaude => "workspace:.claude/skills",
            SkillTier::WorkspaceAgents => "workspace:.agents/skills",
            SkillTier::WorkspaceSakha => "workspace:.sakha/skills",
        }
    }
}

impl std::fmt::Display for SkillTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// A discovered, parsed skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The markdown body following the frontmatter — the instructions handed
    /// to the model verbatim on activation.
    pub body: String,
    /// Directory containing this skill's `SKILL.md` and any resource files.
    pub dir_path: PathBuf,
    pub tier: SkillTier,
}

impl Skill {
    /// Relative paths (POSIX-style, forward slashes) of every file in
    /// `dir_path` other than `SKILL.md` itself — the "resources" a skill body
    /// may reference. Non-recursive at depth beyond nested dirs is still
    /// walked, since resources are commonly organized into subfolders (e.g.
    /// `references/`, `assets/`).
    pub fn resources(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_resources(&self.dir_path, &self.dir_path, &mut out);
        out.sort();
        out
    }
}

fn collect_resources(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_resources(root, &path, out);
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") && path.parent() == Some(root) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Non-fatal problem encountered while discovering/parsing a skill. Per the
/// same "warn and continue" policy `SkillPackLoader` uses: one bad skill
/// directory never blocks loading the rest.
#[derive(Debug, Clone)]
pub struct SkillLoadWarning {
    pub path: PathBuf,
    pub message: String,
}

/// Scans every standard discovery tier and returns the resolved skill set
/// (name conflicts resolved by tier precedence — later tier in the list
/// wins) plus any warnings encountered along the way. `workspace_root` is
/// the directory skills are discovered relative to (typically the process
/// CWD); `None` skips the workspace tiers entirely.
pub fn discover_skills(workspace_root: Option<&Path>) -> (Vec<Skill>, Vec<SkillLoadWarning>) {
    let mut warnings = Vec::new();
    // User tier first, then workspace tiers in override order, per the spec
    // list `.sakha/skills/`, `.agents/skills/`, `.claude/skills/`,
    // `.gemini/skills/` (workspace) with user tier losing to all of them.
    let mut by_name: std::collections::BTreeMap<String, Skill> = std::collections::BTreeMap::new();

    let home = std::env::var_os("SAKHA_HOME").map(PathBuf::from).or_else(dirs::home_dir);
    if let Some(home) = &home {
        for (dir, tier) in [
            (home.join(".agents").join("skills"), SkillTier::UserAgents),
            (home.join(".sakha").join("skills"), SkillTier::UserSakha),
        ] {
            let (found, mut w) = scan_tier(&dir, tier);
            warnings.append(&mut w);
            for skill in found {
                by_name.insert(skill.name.clone(), skill);
            }
        }
    }

    if let Some(root) = workspace_root {
        // Scanned in reverse-priority order so the map insert for a
        // higher-priority tier happens last and naturally wins: spec says
        // "within workspace, first match wins in this order: .sakha/skills/,
        // .agents/skills/, .claude/skills/, .gemini/skills/" — i.e.
        // `.sakha` has the highest workspace precedence, `.gemini` the
        // lowest.
        for (dir, tier) in [
            (root.join(".gemini").join("skills"), SkillTier::WorkspaceGemini),
            (root.join(".claude").join("skills"), SkillTier::WorkspaceClaude),
            (root.join(".agents").join("skills"), SkillTier::WorkspaceAgents),
            (root.join(".sakha").join("skills"), SkillTier::WorkspaceSakha),
        ] {
            let (found, mut w) = scan_tier(&dir, tier);
            warnings.append(&mut w);
            for skill in found {
                by_name.insert(skill.name.clone(), skill);
            }
        }
    }

    let mut skills: Vec<Skill> = by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    (skills, warnings)
}

/// Scans one tier directory (non-recursive: each immediate subdirectory is
/// expected to contain a `SKILL.md`).
fn scan_tier(dir: &Path, tier: SkillTier) -> (Vec<Skill>, Vec<SkillLoadWarning>) {
    let mut skills = Vec::new();
    let mut warnings = Vec::new();

    if !dir.is_dir() {
        return (skills, warnings);
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            warnings.push(SkillLoadWarning { path: dir.to_path_buf(), message: err.to_string() });
            return (skills, warnings);
        }
    };

    for entry in entries.flatten() {
        let skill_dir = entry.path();
        if !skill_dir.is_dir() {
            continue;
        }
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let dir_name = skill_dir.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string();
        match load_skill_file(&skill_md, &dir_name, &skill_dir, tier) {
            Ok(Some(skill)) => skills.push(skill),
            Ok(None) => {
                // Missing description: warn and skip, per spec.
                warnings.push(SkillLoadWarning {
                    path: skill_md.clone(),
                    message: "skill has no description; skipping".to_string(),
                });
            }
            Err(message) => warnings.push(SkillLoadWarning { path: skill_md, message }),
        }
    }

    (skills, warnings)
}

fn load_skill_file(path: &Path, dir_name: &str, dir_path: &Path, tier: SkillTier) -> Result<Option<Skill>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (frontmatter, body) = split_frontmatter(&text)?;
    let fields = parse_frontmatter(&frontmatter);

    let name = fields.get("name").cloned().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| dir_name.to_string());
    let Some(description) = fields.get("description").cloned().filter(|d| !d.trim().is_empty()) else {
        return Ok(None);
    };

    Ok(Some(Skill { name, description, body: body.trim().to_string(), dir_path: dir_path.to_path_buf(), tier }))
}

/// Splits a `SKILL.md` file into its YAML frontmatter block and markdown
/// body. The frontmatter block must open and close with a line containing
/// exactly `---` (tolerating CRLF line endings). Files with no frontmatter
/// delimiter at all are treated as having empty frontmatter and the whole
/// file as body (still tolerated — only `description` is hard-required, and
/// that will simply be absent).
fn split_frontmatter(text: &str) -> Result<(String, String), String> {
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.splitn(2, '\n');
    let first_line = lines.next().unwrap_or("");
    if first_line.trim_end() != "---" {
        return Ok((String::new(), normalized));
    }
    let rest = lines.next().unwrap_or("");
    match rest.find("\n---") {
        Some(idx) => {
            // Confirm the delimiter is on its own line (followed by newline
            // or end-of-string, ignoring trailing spaces).
            let after = &rest[idx + 4..];
            let end_of_delim = after.find('\n').map(|i| idx + 4 + i).unwrap_or(rest.len());
            let delim_line = rest[idx + 1..end_of_delim].trim_end();
            if delim_line != "---" {
                return Err("frontmatter closing delimiter not found".to_string());
            }
            let frontmatter = rest[..idx].to_string();
            let body = if end_of_delim < rest.len() { rest[end_of_delim + 1..].to_string() } else { String::new() };
            Ok((frontmatter, body))
        }
        None => Err("frontmatter opened with '---' but never closed".to_string()),
    }
}

/// Hand-rolled `key: value` frontmatter parser — deliberately not a YAML
/// crate per spec (no new heavy deps). Supports the two fields skills
/// actually use (`name`, `description`); `description` may span multiple
/// lines via YAML block-scalar style (`description: >` or `description: |`
/// followed by indented continuation lines) or simple folded lines that are
/// indented continuations of the previous `key:` line. Unknown keys are
/// ignored (forward-compatible with richer frontmatter other tools may add).
fn parse_frontmatter(text: &str) -> std::collections::HashMap<String, String> {
    let mut fields = std::collections::HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_value = String::new();

    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let is_indented_continuation = line.starts_with(' ') || line.starts_with('\t');

        if is_indented_continuation && current_key.is_some() {
            if !current_value.is_empty() {
                current_value.push(' ');
            }
            current_value.push_str(line.trim());
            continue;
        }

        // Flush the previous key before starting a new one.
        if let Some(key) = current_key.take() {
            fields.insert(key, current_value.trim().to_string());
            current_value.clear();
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else { continue };
        let key = key.trim().to_string();
        let mut value = value.trim().to_string();
        // Strip a YAML block-scalar indicator (`>` folded, `|` literal) —
        // the continuation lines below carry the actual text.
        if value == ">" || value == "|" || value == ">-" || value == "|-" {
            value.clear();
        }
        // Strip surrounding quotes for simple quoted scalars.
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        current_key = Some(key);
        current_value = value;
    }
    if let Some(key) = current_key {
        fields.insert(key, current_value.trim().to_string());
    }

    fields
}

/// Truncates `s` to at most `max_chars` characters (char-boundary safe),
/// appending `...` when truncated. Used to keep the system-prompt skill
/// listing compact per spec ("descriptions truncated to 200 chars").
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{truncated}...")
}

/// Builds the single system message advertising available skills, per spec:
/// "You have access to these skills. When a task matches a skill's
/// description, call skill.activate with its name and follow the returned
/// instructions." followed by one `<name>: <description>` line per skill
/// (descriptions truncated to 200 chars). Returns `None` when `skills` is
/// empty — zero skills means no system message at all.
pub fn build_skills_system_message(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut message = String::from(
        "You have access to these skills. When a task matches a skill's description, call skill.activate with its name and follow the returned instructions.",
    );
    for skill in skills {
        message.push('\n');
        message.push_str(&skill.name);
        message.push_str(": ");
        message.push_str(&truncate_chars(&skill.description, 200));
    }
    Some(message)
}

/// Builds the ordered list of system messages `run`/`chat` should prepend to
/// a fresh request, given the discovered `skills` and an optional explicit
/// `--skill <name>` selection:
///   1. The compact skills-listing message from `build_skills_system_message`
///      (omitted when `skills` is empty), so the model knows what is
///      available and can call `skill.activate` on demand.
///   2. When `explicit_skill` names a known skill, that skill's *full* body
///      as a second system message — no activation round-trip needed, per
///      spec `--skill` flag behavior. When `explicit_skill` names an unknown
///      skill, returns `Err` listing available names so the caller can fail
///      fast rather than silently ignoring a typo'd flag.
pub fn build_startup_system_messages(skills: &[Skill], explicit_skill: Option<&str>) -> Result<Vec<String>, String> {
    let mut messages = Vec::new();
    if let Some(listing) = build_skills_system_message(skills) {
        messages.push(listing);
    }
    if let Some(name) = explicit_skill {
        match skills.iter().find(|s| s.name == name) {
            Some(skill) => messages.push(skill.body.clone()),
            None => {
                let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                return Err(format!("unknown skill '{name}'; available skills: [{}]", available.join(", ")));
            }
        }
    }
    Ok(messages)
}

/// `skill.activate`: the model calls this with `{"name": "<skill-name>"}` to
/// pull a discovered skill's full instructions into the conversation.
/// Discovery is re-run against `workspace_root` at call time (rather than
/// snapshotted at registry-build time) so skills added to disk mid-session
/// are picked up without a restart — cheap, since it is only a filesystem
/// walk of a few small directories.
pub struct SkillActivateTool;

#[async_trait]
impl Tool for SkillActivateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ToolName::new("skill.activate"),
            description: "Activate a discovered agent skill by name, returning its full instructions and any resource file paths. Call this when a task matches a skill's description from the system prompt.".to_string(),
            input_schema: ToolInputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                },
                "required": ["name"]
            })),
            output_schema: ToolOutputSchema(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "instructions": {"type": "string"},
                    "resources": {"type": "array"}
                }
            })),
            permission_spec: ToolPermissionSpec { required: vec![PermissionKind::FileRead] },
            idempotency_policy: IdempotencyPolicy::Idempotent,
        }
    }

    fn validate(&self, input: serde_json::Value) -> SakhaResult<ValidatedInput> {
        sakha_tools::schema::validate_against_schema(MODULE, &input, &self.spec().input_schema.0)?;
        let name = input.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.trim().is_empty() {
            return Err(SakhaError::invalid_input(MODULE, "name must not be empty"));
        }
        Ok(ValidatedInput(input))
    }

    async fn plan(&self, input: &ValidatedInput, _context: &ToolContext) -> SakhaResult<ToolPlan> {
        let name = input.0.get("name").and_then(|n| n.as_str()).unwrap_or_default();
        Ok(ToolPlan { summary: format!("activate skill: {name}"), affected_paths: Vec::new(), is_destructive: false })
    }

    async fn execute(&self, input: &ValidatedInput, context: &ToolContext) -> SakhaResult<ToolResult> {
        let name = input.0.get("name").and_then(|n| n.as_str()).ok_or_else(|| SakhaError::invalid_input(MODULE, "missing name"))?;

        let (skills, _warnings) = discover_skills(Some(&context.workspace_root));
        let Some(skill) = skills.iter().find(|s| s.name == name) else {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return Err(SakhaError::invalid_input(MODULE, format!("unknown skill '{name}'; available skills: [{}]", available.join(", "))));
        };

        Ok(ToolResult::success(serde_json::json!({
            "name": skill.name,
            "instructions": skill.body,
            "resources": skill.resources(),
        })))
    }

    fn summarize(&self, result: &ToolResult) -> ToolSummary {
        let name = result.output_json.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
        ToolSummary { text: format!("activated skill: {name}"), truncated: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), contents).unwrap();
        skill_dir
    }

    #[test]
    fn parses_frontmatter_with_explicit_name() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "review",
            "---\nname: review\ndescription: Reviews diffs for correctness.\n---\n\nDo the review thing.\n",
        );
        let (skills, warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "review");
        assert_eq!(skills[0].description, "Reviews diffs for correctness.");
        assert_eq!(skills[0].body, "Do the review thing.");
    }

    #[test]
    fn missing_name_falls_back_to_directory_name() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "triage", "---\ndescription: Triages bugs.\n---\n\nTriage instructions.\n");
        let (skills, warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        assert!(warnings.is_empty());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "triage");
    }

    #[test]
    fn missing_description_is_skipped_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "no-desc", "---\nname: no-desc\n---\n\nBody only.\n");
        let (skills, warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        assert!(skills.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn tolerates_crlf_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let content = "---\r\nname: crlf-skill\r\ndescription: Works with CRLF.\r\n---\r\n\r\nCRLF body text.\r\n";
        write_skill(dir.path(), "crlf-skill", content);
        let (skills, warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "crlf-skill");
        assert_eq!(skills[0].description, "Works with CRLF.");
        assert_eq!(skills[0].body, "CRLF body text.");
    }

    #[test]
    fn folded_multiline_description_is_joined() {
        let dir = tempfile::tempdir().unwrap();
        let content = "---\nname: multi\ndescription: >\n  First line of the description\n  continues here.\n---\n\nBody.\n";
        write_skill(dir.path(), "multi", content);
        let (skills, warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(skills[0].description, "First line of the description continues here.");
    }

    #[test]
    fn workspace_tier_overrides_user_tier_on_name_conflict() {
        let user_home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();

        write_skill(
            &user_home.path().join(".sakha").join("skills"),
            "review",
            "---\nname: review\ndescription: user tier version\n---\n\nUser body.\n",
        );
        write_skill(
            &workspace.path().join(".sakha").join("skills"),
            "review",
            "---\nname: review\ndescription: workspace tier version\n---\n\nWorkspace body.\n",
        );

        let prev_home = std::env::var_os("SAKHA_HOME");
        std::env::set_var("SAKHA_HOME", user_home.path());
        let (skills, _warnings) = discover_skills(Some(workspace.path()));
        match prev_home {
            Some(v) => std::env::set_var("SAKHA_HOME", v),
            None => std::env::remove_var("SAKHA_HOME"),
        }

        let review = skills.iter().find(|s| s.name == "review").unwrap();
        assert_eq!(review.description, "workspace tier version");
        assert_eq!(review.tier, SkillTier::WorkspaceSakha);
    }

    #[test]
    fn tier_precedence_order_dot_sakha_beats_dot_gemini_in_workspace() {
        // Spec: "within workspace, first match wins in this order:
        // `.sakha/skills/`, `.agents/skills/`, `.claude/skills/`,
        // `.gemini/skills/`" — `.sakha` has the highest workspace
        // precedence, `.gemini` the lowest.
        let workspace = tempfile::tempdir().unwrap();
        write_skill(
            &workspace.path().join(".sakha").join("skills"),
            "dup",
            "---\nname: dup\ndescription: from .sakha\n---\n\nsakha body\n",
        );
        write_skill(
            &workspace.path().join(".gemini").join("skills"),
            "dup",
            "---\nname: dup\ndescription: from .gemini\n---\n\ngemini body\n",
        );

        let (skills, _warnings) = discover_skills(Some(workspace.path()));
        let dup = skills.iter().find(|s| s.name == "dup").unwrap();
        assert_eq!(dup.description, "from .sakha");
        assert_eq!(dup.tier, SkillTier::WorkspaceSakha);
    }

    #[test]
    fn resources_lists_files_other_than_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = write_skill(dir.path(), "with-resources", "---\nname: with-resources\ndescription: has extras\n---\n\nBody.\n");
        std::fs::write(skill_dir.join("reference.md"), "extra info").unwrap();
        std::fs::create_dir_all(skill_dir.join("assets")).unwrap();
        std::fs::write(skill_dir.join("assets").join("logo.svg"), "<svg/>").unwrap();

        let (skills, _warnings) = scan_tier(dir.path(), SkillTier::WorkspaceSakha);
        let skill = &skills[0];
        let resources = skill.resources();
        assert_eq!(resources, vec!["assets/logo.svg".to_string(), "reference.md".to_string()]);
    }

    #[test]
    fn build_skills_system_message_none_when_empty() {
        assert!(build_skills_system_message(&[]).is_none());
    }

    #[test]
    fn build_skills_system_message_lists_names_and_truncated_descriptions() {
        let long_desc = "x".repeat(250);
        let skills = vec![
            Skill { name: "alpha".into(), description: "short desc".into(), body: String::new(), dir_path: PathBuf::new(), tier: SkillTier::WorkspaceSakha },
            Skill { name: "beta".into(), description: long_desc.clone(), body: String::new(), dir_path: PathBuf::new(), tier: SkillTier::WorkspaceSakha },
        ];
        let message = build_skills_system_message(&skills).unwrap();
        assert!(message.starts_with("You have access to these skills."));
        assert!(message.contains("alpha: short desc"));
        let beta_line = message.lines().find(|l| l.starts_with("beta:")).unwrap();
        // "beta: " prefix (6 chars) + 200-char truncated description.
        assert_eq!(beta_line.len(), "beta: ".len() + 200);
        assert!(beta_line.ends_with("..."));
    }

    #[test]
    fn truncate_chars_leaves_short_strings_untouched() {
        assert_eq!(truncate_chars("short", 200), "short");
    }

    #[test]
    fn missing_root_directory_yields_no_skills_no_warnings() {
        let (skills, warnings) = scan_tier(Path::new("/does/not/exist/at/all"), SkillTier::WorkspaceSakha);
        assert!(skills.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn build_startup_system_messages_empty_when_no_skills_and_no_flag() {
        let messages = build_startup_system_messages(&[], None).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn build_startup_system_messages_includes_listing_when_skills_present() {
        let skills = vec![Skill {
            name: "review".into(),
            description: "Reviews diffs.".into(),
            body: "Review body.".into(),
            dir_path: PathBuf::new(),
            tier: SkillTier::WorkspaceSakha,
        }];
        let messages = build_startup_system_messages(&skills, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("review: Reviews diffs."));
    }

    #[test]
    fn build_startup_system_messages_injects_full_body_for_explicit_skill_flag() {
        let skills = vec![Skill {
            name: "review".into(),
            description: "Reviews diffs.".into(),
            body: "Full review instructions body.".into(),
            dir_path: PathBuf::new(),
            tier: SkillTier::WorkspaceSakha,
        }];
        let messages = build_startup_system_messages(&skills, Some("review")).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("review: Reviews diffs."));
        assert_eq!(messages[1], "Full review instructions body.");
    }

    #[test]
    fn build_startup_system_messages_errors_on_unknown_explicit_skill() {
        let skills = vec![Skill {
            name: "review".into(),
            description: "Reviews diffs.".into(),
            body: "Body.".into(),
            dir_path: PathBuf::new(),
            tier: SkillTier::WorkspaceSakha,
        }];
        let err = build_startup_system_messages(&skills, Some("nope")).unwrap_err();
        assert!(err.contains("nope"));
        assert!(err.contains("review"));
    }

    #[test]
    fn skill_activate_tool_spec_requires_name() {
        let tool = SkillActivateTool;
        let spec = tool.spec();
        assert_eq!(spec.name.0, "skill.activate");
        let required = spec.input_schema.0.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|r| r.as_str() == Some("name")));
    }

    #[test]
    fn skill_activate_tool_rejects_empty_name() {
        let tool = SkillActivateTool;
        assert!(tool.validate(serde_json::json!({"name": ""})).is_err());
    }

    #[tokio::test]
    async fn skill_activate_tool_returns_body_and_resources_for_known_skill() {
        let workspace = tempfile::tempdir().unwrap();
        let skill_dir = write_skill(
            &workspace.path().join(".sakha").join("skills"),
            "review",
            "---\nname: review\ndescription: Reviews diffs.\n---\n\nReview instructions here.\n",
        );
        std::fs::write(skill_dir.join("checklist.md"), "1. check style").unwrap();

        let tool = SkillActivateTool;
        let input = tool.validate(serde_json::json!({"name": "review"})).unwrap();
        let context = ToolContext::new(workspace.path());
        let result = tool.execute(&input, &context).await.unwrap();

        assert_eq!(result.output_json.get("name").unwrap().as_str().unwrap(), "review");
        assert_eq!(result.output_json.get("instructions").unwrap().as_str().unwrap(), "Review instructions here.");
        let resources = result.output_json.get("resources").unwrap().as_array().unwrap();
        assert!(resources.iter().any(|r| r.as_str() == Some("checklist.md")));
    }

    #[tokio::test]
    async fn skill_activate_tool_errors_with_available_names_for_unknown_skill() {
        let workspace = tempfile::tempdir().unwrap();
        write_skill(
            &workspace.path().join(".sakha").join("skills"),
            "review",
            "---\nname: review\ndescription: Reviews diffs.\n---\n\nBody.\n",
        );

        let tool = SkillActivateTool;
        let input = tool.validate(serde_json::json!({"name": "does-not-exist"})).unwrap();
        let context = ToolContext::new(workspace.path());
        let err = tool.execute(&input, &context).await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("does-not-exist"));
        assert!(message.contains("review"));
    }
}
