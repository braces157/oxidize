//! `.gitignore` pattern parsing and path matching engine.

use crate::ConfigError;
use std::fs;
use std::path::Path;

/// A single compiled `.gitignore` pattern rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnorePattern {
    pattern: String,
    is_negation: bool,
    dir_only: bool,
    anchored: bool,
}

impl IgnorePattern {
    /// Parses a single line from a `.gitignore` file. Returns `None` for comments/blank lines.
    pub fn parse(line: &str) -> Option<Self> {
        let mut trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        // Handle escape for leading '#' or '!'
        let mut is_negation = false;
        if trimmed.starts_with('!') {
            is_negation = true;
            trimmed = &trimmed[1..];
        }

        let mut dir_only = false;
        if trimmed.ends_with('/') {
            dir_only = true;
            trimmed = trimmed.trim_end_matches('/');
        }

        let anchored = trimmed.starts_with('/') || trimmed.contains('/');
        let pattern = trimmed.trim_start_matches('/').to_string();

        Some(Self {
            pattern,
            is_negation,
            dir_only,
            anchored,
        })
    }

    /// Tests if a normalized relative path (using `/`) matches this rule.
    pub fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }

        let norm_path = path.trim_matches('/');

        if self.anchored {
            glob_match(&self.pattern, norm_path)
        } else {
            // Match either full path or just the final filename / directory name
            if glob_match(&self.pattern, norm_path) {
                return true;
            }
            if let Some(file_name) = norm_path.rsplit('/').next() {
                glob_match(&self.pattern, file_name)
            } else {
                false
            }
        }
    }
}

/// A single compiled `.gitignore` pattern rule with its directory scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedIgnorePattern {
    /// Directory scope prefix using forward slashes (e.g. "" for root, "sub/" for sub/).
    pub scope: String,
    /// The compiled pattern.
    pub pattern: IgnorePattern,
}

/// A collection of ignore rules (typically loaded from `.gitignore` files hierarchically).
#[derive(Debug, Clone, Default)]
pub struct GitIgnore {
    patterns: Vec<ScopedIgnorePattern>,
}

impl GitIgnore {
    /// Creates an empty `GitIgnore`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends patterns scoped to a specific directory prefix.
    pub fn add_patterns(&mut self, scope: &str, content: &str) {
        let clean_scope = scope.replace('\\', "/");
        let clean_scope = if clean_scope.is_empty() || clean_scope.ends_with('/') {
            clean_scope
        } else {
            format!("{}/", clean_scope)
        };
        for line in content.lines() {
            if let Some(pat) = IgnorePattern::parse(line) {
                self.patterns.push(ScopedIgnorePattern {
                    scope: clean_scope.clone(),
                    pattern: pat,
                });
            }
        }
    }

    /// Parses patterns from a string scoped to root.
    pub fn parse(content: &str) -> Self {
        let mut gi = Self::new();
        gi.add_patterns("", content);
        gi
    }

    /// Loads `.gitignore` and nested `.gitignore` files hierarchically starting from the root directory.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::load_hierarchical(dir)
    }

    /// Loads `.gitignore` files hierarchically across directory depth, including `.git/info/exclude`.
    pub fn load_hierarchical(dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let root = dir.as_ref();
        let mut gi = Self::new();

        // 1. .git/info/exclude
        let exclude_file = root.join(".git").join("info").join("exclude");
        if exclude_file.is_file() {
            if let Ok(content) = fs::read_to_string(exclude_file) {
                gi.add_patterns("", &content);
            }
        }

        // 2. root .gitignore
        let root_ignore = root.join(".gitignore");
        if root_ignore.is_file() {
            if let Ok(content) = fs::read_to_string(root_ignore) {
                gi.add_patterns("", &content);
            }
        }

        // 3. BFS walk for nested .gitignore
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root.to_path_buf());

        while let Some(current_dir) = queue.pop_front() {
            let entries = match fs::read_dir(&current_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str == ".git" {
                    continue;
                }

                if path.is_dir() {
                    let rel = match path.strip_prefix(root) {
                        Ok(r) => r.to_string_lossy().replace('\\', "/"),
                        Err(_) => continue,
                    };
                    // If directory is ignored by current rules, do not descend
                    if gi.is_ignored(&rel, true) {
                        continue;
                    }
                    // If directory contains a .gitignore, load it
                    let sub_ignore = path.join(".gitignore");
                    if sub_ignore.is_file() {
                        if let Ok(content) = fs::read_to_string(sub_ignore) {
                            gi.add_patterns(&format!("{}/", rel), &content);
                        }
                    }
                    queue.push_back(path);
                }
            }
        }

        Ok(gi)
    }

    /// Returns `true` if the specified path should be ignored.
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let normalized = path.replace('\\', "/");
        let norm_path = normalized.trim_matches('/');
        let mut ignored = false;

        for scoped in &self.patterns {
            let rel_path = if scoped.scope.is_empty() {
                norm_path
            } else if norm_path == scoped.scope.trim_end_matches('/') {
                ""
            } else if let Some(stripped) = norm_path.strip_prefix(&scoped.scope) {
                stripped
            } else {
                continue;
            };

            if scoped.pattern.matches(rel_path, is_dir) {
                ignored = !scoped.pattern.is_negation;
            }
        }

        ignored
    }
}

