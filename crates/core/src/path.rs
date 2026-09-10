//! Validated repository-relative paths and reference names.
//! Enforces repository boundary safety, protecting against directory traversal,
//! absolute paths, `.git` metadata overwrite, Windows reserved devices/ADS/8.3 aliases,
//! and symlink escapes.

use crate::error::CoreError;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Validates a single path component (such as a filename in a tree entry).
pub fn validate_tree_component(name: &str) -> Result<(), CoreError> {
    if name.is_empty() {
        return Err(CoreError::InvalidPath("empty component".to_string()));
    }

    if name == "." || name == ".." {
        return Err(CoreError::InvalidPath(format!(
            "traversal component '{}' not allowed",
            name
        )));
    }

    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' contains forbidden separator or NUL",
            name
        )));
    }

    // Protect .git and case variations (.git, .GIT, .Git, etc.)
    let lower = name.to_ascii_lowercase();
    if lower == ".git" {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' targets .git metadata directory",
            name
        )));
    }

    // Windows NTFS 8.3 short name aliases for .git: git~1, git~2, etc., and gi~1, etc.
    if lower.starts_with("git~") || lower.starts_with("gi~") || lower.starts_with("g~") {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' targets potential 8.3 .git alias",
            name
        )));
    }

    // NTFS Alternate Data Streams (:stream)
    if name.contains(':') {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' contains NTFS alternate data stream delimiter ':'",
            name
        )));
    }

    // Windows strips trailing dots and spaces in Win32 APIs (e.g. ".git." -> ".git")
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' ends with space or dot which Windows strips",
            name
        )));
    }

    // Windows DOS device names (CON, PRN, AUX, NUL, COM1..9, LPT1..9)
    let stem = if let Some(dot_idx) = lower.find('.') {
        &lower[..dot_idx]
    } else {
        &lower
    };

    const DOS_DEVICES: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];

    if DOS_DEVICES.contains(&stem) {
        return Err(CoreError::InvalidPath(format!(
            "component '{}' is a reserved DOS device name",
            name
        )));
    }

    Ok(())
}

/// Validates that a string is a safe, repository-relative path.
/// Must use forward slashes `/`, cannot be absolute, cannot traverse with `..`,
/// and cannot target `.git` or any platform-specific dangerous names.
pub fn validate_repo_path(path: &str) -> Result<(), CoreError> {
    if path.is_empty() {
        return Err(CoreError::InvalidPath("empty repository path".to_string()));
    }

    if path.starts_with('/') || path.starts_with('\\') {
        return Err(CoreError::InvalidPath(format!(
            "path '{}' cannot start with a slash",
            path
        )));
    }

    if path.ends_with('/') || path.ends_with('\\') {
        return Err(CoreError::InvalidPath(format!(
            "path '{}' cannot end with a trailing slash",
            path
        )));
    }

    // Check for Windows drive letter (e.g. "C:...")
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0].is_ascii_alphabetic()) {
        return Err(CoreError::InvalidPath(format!(
            "path '{}' has Windows drive prefix",
            path
        )));
    }

    if path.contains("//") || path.contains("\\\\") {
        return Err(CoreError::InvalidPath(format!(
            "path '{}' contains duplicate slashes",
            path
        )));
    }

    // Normalize or check components
    for comp in path.split(['/', '\\']) {
        validate_tree_component(comp)?;
    }

    Ok(())
}

/// A validated repository-relative path with normalized forward slashes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(String);

impl RepoPath {
    /// Creates and validates a new `RepoPath`.
    pub fn new(path: &str) -> Result<Self, CoreError> {
        validate_repo_path(path)?;
        let normalized = path.replace('\\', "/");
        Ok(Self(normalized))
    }

    /// Returns the path as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for RepoPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for RepoPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepoPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strips Windows extended-length (verbatim) prefix `\\?\` or `\\?\UNC\` if present.
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", stripped))
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// Safely joins `rel_path` to `base`, verifying repository containment and
/// ensuring no intermediate symlink or reparse point escapes `base` or enters `.git`.
pub fn safe_join(base: &Path, rel_path: &str) -> Result<PathBuf, CoreError> {
    validate_repo_path(rel_path)?;

    // Check base canonicalization if it exists
    let canonical_base = if base.exists() {
        base.canonicalize().map_err(CoreError::Io)?
    } else {
        base.to_path_buf()
    };

    let mut current = base.to_path_buf();
    let mut check_current = canonical_base.clone();

    for comp in Path::new(rel_path).components() {
        match comp {
            Component::Normal(c) => {
                current.push(c);
                check_current.push(c);

                // If this intermediate component exists on disk, check if it is a symlink
                if let Ok(meta) = fs::symlink_metadata(&current) {
                    if meta.file_type().is_symlink() {
                        // Check where it points
                        let target = fs::canonicalize(&current).map_err(CoreError::Io)?;
                        if !target.starts_with(&canonical_base) {
                            return Err(CoreError::InvalidPath(format!(
                                "path '{}' traverses symlink pointing outside repository: {}",
                                rel_path,
                                target.display()
                            )));
                        }
                        let git_dir = canonical_base.join(".git");
                        if target.starts_with(&git_dir) {
                            return Err(CoreError::InvalidPath(format!(
                                "path '{}' traverses symlink pointing into .git: {}",
                                rel_path,
                                target.display()
                            )));
                        }
                        check_current = target;
                    }
                }
            }
            _ => {
                return Err(CoreError::InvalidPath(format!(
                    "invalid path component in '{}'",
                    rel_path
                )));
            }
        }
    }

    // Ensure final target does not point into .git
    let git_dir = canonical_base.join(".git");
    if check_current.starts_with(&git_dir) {
        return Err(CoreError::InvalidPath(format!(
            "path '{}' targets .git metadata directory",
            rel_path
        )));
    }

    Ok(current)
}

/// Validates reference names according to Git rules (`git check-ref-format`).
pub fn validate_ref_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() {
        return Err(CoreError::InvalidRefName("empty ref name".to_string()));
    }

