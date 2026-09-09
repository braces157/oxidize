//! Author and committer signature extraction from environment, config, and system clock.

use chrono::{DateTime, Local, Offset};
use oxidize_core::object::Signature;
use std::env;
use std::fs;
use std::path::Path;

/// Obtains the current committer/author signature according to standard Git resolution rules.
pub fn get_default_signature(git_dir: Option<&Path>) -> Signature {
    let name = env::var("GIT_AUTHOR_NAME")
        .or_else(|_| env::var("GIT_COMMITTER_NAME"))
        .ok()
        .or_else(|| read_config_key(git_dir, "user", "name"))
        .or_else(|| env::var("USERNAME").or_else(|_| env::var("USER")).ok())
        .unwrap_or_else(|| "Oxidize User".to_string());

    let email = env::var("GIT_AUTHOR_EMAIL")
        .or_else(|_| env::var("GIT_COMMITTER_EMAIL"))
        .ok()
        .or_else(|| read_config_key(git_dir, "user", "email"))
        .unwrap_or_else(|| format!("{}@localhost", name.replace(' ', ".").to_lowercase()));

    let now: DateTime<Local> = Local::now();
    let time_seconds = now.timestamp();
    let offset = now.offset().fix();
    let offset_seconds = offset.local_minus_utc();
    let sign = if offset_seconds >= 0 { '+' } else { '-' };
    let abs_sec = offset_seconds.abs();
    let hours = abs_sec / 3600;
    let mins = (abs_sec % 3600) / 60;
    let tz_offset = format!("{}{:02}{:02}", sign, hours, mins);

    Signature {
        name,
        email,
        time_seconds,
        tz_offset,
    }
}

fn read_config_key(git_dir: Option<&Path>, section: &str, key: &str) -> Option<String> {
    // 1. Try local .git/config
    if let Some(dir) = git_dir {
        let local_cfg = dir.join("config");
        if let Some(val) = extract_ini_val(&local_cfg, section, key) {
            return Some(val);
        }
    }

    // 2. Try global ~/.gitconfig
    if let Ok(home) = env::var("USERPROFILE").or_else(|_| env::var("HOME")) {
        let global_cfg = Path::new(&home).join(".gitconfig");
        if let Some(val) = extract_ini_val(&global_cfg, section, key) {
            return Some(val);
        }
    }

    None
}

fn extract_ini_val(path: &Path, target_section: &str, target_key: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let mut in_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            let sec = &line[1..line.len() - 1].trim();
            in_section = sec.eq_ignore_ascii_case(target_section);
            continue;
        }

        if in_section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(target_key) {
                    return Some(v.trim().to_string());
                }
            }
        }
    }

    None
}