/// Simple glob matcher supporting `*`, `?`, `**`, and character classes `[...]`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    match_helper(&p_chars, 0, &t_chars, 0)
}

fn match_helper(p: &[char], pi: usize, t: &[char], ti: usize) -> bool {
    if pi == p.len() {
        return ti == t.len();
    }

    // Handle `**` (recursive directory match)
    if pi + 1 < p.len() && p[pi] == '*' && p[pi + 1] == '*' {
        let mut next_pi = pi + 2;
        if next_pi < p.len() && p[next_pi] == '/' {
            next_pi += 1;
        }

        // Try matching zero or more segments
        for i in ti..=t.len() {
            if match_helper(p, next_pi, t, i) {
                return true;
            }
        }
        return false;
    }

    // Handle single `*` (matches within a path segment, doesn't cross `/`)
    if p[pi] == '*' {
        let next_pi = pi + 1;
        for i in ti..=t.len() {
            if i > ti && t[i - 1] == '/' {
                break;
            }
            if match_helper(p, next_pi, t, i) {
                return true;
            }
        }
        return false;
    }

    if ti == t.len() {
        return false;
    }

    // Handle `?`
    if p[pi] == '?' {
        if t[ti] == '/' {
            return false;
        }
        return match_helper(p, pi + 1, t, ti + 1);
    }

    // Handle character class `[...]`
    if p[pi] == '[' {
        if let Some(close_idx) = p[pi + 1..].iter().position(|&c| c == ']') {
            let class_slice = &p[pi + 1..pi + 1 + close_idx];
            let matched = class_slice.contains(&t[ti]);
            if matched {
                return match_helper(p, pi + 2 + close_idx, t, ti + 1);
            }
            return false;
        }
    }

    // Literal char match
    if p[pi] == t[ti] {
        return match_helper(p, pi + 1, t, ti + 1);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gitignore_matching() {
        let ignore_text = r#"
# Comments and empty lines
target/
*.o
*.log
!important.log
/root_only.txt
doc/**/*.pdf
"#;

        let gi = GitIgnore::parse(ignore_text);

        // target/
        assert!(gi.is_ignored("target", true));
        assert!(!gi.is_ignored("target", false));
        assert!(gi.is_ignored("sub/target", true));

        // *.log and negation
        assert!(gi.is_ignored("debug.log", false));
        assert!(gi.is_ignored("build/debug.log", false));
        assert!(!gi.is_ignored("important.log", false));

        // /root_only.txt
        assert!(gi.is_ignored("root_only.txt", false));
        assert!(!gi.is_ignored("sub/root_only.txt", false));

        // doc/**/*.pdf
        assert!(gi.is_ignored("doc/manual.pdf", false));
        assert!(gi.is_ignored("doc/api/v1/spec.pdf", false));
        assert!(!gi.is_ignored("other/manual.pdf", false));
    }

    #[test]
    fn test_hierarchical_gitignore() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Root ignores *.log
        fs::write(root.join(".gitignore"), "*.log\n/root.txt\n").unwrap();

        // sub/ contains .gitignore that un-ignores important.log and ignores *.tmp
        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(
            sub.join(".gitignore"),
            "!important.log\n*.tmp\n/sub_only.txt\n",
        )
        .unwrap();

        let gi = GitIgnore::load_hierarchical(root).unwrap();

        // Root rules
        assert!(gi.is_ignored("app.log", false));
        assert!(gi.is_ignored("root.txt", false));
        assert!(!gi.is_ignored("sub/root.txt", false)); // /root.txt was anchored to root

        // Sub overrides root: important.log is unignored in sub
        assert!(gi.is_ignored("sub/other.log", false));
        assert!(!gi.is_ignored("sub/important.log", false));

        // Sub rule: *.tmp is ignored in sub, but not in root
        assert!(gi.is_ignored("sub/test.tmp", false));
        assert!(!gi.is_ignored("test.tmp", false));

        // Sub rule: /sub_only.txt is anchored to sub
        assert!(gi.is_ignored("sub/sub_only.txt", false));
        assert!(!gi.is_ignored("sub/nested/sub_only.txt", false));
    }
}
