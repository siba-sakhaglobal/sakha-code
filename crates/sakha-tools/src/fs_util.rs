//! Workspace-relative path resolution with traversal prevention. See
//! `modules/08-files-git-worktrees.md` `WorkspaceFs` / `PathPolicy`.

use std::path::{Component, Path, PathBuf};

use sakha_core::{SakhaError, SakhaResult};

/// Resolves `requested` (relative or absolute) against `workspace_root`,
/// rejecting any path that would escape the workspace root (`..` traversal,
/// or an absolute path outside the root). Does not require the path to
/// exist; normalizes purely lexically so callers can use it for paths that
/// are about to be created.
pub fn resolve_in_workspace(workspace_root: &Path, requested: &str) -> SakhaResult<PathBuf> {
    let requested_path = Path::new(requested);

    let candidate = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        workspace_root.join(requested_path)
    };

    let normalized = normalize_lexically(&candidate);
    let normalized_root = normalize_lexically(workspace_root);

    if !normalized.starts_with(&normalized_root) {
        return Err(SakhaError::permission(
            "sakha-tools",
            format!("path escapes workspace root: {requested}"),
        ));
    }

    Ok(normalized)
}

/// Lexically normalizes a path (resolves `.` and `..` components without
/// touching the filesystem, so it works for paths that do not exist yet).
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_path_inside_workspace() {
        let root = Path::new("/workspace");
        let result = resolve_in_workspace(root, "src/main.rs").unwrap();
        assert_eq!(result, PathBuf::from("/workspace/src/main.rs"));
    }

    #[test]
    fn rejects_parent_traversal_escaping_root() {
        let root = Path::new("/workspace");
        let result = resolve_in_workspace(root, "../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_deep_traversal_escaping_root() {
        let root = Path::new("/workspace/project");
        let result = resolve_in_workspace(root, "sub/../../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn allows_traversal_that_stays_inside_root() {
        let root = Path::new("/workspace");
        let result = resolve_in_workspace(root, "src/../lib/mod.rs").unwrap();
        assert_eq!(result, PathBuf::from("/workspace/lib/mod.rs"));
    }
}
