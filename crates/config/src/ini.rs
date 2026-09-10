//! Git configuration file (`.git/config`, `~/.gitconfig`) INI parser and serializer.

use crate::ConfigError;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Key identifying a configuration section and optional subsection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigSectionKey {
    /// Section name, e.g. "core", "remote", "branch".
    pub section: String,
    /// Optional subsection, e.g. "origin" in `[remote "origin"]`.
    pub subsection: Option<String>,
}

/// Parsed Git configuration file representation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GitConfig {
    sections: BTreeMap<ConfigSectionKey, BTreeMap<String, Vec<String>>>,
}

impl GitConfig {
    /// Creates a new empty `GitConfig`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads and parses a config file from disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }
        let content = fs::read_to_string(path)?;
        Self::parse_str(&content)
    }

    /// Parses configuration text in Git INI format.
    pub fn parse_str(content: &str) -> Result<Self, ConfigError> {
        let mut config = Self::new();
        let mut current_section: Option<ConfigSectionKey> = None;

        for (line_idx, raw_line) in content.lines().enumerate() {
            let line_num = line_idx + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') {
                if !line.ends_with(']') {
                    return Err(ConfigError::SyntaxError(format!(
                        "unclosed section header at line {}: {}",
                        line_num, line
                    )));
                }
                let inner = line[1..line.len() - 1].trim();
                if let Some(quote_start) = inner.find('"') {
                    if let Some(quote_end) = inner.rfind('"') {
                        if quote_end > quote_start {
                            let sec = inner[..quote_start].trim().to_lowercase();
                            let sub = inner[quote_start + 1..quote_end].to_string();
                            current_section = Some(ConfigSectionKey {
                                section: sec,
                                subsection: Some(sub),
                            });
                            continue;
                        }
                    }
                    return Err(ConfigError::SyntaxError(format!(
                        "malformed subsection header at line {}: {}",
                        line_num, line
                    )));
                }
                current_section = Some(ConfigSectionKey {
                    section: inner.to_lowercase(),
                    subsection: None,
                });
            } else if let Some(ref sec_key) = current_section {
                // Strip unquoted inline comments
                let line_content = if let Some(hash_pos) = line.find(['#', ';']) {
                    let before = &line[..hash_pos];
                    let quote_count = before.chars().filter(|&c| c == '"').count();
                    if quote_count % 2 == 0 {
                        before.trim()
                    } else {
                        line
                    }
                } else {
                    line
                };

                if line_content.is_empty() {
                    continue;
                }

                if let Some(eq_idx) = line_content.find('=') {
                    let key = line_content[..eq_idx].trim().to_lowercase();
                    if key.is_empty() {
                        return Err(ConfigError::SyntaxError(format!(
                            "empty config key at line {}: {}",
                            line_num, line
                        )));
                    }
                    let raw_val = line_content[eq_idx + 1..].trim();
                    let val =
                        if raw_val.starts_with('"') && raw_val.ends_with('"') && raw_val.len() >= 2
                        {
                            raw_val[1..raw_val.len() - 1].to_string()
                        } else {
                            raw_val.to_string()
                        };
                    config
                        .sections
                        .entry(sec_key.clone())
                        .or_default()
                        .entry(key)
                        .or_default()
                        .push(val);
                } else {
                    let key = line_content.trim().to_lowercase();
                    if !key.is_empty() && !key.contains(' ') {
                        config
                            .sections
                            .entry(sec_key.clone())
                            .or_default()
                            .entry(key)
                            .or_default()
                            .push("true".to_string());
                    } else {
                        return Err(ConfigError::SyntaxError(format!(
                            "bad config line at line {}: {}",
                            line_num, line
                        )));
                    }
                }
            } else {
                return Err(ConfigError::SyntaxError(format!(
                    "config entry outside of any section at line {}: {}",
                    line_num, line
                )));
            }
        }

        Ok(config)
    }

    /// Retrieves the last value from the specified section and key.
    pub fn get(&self, section: &str, subsection: Option<&str>, key: &str) -> Option<&str> {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections.get(&sec_key).and_then(|entries| {
            entries
                .get(&key.to_lowercase())
                .and_then(|v| v.last())
                .map(|s| s.as_str())
        })
    }

    /// Retrieves all values for a given section, subsection, and key.
    pub fn get_all(&self, section: &str, subsection: Option<&str>, key: &str) -> Option<&[String]> {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections
            .get(&sec_key)
            .and_then(|entries| entries.get(&key.to_lowercase()).map(|v| v.as_slice()))
    }

    /// Returns a map of all key-value entries in a section (last value per key).
    pub fn get_section(
        &self,
        section: &str,
        subsection: Option<&str>,
    ) -> Option<BTreeMap<String, String>> {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections.get(&sec_key).map(|entries| {
            entries
                .iter()
                .filter_map(|(k, v)| v.last().map(|val| (k.clone(), val.clone())))
                .collect()
        })
    }

    /// Returns a reference to all key-values entries (including multi-values) in a section.
    pub fn get_section_all(
        &self,
        section: &str,
        subsection: Option<&str>,
    ) -> Option<&BTreeMap<String, Vec<String>>> {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections.get(&sec_key)
    }

    /// Returns all configured aliases as a map of `alias_name -> command_string`.
    pub fn get_aliases(&self) -> BTreeMap<String, String> {
        self.get_section("alias", None).unwrap_or_default()
    }

    /// Retrieves the command string for a named alias if defined.
    pub fn get_alias(&self, name: &str) -> Option<&str> {
        self.get("alias", None, name)
    }

    /// Sets a value for the specified section, subsection, and key, replacing prior values.
    pub fn set(&mut self, section: &str, subsection: Option<&str>, key: &str, value: &str) {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections
            .entry(sec_key)
            .or_default()
            .insert(key.to_lowercase(), vec![value.to_string()]);
    }

    /// Appends an additional value for the specified section, subsection, and key without clearing prior values.
    pub fn add_value(&mut self, section: &str, subsection: Option<&str>, key: &str, value: &str) {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections
            .entry(sec_key)
            .or_default()
            .entry(key.to_lowercase())
            .or_default()
            .push(value.to_string());
    }

    /// Removes a configuration section and all its entries.
    pub fn remove_section(&mut self, section: &str, subsection: Option<&str>) -> bool {
        let sec_key = ConfigSectionKey {
            section: section.to_lowercase(),
            subsection: subsection.map(|s| s.to_string()),
        };
        self.sections.remove(&sec_key).is_some()
    }

    /// Retrieves the URL for a named remote (e.g. "origin").
    pub fn get_remote_url(&self, name: &str) -> Option<&str> {
        self.get("remote", Some(name), "url")
    }

    /// Retrieves the last value by its full dot-separated key (e.g. "user.name" or "remote.origin.url").
    pub fn get_by_name(&self, full_key: &str) -> Option<&str> {
        let first_dot = full_key.find('.')?;
        let last_dot = full_key.rfind('.')?;
        if first_dot == last_dot {
            let section = &full_key[..first_dot];
            let key = &full_key[first_dot + 1..];
            self.get(section, None, key)
        } else {
            let section = &full_key[..first_dot];
            let subsection = &full_key[first_dot + 1..last_dot];
            let key = &full_key[last_dot + 1..];
            self.get(section, Some(subsection), key)
        }
    }

    /// Retrieves all values by full dot-separated key.
    pub fn get_all_by_name(&self, full_key: &str) -> Option<&[String]> {
        let first_dot = full_key.find('.')?;
        let last_dot = full_key.rfind('.')?;
        if first_dot == last_dot {
            let section = &full_key[..first_dot];
            let key = &full_key[first_dot + 1..];
            self.get_all(section, None, key)
        } else {
            let section = &full_key[..first_dot];
            let subsection = &full_key[first_dot + 1..last_dot];
            let key = &full_key[last_dot + 1..];
            self.get_all(section, Some(subsection), key)
        }
    }

    /// Sets a value by its full dot-separated key, replacing prior values.
    pub fn set_by_name(&mut self, full_key: &str, value: &str) -> bool {
        if let Some(first_dot) = full_key.find('.') {
            let last_dot = full_key.rfind('.').unwrap();
            if first_dot == last_dot {
                let section = &full_key[..first_dot];
                let key = &full_key[first_dot + 1..];
                self.set(section, None, key, value);
                true
            } else {
                let section = &full_key[..first_dot];
                let subsection = &full_key[first_dot + 1..last_dot];
                let key = &full_key[last_dot + 1..];
                self.set(section, Some(subsection), key, value);
                true
            }
        } else {
            false
        }
    }

    /// Appends a value by its full dot-separated key without overwriting prior values.
    pub fn add_by_name(&mut self, full_key: &str, value: &str) -> bool {
        if let Some(first_dot) = full_key.find('.') {
            let last_dot = full_key.rfind('.').unwrap();
            if first_dot == last_dot {
                let section = &full_key[..first_dot];
                let key = &full_key[first_dot + 1..];
                self.add_value(section, None, key, value);
                true
            } else {
                let section = &full_key[..first_dot];
                let subsection = &full_key[first_dot + 1..last_dot];
                let key = &full_key[last_dot + 1..];
                self.add_value(section, Some(subsection), key, value);
                true
            }
        } else {
            false
        }
    }

    /// Unsets a key by its full dot-separated key (e.g. "user.name" or "remote.origin.url").
    pub fn unset_by_name(&mut self, full_key: &str) -> bool {
        if let Some(first_dot) = full_key.find('.') {
            let last_dot = full_key.rfind('.').unwrap();
            let (section, subsection, key) = if first_dot == last_dot {
                (&full_key[..first_dot], None, &full_key[first_dot + 1..])
            } else {
                (
                    &full_key[..first_dot],
                    Some(&full_key[first_dot + 1..last_dot]),
                    &full_key[last_dot + 1..],
                )
            };
            let sec_key = ConfigSectionKey {
                section: section.to_lowercase(),
                subsection: subsection.map(|s| s.to_string()),
            };
            if let Some(entries) = self.sections.get_mut(&sec_key) {
                let removed = entries.remove(&key.to_lowercase()).is_some();
                if entries.is_empty() {
                    self.sections.remove(&sec_key);
                }
                removed
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Lists all configuration entries as `(full_key, value)` sorted lexicographically.
    pub fn list_all(&self) -> Vec<(String, String)> {
        let mut list = Vec::new();
        for (sec_key, entries) in &self.sections {
            for (k, values) in entries {
                let full_name = if let Some(ref sub) = sec_key.subsection {
                    format!("{}.{}.{}", sec_key.section, sub, k)
                } else {
                    format!("{}.{}", sec_key.section, k)
                };
                for val in values {
                    list.push((full_name.clone(), val.clone()));
                }
            }
        }
        list
    }

    /// Configures a remote with the specified URL and standard fetch refspec.
    pub fn add_remote(&mut self, name: &str, url: &str) {
        let normalized_url = url.replace('\\', "/");
        self.set("remote", Some(name), "url", &normalized_url);
        self.set(
            "remote",
            Some(name),
            "fetch",
            &format!("+refs/heads/*:refs/remotes/{}/*", name),
        );
    }

    /// Removes a remote from the configuration.
    pub fn remove_remote(&mut self, name: &str) -> bool {
        self.remove_section("remote", Some(name))
    }

    /// Lists all configured remotes as `(name, url)`.
    pub fn list_remotes(&self) -> Vec<(String, String)> {
        let mut remotes = Vec::new();
        for (sec, entries) in &self.sections {
            if sec.section == "remote" {
                if let Some(ref name) = sec.subsection {
                    if let Some(urls) = entries.get("url") {
                        if let Some(url) = urls.last() {
                            remotes.push((name.clone(), url.clone()));
                        }
                    }
                }
            }
        }
        remotes
    }

    /// Serializes configuration back to Git INI format, preserving all values.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (sec, entries) in &self.sections {
            if let Some(ref sub) = sec.subsection {
                out.push_str(&format!("[{} \"{}\"]\n", sec.section, sub));
            } else {
                out.push_str(&format!("[{}]\n", sec.section));
            }
            for (key, values) in entries {
                for val in values {
                    out.push_str(&format!("\t{} = {}\n", key, val));
                }
            }
        }
        out
    }

    /// Saves configuration to a file on disk.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let serialized = self.serialize();
        fs::write(path, serialized)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_config_parse_and_serialize() {
        let sample = r#"
[core]
	repositoryformatversion = 0
	filemode = false
	bare = false
[remote "origin"]
	url = https://github.com/oxidize/ox.git
	fetch = +refs/heads/*:refs/remotes/origin/*
[branch "main"]
	remote = origin
	merge = refs/heads/main
"#;

        let mut config = GitConfig::parse_str(sample).unwrap();
        assert_eq!(config.get("core", None, "bare"), Some("false"));
        assert_eq!(
            config.get_remote_url("origin"),
            Some("https://github.com/oxidize/ox.git")
        );
        assert_eq!(
            config.list_remotes(),
            vec![(
                "origin".to_string(),
                "https://github.com/oxidize/ox.git".to_string()
            )]
        );

        config.add_remote("upstream", "https://github.com/upstream/ox.git");
        assert_eq!(
            config.get_remote_url("upstream"),
            Some("https://github.com/upstream/ox.git")
        );

        let serialized = config.serialize();
        assert!(serialized.contains("[remote \"upstream\"]"));
    }

    #[test]
    fn test_git_config_aliases() {
        let sample = r#"
[alias]
	st = status
	ci = commit
	lg = log --oneline --graph
"#;
        let config = GitConfig::parse_str(sample).unwrap();
        assert_eq!(config.get_alias("st"), Some("status"));
        assert_eq!(config.get_alias("lg"), Some("log --oneline --graph"));
        assert_eq!(config.get_alias("unknown"), None);
        let aliases = config.get_aliases();
        assert_eq!(aliases.len(), 3);
        assert_eq!(aliases.get("ci").map(|s| s.as_str()), Some("commit"));
    }

    #[test]
    fn test_git_config_dot_notation_helpers() {
        let mut config = GitConfig::new();
        config.set_by_name("user.name", "Jane Doe");
        config.set_by_name("user.email", "jane@example.com");
        config.set_by_name("remote.origin.url", "https://github.com/test/repo.git");

        assert_eq!(config.get_by_name("user.name"), Some("Jane Doe"));
        assert_eq!(config.get_by_name("user.email"), Some("jane@example.com"));
        assert_eq!(
            config.get_by_name("remote.origin.url"),
            Some("https://github.com/test/repo.git")
        );

        let all = config.list_all();
        assert_eq!(all.len(), 3);

        assert!(config.unset_by_name("user.email"));
        assert_eq!(config.get_by_name("user.email"), None);
        assert_eq!(config.list_all().len(), 2);
    }

    #[test]
    fn test_multivalued_keys_and_roundtrip() {
        let sample = r#"
[remote "origin"]
	url = https://github.com/first/repo.git
	url = https://github.com/second/repo.git
	fetch = +refs/heads/*:refs/remotes/origin/*
	fetch = +refs/tags/*:refs/tags/*
"#;
        let config = GitConfig::parse_str(sample).unwrap();
        // get returns last value
        assert_eq!(
            config.get("remote", Some("origin"), "url"),
            Some("https://github.com/second/repo.git")
        );
        // get_all returns all values in order
        let urls = config.get_all("remote", Some("origin"), "url").unwrap();
        assert_eq!(
            urls,
            &[
                "https://github.com/first/repo.git",
                "https://github.com/second/repo.git"
            ]
        );
        let fetches = config.get_all("remote", Some("origin"), "fetch").unwrap();
        assert_eq!(
            fetches,
            &[
                "+refs/heads/*:refs/remotes/origin/*",
                "+refs/tags/*:refs/tags/*"
            ]
        );

        // Serialize preserves all multi-valued lines
        let serialized = config.serialize();
        assert_eq!(serialized.matches("url =").count(), 2);
        assert_eq!(serialized.matches("fetch =").count(), 2);
    }

    #[test]
    fn test_syntax_errors_rejection() {
        // Unclosed header
        assert!(GitConfig::parse_str("[core\nkey = val").is_err());
        // Entry outside of section
        assert!(GitConfig::parse_str("key = val\n[core]").is_err());
        // Empty key
        assert!(GitConfig::parse_str("[core]\n = val").is_err());
    }

    #[test]
    fn test_bare_boolean_flags_and_inline_comments() {
        let sample = r#"
[core]
	bare # this repository is bare
	filemode = false ; ignore filemode changes
"#;
        let config = GitConfig::parse_str(sample).unwrap();
        assert_eq!(config.get("core", None, "bare"), Some("true"));
        assert_eq!(config.get("core", None, "filemode"), Some("false"));
    }
}
