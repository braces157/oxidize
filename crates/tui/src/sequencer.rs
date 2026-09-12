//! Durable native replay sequencer state machine for interactive rebase,
//! cherry-pick, and conflict resolution workflows.
//!
//! Conforms to standard Git `.git/rebase-merge/` persistence conventions so that
//! operations are durable across application restarts and cleanly inspectable.

use crate::TuiError;
use oxidize_core::id::ObjectId;
use std::fs;
use std::path::{Path, PathBuf};

/// Individual rebase action for an item in the rebase todo list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Edit => "edit",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }
}

impl std::str::FromStr for RebaseAction {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "p" | "pick" => Ok(Self::Pick),
            "r" | "reword" => Ok(Self::Reword),
            "e" | "edit" => Ok(Self::Edit),
            "s" | "squash" => Ok(Self::Squash),
            "f" | "fixup" => Ok(Self::Fixup),
            "d" | "drop" => Ok(Self::Drop),
            _ => Err(()),
        }
    }
}

/// A single step in an interactive rebase plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseTodoItem {
    pub action: RebaseAction,
    pub commit_oid: ObjectId,
    pub short_oid: String,
    pub summary: String,
    pub message: Option<String>,
}

impl RebaseTodoItem {
    pub fn new(action: RebaseAction, commit_oid: ObjectId, summary: String) -> Self {
        let short_oid = if commit_oid.to_string().len() >= 7 {
            commit_oid.to_string()[..7].to_string()
        } else {
            commit_oid.to_string()
        };
        Self {
            action,
            commit_oid,
            short_oid,
            summary,
            message: None,
        }
    }
}

/// Current status of the active sequencer operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencerStatus {
    Running,
    Conflicted,
    StoppedForEditing,
}

impl SequencerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Conflicted => "conflicted",
            Self::StoppedForEditing => "stopped_for_editing",
        }
    }
}

impl std::str::FromStr for SequencerStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "conflicted" => Ok(Self::Conflicted),
            "stopped_for_editing" => Ok(Self::StoppedForEditing),
            _ => Ok(Self::Running),
        }
    }
}

/// Persistent in-memory and on-disk state of an active interactive rebase or replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencerState {
    /// Name of the branch being rebased, e.g. "refs/heads/feature" or "HEAD".
    pub head_name: String,
    /// The original commit OID before rebase started.
    pub orig_head: ObjectId,
    /// The commit OID onto which the branch is being replayed.
    pub onto: ObjectId,
    /// 1-based index of the step currently being executed.
    pub current_step: usize,
    /// Total number of steps in the original rebase plan.
    pub total_steps: usize,
    /// Remaining todo steps to be processed.
    pub todo: Vec<RebaseTodoItem>,
    /// Steps already completed.
    pub done: Vec<RebaseTodoItem>,
    /// The commit OID currently stopped on (e.g. for conflict or editing).
    pub stopped_sha: Option<ObjectId>,
    /// Status indicator (running, conflicted, stopped for editing).
    pub status: SequencerStatus,
}

impl SequencerState {
    /// Directory used for git-compatible rebase state.
    pub fn state_dir(git_dir: &Path) -> PathBuf {
        git_dir.join("rebase-merge")
    }

    /// Checks if a sequencer / rebase is currently active on disk.
    pub fn is_active(git_dir: &Path) -> bool {
        Self::state_dir(git_dir).is_dir()
    }

