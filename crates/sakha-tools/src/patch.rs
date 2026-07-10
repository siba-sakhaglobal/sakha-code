//! Unified-diff patch parser/applier. See `modules/08-files-git-worktrees.md`
//! `Patch` / `PatchHunk` and "Patch Safety": a patch must match expected
//! context, and a mismatch is reported as a conflict rather than silently
//! applied.

/// One hunk of a unified diff: a starting line (1-indexed, in the original
/// file) and a sequence of context/add/remove lines.
#[derive(Debug, Clone)]
struct Hunk {
    old_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

/// Parses a unified diff into hunks, ignoring `---`/`+++` file headers.
fn parse_hunks(diff: &str) -> Result<Vec<Hunk>, String> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;

    for line in diff.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(existing) = current.take() {
                hunks.push(existing);
            }
            let old_start = parse_hunk_header(rest)?;
            current = Some(Hunk {
                old_start,
                lines: Vec::new(),
            });
            continue;
        }
        if let Some(hunk) = current.as_mut() {
            if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine::Add(content.to_string()));
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(HunkLine::Remove(content.to_string()));
            } else if let Some(content) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine::Context(content.to_string()));
            } else if line.is_empty() {
                hunk.lines.push(HunkLine::Context(String::new()));
            }
            // Lines that don't match any of the above (e.g. "\ No newline at
            // end of file") are ignored.
        }
    }

    if let Some(existing) = current.take() {
        hunks.push(existing);
    }

    if hunks.is_empty() {
        return Err("no hunks found in diff".to_string());
    }

    Ok(hunks)
}

/// Parses the `-old_start,old_count +new_start,new_count @@` portion of a
/// hunk header (the leading `@@ ` has already been stripped).
fn parse_hunk_header(rest: &str) -> Result<usize, String> {
    let end = rest.find(" @@").ok_or_else(|| "malformed hunk header".to_string())?;
    let range_part = &rest[..end];
    let old_range = range_part
        .split_whitespace()
        .find(|s| s.starts_with('-'))
        .ok_or_else(|| "malformed hunk header: missing old range".to_string())?;
    let old_start_str = old_range.trim_start_matches('-').split(',').next().unwrap_or("1");
    old_start_str
        .parse::<usize>()
        .map_err(|_| "malformed hunk header: bad line number".to_string())
}

/// Applies a unified diff `diff` to `original`, returning the patched text.
/// Returns `Err(conflict_description)` if any hunk's context/remove lines do
/// not match the original content at the expected location.
pub fn apply_unified_diff(original: &str, diff: &str) -> Result<String, String> {
    let hunks = parse_hunks(diff)?;

    let original_lines: Vec<&str> = if original.is_empty() {
        Vec::new()
    } else {
        original.lines().collect()
    };

    let mut result: Vec<String> = Vec::new();
    // Cursor into original_lines, 0-indexed.
    let mut cursor: usize = 0;

    for hunk in &hunks {
        let hunk_start_idx = hunk.old_start.saturating_sub(1);
        if hunk_start_idx > original_lines.len() {
            return Err(format!(
                "hunk start line {} is beyond end of file ({} lines)",
                hunk.old_start,
                original_lines.len()
            ));
        }

        // Copy unchanged lines up to the hunk start.
        while cursor < hunk_start_idx {
            result.push(original_lines[cursor].to_string());
            cursor += 1;
        }

        for hunk_line in &hunk.lines {
            match hunk_line {
                HunkLine::Context(expected) => {
                    let actual = original_lines.get(cursor).copied().unwrap_or("");
                    if actual != expected {
                        return Err(format!(
                            "context mismatch at line {}: expected {:?}, found {:?}",
                            cursor + 1,
                            expected,
                            actual
                        ));
                    }
                    result.push(actual.to_string());
                    cursor += 1;
                }
                HunkLine::Remove(expected) => {
                    let actual = original_lines.get(cursor).copied().unwrap_or("");
                    if actual != expected {
                        return Err(format!(
                            "removal mismatch at line {}: expected {:?}, found {:?}",
                            cursor + 1,
                            expected,
                            actual
                        ));
                    }
                    cursor += 1;
                }
                HunkLine::Add(content) => {
                    result.push(content.clone());
                }
            }
        }
    }

    // Copy any remaining unchanged tail.
    while cursor < original_lines.len() {
        result.push(original_lines[cursor].to_string());
        cursor += 1;
    }

    let mut joined = result.join("\n");
    if original.ends_with('\n') || original.is_empty() {
        joined.push('\n');
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_simple_single_line_change() {
        let original = "line1\nline2\nline3\n";
        let diff = "--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2-modified\n line3\n";
        let patched = apply_unified_diff(original, diff).unwrap();
        assert_eq!(patched, "line1\nline2-modified\nline3\n");
    }

    #[test]
    fn applies_addition_only_hunk() {
        let original = "line1\nline2\n";
        let diff = "--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@\n line1\n+inserted\n line2\n";
        let patched = apply_unified_diff(original, diff).unwrap();
        assert_eq!(patched, "line1\ninserted\nline2\n");
    }

    #[test]
    fn detects_context_conflict() {
        let original = "completely different\n";
        let diff = "--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2-modified\n line3\n";
        let result = apply_unified_diff(original, diff);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_diff_with_no_hunks() {
        let result = apply_unified_diff("hello\n", "--- a/f\n+++ b/f\n");
        assert!(result.is_err());
    }

    #[test]
    fn removal_only_hunk_deletes_line() {
        let original = "line1\nline2\nline3\n";
        let diff = "--- a/f\n+++ b/f\n@@ -1,3 +1,2 @@\n line1\n-line2\n line3\n";
        let patched = apply_unified_diff(original, diff).unwrap();
        assert_eq!(patched, "line1\nline3\n");
    }
}
