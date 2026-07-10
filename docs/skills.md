# Agent Skills

Sakha supports Claude-Code/Gemini-CLI-style **skills**: reusable `SKILL.md`
instruction packs that are discovered from standard directories, listed to
the model compactly in the system prompt, and pulled into the conversation
on demand via a tool call. A skill directory authored for Claude Code or
Gemini CLI works with Sakha unmodified, and vice versa.

## Skill format

A skill is a directory containing a `SKILL.md` file with YAML frontmatter
followed by a markdown body:

```markdown
---
name: review
description: Reviews a diff for correctness before style. Use before approving any PR.
---

Review diffs for correctness first, then style. Check for:
- Off-by-one errors and null/undefined handling
- Missing error handling on I/O
- Tests that don't actually exercise the changed behavior
```

- **`name`** (optional) — the skill's id, used in `skill.activate` calls and
  `sakha skills` output. If omitted, the containing directory's name is used
  instead.
- **`description`** (required) — one or more lines describing what the skill
  does and when to use it. A skill with no description is skipped (a warning
  is printed to stderr) since the model has nothing to match the skill
  against.
- The frontmatter block must open and close with a line containing exactly
  `---`. CRLF line endings are tolerated.
- Everything after the closing `---` is the skill's **body** — the
  instructions handed to the model verbatim on activation.
- Any other files in the skill directory (including nested subdirectories,
  e.g. `references/`, `assets/`) are **resources** the body may reference.
  `skill.activate` lists their relative paths so the model knows what's
  available without reading the whole tree.

Sakha's frontmatter parser is hand-rolled (`key: value` line splitting, no
YAML crate dependency) but tolerates the common variants other tools
produce: folded/literal block scalars (`description: >` or `description: |`
followed by indented continuation lines) for multi-line descriptions, and
quoted scalar values.

## Discovery tiers

Skills are discovered from these directories, later tiers overriding earlier
ones on name conflict:

| Tier | Path | Scope |
|---|---|---|
| User | `~/.agents/skills/` | shared across every project |
| User | `~/.sakha/skills/` | shared across every project |
| Workspace | `<cwd>/.gemini/skills/` | this project (Gemini CLI's directory) |
| Workspace | `<cwd>/.claude/skills/` | this project (Claude Code's directory) |
| Workspace | `<cwd>/.agents/skills/` | this project |
| Workspace | `<cwd>/.sakha/skills/` | this project (highest precedence) |

Within the workspace tiers, first match wins in the order `.sakha/skills/`,
`.agents/skills/`, `.claude/skills/`, `.gemini/skills/` — i.e. a skill named
`review` under `.sakha/skills/review/` shadows a same-named skill under
`.gemini/skills/review/`. Any workspace tier overrides the user tier. This
lets a project pin its own version of a skill the user also has installed
globally, while still working out of the box with skill directories that
already exist for Claude Code (`.claude/skills/`) or Gemini CLI
(`.gemini/skills/`) in that repo.

`{SAKHA_HOME}` (or `~` if unset) is used for the user tier, consistent with
Sakha's other config/credential paths.

## Listing and inspecting skills

```bash
# Table: name, tier, directory, first line of description.
sakha skills list

# Machine-readable form.
sakha skills list --output json

# Full SKILL.md body to stdout.
sakha skills show review
```

## Activation

### Automatic (system-prompt + tool call)

When one or more skills are discovered, `sakha run`/`sakha chat` prepend a
single system message listing every skill compactly (name + description,
truncated to 200 characters):

```
You have access to these skills. When a task matches a skill's description, call skill.activate with its name and follow the returned instructions.
review: Reviews a diff for correctness before style. Use before approving any PR.
```

The model is expected to call the `skill.activate` tool with
`{"name": "review"}` when a task matches. The tool returns:

```json
{
  "name": "review",
  "instructions": "<the skill's markdown body>",
  "resources": ["references/checklist.md"]
}
```

An unknown name returns an `invalid_input` error listing every available
skill name, so the model can self-correct instead of retrying blind.

When zero skills are discovered, no system message is added and
`skill.activate` simply has nothing to activate (it still registers, so
`sakha tools list` output is stable regardless of whether any skills happen
to be on disk).

### Explicit (`--skill` flag)

To skip the discovery/activation round-trip entirely, pass `--skill <name>`
to `run` or `chat`:

```bash
sakha run --skill review "review the diff in this PR"
sakha chat --skill review
```

This injects the named skill's **full body** as a second system message up
front, in addition to the normal skills listing. The named skill must be one
Sakha actually discovered (see `sakha skills list`); an unknown name fails
the command immediately with the same available-names list `skill.activate`
would report, rather than silently proceeding without it.

## Authoring skills

- Keep `description` specific enough that the model can tell *when* to use
  the skill from the compact listing alone — it only sees the description
  (truncated to 200 characters) until it activates the skill.
- Put anything long, reference-heavy, or rarely needed (checklists, schemas,
  example files) in a separate resource file rather than the body, and
  mention it by relative path in the body. `skill.activate`'s `resources`
  list tells the model those paths exist; it can then `file.read` them if
  actually needed.
- Skills are plain directories — `git`-track them like any other project
  file under `.sakha/skills/` (or `.agents/skills/`, etc.) to share them with
  collaborators, or install them under `~/.sakha/skills/` for personal,
  cross-project skills.
- See `crates/sakha-cli/src/skills.rs` for the discovery/parsing
  implementation and `~/.sakha/skills/design-md/SKILL.md` for a worked
  example (authoring/applying `DESIGN.md` design-system files).
