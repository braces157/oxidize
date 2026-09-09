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

/// A collection of ignore rules (typically loaded from `.gitignore`).
#[derive(Debug, Clone, Default)]
pub struct GitIgnore {
    patterns: Vec<IgnorePattern>,
}

impl GitIgnore {
    /// Creates an empty `GitIgnore`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses patterns from a string.
    pub fn parse(content: &str) -> Self {
        let mut patterns = Vec::new();
        for line in content.lines() {
            if let Some(pat) = IgnorePattern::parse(line) {
                patterns.push(pat);
            }
        }
        Self { patterns }
    }

    /// Loads `.gitignore` from the root of a repository.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let ignore_file = dir.as_ref().join(".gitignore");
        if ignore_file.is_file() {
            let content = fs::read_to_string(ignore_file)?;
            Ok(Self::parse(&content))
        } else {
            Ok(Self::new())
        }
    }

    /// Returns `true` if the specified path should be ignored.
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let normalized = path.replace('\\', "/");
        let mut ignored = false;

        for pat in &self.patterns {
            if pat.matches(&normalized, is_dir) {
                ignored = !pat.is_negation;
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
}