    if name.starts_with('/') || name.ends_with('/') {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot begin or end with a slash",
            name
        )));
    }

    if name.contains("//") {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot contain consecutive slashes",
            name
        )));
    }

    if name.contains("..") {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot contain traversal '..'",
            name
        )));
    }

    if name.contains("@{") {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot contain '@{{'",
            name
        )));
    }

    if name == "@" {
        return Err(CoreError::InvalidRefName(
            "ref name cannot be '@'".to_string(),
        ));
    }

    if name.ends_with(".lock") {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot end with '.lock'",
            name
        )));
    }

    if name.ends_with('.') {
        return Err(CoreError::InvalidRefName(format!(
            "ref name '{}' cannot end with a dot",
            name
        )));
    }

    for c in name.chars() {
        if (c as u32) < 0x20 || (c as u32) == 0x7f {
            return Err(CoreError::InvalidRefName(format!(
                "ref name '{}' contains control character",
                name
            )));
        }
        if matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
            return Err(CoreError::InvalidRefName(format!(
                "ref name '{}' contains forbidden character '{}'",
                name, c
            )));
        }
    }

    for comp in name.split('/') {
        if comp.is_empty() {
            return Err(CoreError::InvalidRefName(format!(
                "ref name '{}' contains empty component",
                name
            )));
        }
        if comp.starts_with('.') {
            return Err(CoreError::InvalidRefName(format!(
                "ref component '{}' cannot begin with a dot",
                comp
            )));
        }
        if comp.ends_with(".lock") {
            return Err(CoreError::InvalidRefName(format!(
                "ref component '{}' cannot end with '.lock'",
                comp
            )));
        }
        if comp.ends_with('.') {
            return Err(CoreError::InvalidRefName(format!(
                "ref component '{}' cannot end with a dot",
                comp
            )));
        }
    }

    Ok(())
}

/// Validates a branch name that will be placed under `refs/heads/`.
pub fn validate_branch_name(branch: &str) -> Result<(), CoreError> {
    if branch.is_empty() {
        return Err(CoreError::InvalidRefName("empty branch name".to_string()));
    }

    if branch == "HEAD" {
        return Err(CoreError::InvalidRefName(
            "branch name cannot be 'HEAD'".to_string(),
        ));
    }

    if branch.starts_with('/') || branch.starts_with('\\') || branch.contains("..") {
        return Err(CoreError::InvalidRefName(format!(
            "branch name '{}' contains traversal or leading slash",
            branch
        )));
    }

    let full_ref = format!("refs/heads/{}", branch);
    validate_ref_name(&full_ref)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_repo_paths() {
        assert!(validate_repo_path("a.txt").is_ok());
        assert!(validate_repo_path("src/lib.rs").is_ok());
        assert!(validate_repo_path("dir/sub/file.txt").is_ok());
    }

    #[test]
    fn test_invalid_repo_paths() {
        assert!(validate_repo_path("").is_err());
        assert!(validate_repo_path("/a.txt").is_err());
        assert!(validate_repo_path("a/").is_err());
        assert!(validate_repo_path("../a.txt").is_err());
        assert!(validate_repo_path("a/../b.txt").is_err());
        assert!(validate_repo_path(".git").is_err());
        assert!(validate_repo_path(".git/config").is_err());
        assert!(validate_repo_path(".GIT/HEAD").is_err());
        assert!(validate_repo_path("git~1").is_err());
        assert!(validate_repo_path("foo:bar").is_err());
        assert!(validate_repo_path("trailing. ").is_err());
        assert!(validate_repo_path("con.txt").is_err());
        assert!(validate_repo_path("aux").is_err());
        assert!(validate_repo_path("C:/escaped").is_err());
    }

    #[test]
    fn test_valid_ref_names() {
        assert!(validate_ref_name("refs/heads/main").is_ok());
        assert!(validate_ref_name("refs/tags/v1.0").is_ok());
        assert!(validate_ref_name("refs/remotes/origin/feature/foo").is_ok());
    }

    #[test]
    fn test_invalid_ref_names() {
        assert!(validate_ref_name("").is_err());
        assert!(validate_ref_name("/refs/heads/main").is_err());
        assert!(validate_ref_name("refs/heads/main/").is_err());
        assert!(validate_ref_name("refs/heads/../main").is_err());
        assert!(validate_ref_name("refs/heads/main.lock").is_err());
        assert!(validate_ref_name("refs/heads/.hidden").is_err());
        assert!(validate_ref_name("refs/heads/has space").is_err());
        assert!(validate_ref_name("refs/heads/has~tilde").is_err());
        assert!(validate_ref_name("refs/heads/has^caret").is_err());
        assert!(validate_ref_name("refs/heads/has:colon").is_err());
        assert!(validate_ref_name("refs/heads/has?question").is_err());
        assert!(validate_ref_name("refs/heads/has*star").is_err());
        assert!(validate_ref_name("refs/heads/has[bracket").is_err());
        assert!(validate_ref_name("refs/heads/has\\backslash").is_err());
        assert!(validate_ref_name("refs/heads/has@{seq").is_err());
        assert!(validate_ref_name("@").is_err());
    }

    #[test]
    fn test_branch_names() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/x").is_ok());
        assert!(validate_branch_name("../../sentinel").is_err());
        assert!(validate_branch_name("HEAD").is_err());
        assert!(validate_branch_name("foo.lock").is_err());
    }
}
