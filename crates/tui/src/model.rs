//! Data models for the multi-panel interactive terminal UI.

use oxidize_core::id::ObjectId;
use ratatui::style::Color;

/// Available docked panels in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// Working tree files: Staged, Unstaged, Untracked.
    Files,
    /// Local and remote branch references.
    Branches,
    /// Commit log history.
    Commits,
    /// Stash stack entries.
    Stash,
}

impl Panel {
    /// Numerical shortcut 1-4 for quick jumping.
    pub fn index(self) -> usize {
        match self {
            Panel::Files => 1,
            Panel::Branches => 2,
            Panel::Commits => 3,
            Panel::Stash => 4,
        }
    }

    /// Title with keyboard shortcut badge.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Files => "1 Files",
            Panel::Branches => "2 Branches",
            Panel::Commits => "3 Commits",
            Panel::Stash => "4 Stash",
        }
    }

    /// Cycles to the next panel in order.
    pub fn next(self) -> Self {
        match self {
            Panel::Files => Panel::Branches,
            Panel::Branches => Panel::Commits,
            Panel::Commits => Panel::Stash,
            Panel::Stash => Panel::Files,
        }
    }

    /// Cycles to the previous panel in order.
    pub fn prev(self) -> Self {
        match self {
            Panel::Files => Panel::Stash,
            Panel::Branches => Panel::Files,
            Panel::Commits => Panel::Branches,
            Panel::Stash => Panel::Commits,
        }
    }
}

/// Detailed file status change category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatusKind {
    /// Newly staged file.
    StagedNew,
    /// Staged modification.
    StagedModified,
    /// Staged deletion.
    StagedDeleted,
    /// Staged rename.
    StagedRenamed,
    /// Unstaged modification in working directory.
    UnstagedModified,
    /// Unstaged deletion in working directory.
    UnstagedDeleted,
    /// Untracked new file in working directory.
    Untracked,
}

impl FileStatusKind {
    /// Returns a short prefix badge and corresponding theme color.
    pub fn badge(&self) -> (&'static str, Color) {
        match self {
            FileStatusKind::StagedNew => ("[+]", Color::Green),
            FileStatusKind::StagedModified => ("[●]", Color::Green),
            FileStatusKind::StagedDeleted => ("[-]", Color::Red),
            FileStatusKind::StagedRenamed => ("[R]", Color::Green),
            FileStatusKind::UnstagedModified => ("[M]", Color::Yellow),
            FileStatusKind::UnstagedDeleted => ("[D]", Color::LightRed),
            FileStatusKind::Untracked => ("[?]", Color::Magenta),
        }
    }

    /// Descriptive text for the status category.
    pub fn label(&self) -> &'static str {
        match self {
            FileStatusKind::StagedNew => "new file",
            FileStatusKind::StagedModified => "modified",
            FileStatusKind::StagedDeleted => "deleted",
            FileStatusKind::StagedRenamed => "renamed",
            FileStatusKind::UnstagedModified => "modified",
            FileStatusKind::UnstagedDeleted => "deleted",
            FileStatusKind::Untracked => "untracked",
        }
    }

    /// Whether this status corresponds to staged changes.
    pub fn is_staged(&self) -> bool {
        matches!(
            self,
            FileStatusKind::StagedNew
                | FileStatusKind::StagedModified
                | FileStatusKind::StagedDeleted
                | FileStatusKind::StagedRenamed
        )
    }
}

/// Represents an entry in the Files panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileItem {
    /// Canonical relative path in repository.
    pub path: String,
    /// Previous path for renames.
    pub old_path: Option<String>,
    /// Status category.
    pub kind: FileStatusKind,
}

/// Represents a branch entry in the Branches panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchItem {
    /// Branch name (e.g. "master" or "origin/main").
    pub name: String,
    /// Whether this is the currently checked-out branch.
    pub is_head: bool,
    /// Whether this is a remote-tracking branch.
    pub is_remote: bool,
    /// Target commit OID.
    pub commit_oid: Option<ObjectId>,
    /// First line summary of the latest commit.
    pub summary: Option<String>,
}

/// Represents an entry in the Commits panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitItem {
    /// Full 20-byte object ID.
    pub oid: ObjectId,
    /// 7-character hexadecimal prefix.
    pub short_oid: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub author_email: String,
    /// Commit date string.
    pub date: String,
    /// First line summary of commit message.
    pub summary: String,
    /// Complete commit message body.
    pub full_message: String,
    /// Parent commit IDs.
    pub parents: Vec<ObjectId>,
}

/// Represents an entry in the Stash panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashItem {
    /// Stash stack index (0 for `stash@{0}`).
    pub index: usize,
    /// Commit object ID representing the stash.
    pub oid: ObjectId,
    /// Stash commit message.
    pub message: String,
    /// Formatted date of stash creation.
    pub date: String,
}

/// Line classification for syntax-highlighted diff rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Diff command or file headers (`diff --git`, `---`, `+++`).
    Header,
    /// Hunk range headers (`@@ ... @@`).
    HunkHeader,
    /// Added line (`+`).
    Addition,
    /// Deleted line (`-`).
    Deletion,
    /// Unchanged context line (` `).
    Context,
    /// General metadata or separator.
    Normal,
}

/// Individual line in the Inspector view with styling information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Raw text content of line.
    pub content: String,
    /// Line style category.
    pub kind: DiffLineKind,
}

impl DiffLine {
    /// Constructs a new diff line with detected kind.
    pub fn new(content: impl Into<String>, kind: DiffLineKind) -> Self {
        Self {
            content: content.into(),
            kind,
        }
    }

    /// Infers diff line type from prefix character.
    pub fn from_raw_line(line: &str) -> Self {
        let kind = if line.starts_with("diff --git")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("similarity index")
            || line.starts_with("rename from")
            || line.starts_with("rename to")
            || line.starts_with("commit ")
            || line.starts_with("Author:")
            || line.starts_with("Date:")
        {
            DiffLineKind::Header
        } else if line.starts_with("@@") {
            DiffLineKind::HunkHeader
        } else if line.starts_with('+') {
            DiffLineKind::Addition
        } else if line.starts_with('-') {
            DiffLineKind::Deletion
        } else if line.starts_with(' ') {
            DiffLineKind::Context
        } else {
            DiffLineKind::Normal
        };

        Self {
            content: line.to_string(),
            kind,
        }
    }
}

/// Formatted contents displayed in the right-hand Inspector pane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffView {
    /// Title header for the inspector panel.
    pub title: String,
    /// List of formatted diff lines.
    pub lines: Vec<DiffLine>,
}

impl DiffView {
    /// Creates an empty DiffView.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            lines: Vec::new(),
        }
    }

    /// Constructs DiffView by parsing unified diff text.
    pub fn from_unified_text(title: impl Into<String>, text: &str) -> Self {
        let lines = text.lines().map(DiffLine::from_raw_line).collect();
        Self {
            title: title.into(),
            lines,
        }
    }
}

/// Backwards compatibility alias for the legacy 2-tab mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabMode {
    Commits,
    Status,
}

/// Floating modal dialog states for interactive user input.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActiveModal {
    #[default]
    None,
    /// Commit message prompt.
    CommitPrompt {
        /// Entered message text.
        message: String,
        /// Cursor character position.
        cursor: usize,
    },
    /// New branch name prompt.
    BranchCreate {
        /// Entered branch name.
        name: String,
        /// Cursor character position.
        cursor: usize,
    },
    /// Keybindings help cheat sheet.
    Help,
}
