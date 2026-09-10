//! TUI Application state and event handling.

use crate::TuiError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::Object;
use oxidize_index::{compute_status, Index, StagedChange, UnstagedChange};
use oxidize_pack::RepoObjectStore;
use oxidize_refs::RefStore;
use std::path::Path;

/// Tab view mode in the TUI dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabMode {
    /// Commit log graph & details.
    Commits,
    /// Working tree & staging status.
    Status,
}

/// An entry in the commit history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitItem {
    /// Full 20-byte object ID.
    pub oid: ObjectId,
    /// 7-character hexadecimal prefix.
    pub short_oid: String,
    /// Author name.
    pub author: String,
    /// Commit date string.
    pub date: String,
    /// First line summary of the commit message.
    pub summary: String,
    /// Complete commit message body.
    pub full_message: String,
    /// Parent commit IDs.
    pub parents: Vec<ObjectId>,
}

/// Core application state for the interactive TUI.
pub struct App {
    /// Loaded list of commits in reverse chronological order.
    pub commits: Vec<CommitItem>,
    /// Currently selected index in the commit list.
    pub selected_index: usize,
    /// Active dashboard tab.
    pub active_tab: TabMode,
    /// Formatted status lines for working tree.
    pub status_lines: Vec<String>,
    /// Active branch name.
    pub branch_name: String,
    /// Flag indicating whether the application should terminate.
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates a new empty `App`.
    pub fn new() -> Self {
        Self {
            commits: Vec::new(),
            selected_index: 0,
            active_tab: TabMode::Commits,
            status_lines: Vec::new(),
            branch_name: "master".to_string(),
            should_quit: false,
        }
    }

    /// Loads repository commits and status from disk.
    pub fn load_repository(&mut self, git_dir: &Path) -> Result<(), TuiError> {
        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let ref_store = RefStore::new(git_dir);

        let (branch, head_oid_opt) = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        self.branch_name = branch;

        if let Some(head_oid) = head_oid_opt {
            let mut visited = std::collections::HashSet::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(head_oid);
            visited.insert(head_oid);

            while let Some(oid) = queue.pop_front() {
                if let Ok(Object::Commit(commit)) = store.read_object(&oid) {
                    let first_line = commit.message.lines().next().unwrap_or("").to_string();
                    let date_str = chrono::DateTime::from_timestamp(commit.author.time_seconds, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| commit.author.time_seconds.to_string());
                    self.commits.push(CommitItem {
                        oid,
                        short_oid: oid.to_string()[..7].to_string(),
                        author: commit.author.name.clone(),
                        date: date_str,
                        summary: first_line,
                        full_message: commit.message.clone(),
                        parents: commit.parents.clone(),
                    });

                    for p in commit.parents {
                        if visited.insert(p) {
                            queue.push_back(p);
                        }
                    }
                }
            }
        }

        // Load working tree status if working directory exists
        if let Some(repo_root) = git_dir.parent() {
            let index_path = git_dir.join("index");
            let index = Index::load_from(&index_path).unwrap_or_default();
            let head_tree = head_oid_opt.and_then(|oid| {
                if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                    Some(c.tree)
                } else {
                    None
                }
            });

            if let Ok(status) = compute_status(repo_root, &index, head_tree.as_ref(), &store) {
                if !status.staged.is_empty() {
                    self.status_lines
                        .push("Changes to be committed:".to_string());
                    for change in &status.staged {
                        match change {
                            StagedChange::New(p) => {
                                self.status_lines.push(format!("  new file:   {}", p))
                            }
                            StagedChange::Modified(p) => {
                                self.status_lines.push(format!("  modified:   {}", p))
                            }
                            StagedChange::Deleted(p) => {
                                self.status_lines.push(format!("  deleted:    {}", p))
                            }
                            StagedChange::Renamed { from, to } => self
                                .status_lines
                                .push(format!("  renamed:    {} -> {}", from, to)),
                        }
                    }
                }
                if !status.unstaged.is_empty() {
                    self.status_lines.push("Changes not staged:".to_string());
                    for change in &status.unstaged {
                        match change {
                            UnstagedChange::Modified(p) => {
                                self.status_lines.push(format!("  modified:   {}", p))
                            }
                            UnstagedChange::Deleted(p) => {
                                self.status_lines.push(format!("  deleted:    {}", p))
                            }
                        }
                    }
                }
                if !status.untracked.is_empty() {
                    self.status_lines.push("Untracked files:".to_string());
                    for file in &status.untracked {
                        self.status_lines.push(format!("  {}", file));
                    }
                }
                if self.status_lines.is_empty() {
                    self.status_lines.push("working tree clean".to_string());
                }
            }
        }

        Ok(())
    }

    /// Selects the next commit in the list.
    pub fn next(&mut self) {
        if !self.commits.is_empty() && self.selected_index + 1 < self.commits.len() {
            self.selected_index += 1;
        }
    }

    /// Selects the previous commit in the list.
    pub fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Toggles active tab between Commits and Status.
    pub fn toggle_tab(&mut self) {
        self.active_tab = match self.active_tab {
            TabMode::Commits => TabMode::Status,
            TabMode::Status => TabMode::Commits,
        };
    }

    /// Currently selected commit item, if any.
    pub fn selected_commit(&self) -> Option<&CommitItem> {
        self.commits.get(self.selected_index)
    }
}