    /// Loads the active sequencer state from `.git/rebase-merge/`, or returns `None` if inactive.
    pub fn load(git_dir: &Path) -> Result<Option<Self>, TuiError> {
        let dir = Self::state_dir(git_dir);
        if !dir.is_dir() {
            return Ok(None);
        }

        let head_name = fs::read_to_string(dir.join("head-name"))
            .unwrap_or_else(|_| "HEAD".to_string())
            .trim()
            .to_string();

        let orig_head_str = fs::read_to_string(dir.join("orig-head"))
            .map_err(|e| TuiError::Terminal(format!("Failed to read orig-head: {}", e)))?;
        let orig_head = orig_head_str
            .trim()
            .parse::<ObjectId>()
            .map_err(|e| TuiError::Terminal(format!("Invalid orig-head OID: {}", e)))?;

        let onto_str = fs::read_to_string(dir.join("onto"))
            .map_err(|e| TuiError::Terminal(format!("Failed to read onto: {}", e)))?;
        let onto = onto_str
            .trim()
            .parse::<ObjectId>()
            .map_err(|e| TuiError::Terminal(format!("Invalid onto OID: {}", e)))?;

        let stopped_sha = if let Ok(s) = fs::read_to_string(dir.join("stopped-sha")) {
            s.trim().parse::<ObjectId>().ok()
        } else {
            None
        };

        let status = if let Ok(s) = fs::read_to_string(dir.join("state")) {
            s.parse::<SequencerStatus>()
                .unwrap_or(SequencerStatus::Running)
        } else {
            SequencerStatus::Running
        };

        let current_step = fs::read_to_string(dir.join("msgnum"))
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(1);

        let total_steps = fs::read_to_string(dir.join("end"))
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(1);

        let mut todo = if let Ok(content) = fs::read_to_string(dir.join("git-rebase-todo")) {
            Self::parse_todo_list(&content)
        } else {
            Vec::new()
        };

        for item in &mut todo {
            if let Ok(m) = fs::read_to_string(dir.join(format!("msg-{}", item.commit_oid))) {
                item.message = Some(m);
            }
        }

        let done = if let Ok(content) = fs::read_to_string(dir.join("done")) {
            Self::parse_todo_list(&content)
        } else {
            Vec::new()
        };

        Ok(Some(Self {
            head_name,
            orig_head,
            onto,
            current_step,
            total_steps,
            todo,
            done,
            stopped_sha,
            status,
        }))
    }

    /// Persists this sequencer state to disk in `.git/rebase-merge/`.
    pub fn save(&self, git_dir: &Path) -> Result<(), TuiError> {
        let dir = Self::state_dir(git_dir);
        fs::create_dir_all(&dir)?;

        fs::write(dir.join("head-name"), &self.head_name)?;
        fs::write(dir.join("orig-head"), format!("{}\n", self.orig_head))?;
        fs::write(dir.join("onto"), format!("{}\n", self.onto))?;
        fs::write(dir.join("msgnum"), format!("{}\n", self.current_step))?;
        fs::write(dir.join("end"), format!("{}\n", self.total_steps))?;
        fs::write(dir.join("state"), format!("{}\n", self.status.as_str()))?;

        if let Some(sha) = &self.stopped_sha {
            fs::write(dir.join("stopped-sha"), format!("{}\n", sha))?;
        } else {
            match fs::remove_file(dir.join("stopped-sha")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }

        let todo_content = Self::format_todo_list(&self.todo);
        fs::write(dir.join("git-rebase-todo"), todo_content)?;

        for item in &self.todo {
            if let Some(ref m) = item.message {
                fs::write(dir.join(format!("msg-{}", item.commit_oid)), m)?;
            }
        }

        let done_content = Self::format_todo_list(&self.done);
        fs::write(dir.join("done"), done_content)?;

        Ok(())
    }

    /// Removes the `.git/rebase-merge/` directory cleanly after completion or abort.
    pub fn clear(git_dir: &Path) -> Result<(), TuiError> {
        let dir = Self::state_dir(git_dir);
        if dir.is_dir() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    fn parse_todo_list(content: &str) -> Vec<RebaseTodoItem> {
        let mut items = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(3, ' ');
            let action_str = parts.next().unwrap_or("");
            let oid_str = parts.next().unwrap_or("");
            let summary = parts.next().unwrap_or("").to_string();

            if let (Ok(action), Ok(oid)) = (
                action_str.parse::<RebaseAction>(),
                oid_str.parse::<ObjectId>(),
            ) {
                items.push(RebaseTodoItem::new(action, oid, summary));
            }
        }
        items
    }

    fn format_todo_list(items: &[RebaseTodoItem]) -> String {
        let mut out = String::new();
        for item in items {
            out.push_str(&format!(
                "{} {} {}\n",
                item.action.as_str(),
                item.commit_oid,
                item.summary
            ));
        }
        out
    }
}
