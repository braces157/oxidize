//! Data models for the multi-panel interactive terminal UI.

use oxidize_core::id::ObjectId;
use ratatui::style::Color;

/// Available docked panels in the interface (matching authentic LazyGit 1-5 layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Panel {
    /// Panel 1: Repository status, current branch & tracking overview.
    Status,
    /// Panel 2: Working tree files: Staged, Unstaged, Untracked.
    #[default]
    Files,
    /// Panel 3: Local branches, remotes, and tags.
    Branches,
    /// Panel 4: Commit log history and reflog.
    Commits,
    /// Panel 5: Stash stack entries.
    Stash,
}

impl Panel {
    /// Numerical shortcut 1-5 for quick jumping.
    pub fn index(self) -> usize {
        match self {
            Panel::Status => 1,
            Panel::Files => 2,
            Panel::Branches => 3,
            Panel::Commits => 4,
            Panel::Stash => 5,
        }
    }

    /// Creates Panel from 1-based index.
    pub fn from_index(idx: usize) -> Option<Self> {
        match idx {
            1 => Some(Panel::Status),
            2 => Some(Panel::Files),
            3 => Some(Panel::Branches),
            4 => Some(Panel::Commits),
            5 => Some(Panel::Stash),
            _ => None,
        }
    }

    /// Title with keyboard shortcut badge.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Status => "1 Status",
            Panel::Files => "2 Files",
            Panel::Branches => "3 Branches",
            Panel::Commits => "4 Commits",
            Panel::Stash => "5 Stash",
        }
    }

    /// Cycles to the next panel in order.
    pub fn next(self) -> Self {
        match self {
            Panel::Status => Panel::Files,
            Panel::Files => Panel::Branches,
            Panel::Branches => Panel::Commits,
            Panel::Commits => Panel::Stash,
            Panel::Stash => Panel::Status,
        }
    }

    /// Cycles to the previous panel in order.
    pub fn prev(self) -> Self {
        match self {
            Panel::Status => Panel::Stash,
            Panel::Files => Panel::Status,
            Panel::Branches => Panel::Files,
            Panel::Commits => Panel::Branches,
            Panel::Stash => Panel::Commits,
        }
    }
}

/// Sub-tabs within the Branches panel (switched via '[' and ']').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchesTab {
    #[default]
    Local,
    Remotes,
    Tags,
}

impl BranchesTab {
    /// Title header for the tab.
    pub fn title(self) -> &'static str {
        match self {
            Self::Local => "Local Branches",
            Self::Remotes => "Remotes",
            Self::Tags => "Tags",
        }
    }

    /// Next tab in cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Local => Self::Remotes,
            Self::Remotes => Self::Tags,
            Self::Tags => Self::Local,
        }
    }

    /// Previous tab in cycle.
    pub fn prev(self) -> Self {
        match self {
            Self::Local => Self::Tags,
            Self::Remotes => Self::Local,
            Self::Tags => Self::Remotes,
        }
    }
}

/// Sub-tabs within the Commits panel (switched via '[' and ']').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommitsTab {
    #[default]
    Commits,
    Reflog,
}

impl CommitsTab {
    /// Title header for the tab.
    pub fn title(self) -> &'static str {
        match self {
            Self::Commits => "Commits",
            Self::Reflog => "Reflog",
        }
    }

    /// Next tab in cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Commits => Self::Reflog,
            Self::Reflog => Self::Commits,
        }
    }

    /// Previous tab in cycle.
    pub fn prev(self) -> Self {
        match self {
            Self::Commits => Self::Reflog,
            Self::Reflog => Self::Commits,
        }
    }
}

/// Active focused window (Vim-style h/l switching).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedWindow {
    #[default]
    Sidebar,
    Inspector,
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
    /// Unmerged conflicted file with unresolved merge stages.
    Conflicted,
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
            FileStatusKind::Conflicted => ("[U]", Color::Red),
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
            FileStatusKind::Conflicted => "unmerged",
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

/// Represents a configured remote repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteItem {
    /// Remote identifier (e.g. "origin").
    pub name: String,
    /// Configured fetch or push URL.
    pub url: String,
}

/// Represents a Git tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagItem {
    /// Tag name (e.g. "v0.1.0").
    pub name: String,
    /// Commit object ID pointed to by the tag.
    pub oid: ObjectId,
    /// 7-character short hex prefix.
    pub short_oid: String,
    /// Optional tag annotation message.
    pub message: Option<String>,
}

/// Represents an entry in the reflog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogItem {
    /// Reflog index (0 is most recent).
    pub index: usize,
    /// Selector string e.g. "HEAD@{0}".
    pub selector: String,
    /// Target commit OID.
    pub oid: ObjectId,
    /// 7-character short hex prefix.
    pub short_oid: String,
    /// Action category e.g. "commit", "checkout".
    pub action: String,
    /// Full action message.
    pub message: String,
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
    /// Amend commit message prompt.
    CommitAmend {
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
    /// Stash message prompt.
    StashSave {
        /// Entered message text.
        message: String,
        /// Cursor character position.
        cursor: usize,
    },
    /// Keybindings help cheat sheet.
    Help,
}
