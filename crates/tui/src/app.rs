//! TUI Application state, multi-panel repository data loader, and live diff inspector.

use crate::model::{
    ActiveModal, BranchItem, CommitItem, DiffLine, DiffLineKind, DiffView, FileItem,
    FileStatusKind, Panel, StashItem, TabMode,
};
use crate::ops;
use crate::TuiError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::Object;
use oxidize_diff::format_unified_diff;
use oxidize_index::{compute_status, flatten_tree, Index, StagedChange, UnstagedChange};
use oxidize_pack::RepoObjectStore;
use oxidize_refs::RefStore;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// Core state machine for the multi-panel interactive TUI.
pub struct App {
    /// Root directory of the repository working tree.
    pub repo_root: PathBuf,
    /// Path to the `.git` directory.
    pub git_dir: PathBuf,
    /// Currently checked-out branch or "HEAD (detached)".
    pub branch_name: String,
    /// Active docked panel in focus.
    pub active_panel: Panel,

    /// Changed files in working tree and staging area.
    pub files: Vec<FileItem>,
    /// Selected index in files list.
    pub files_selected: usize,

    /// Local and remote branches.
    pub branches: Vec<BranchItem>,
    /// Selected index in branches list.
    pub branches_selected: usize,

    /// Loaded commit history.
    pub commits: Vec<CommitItem>,
    /// Selected index in commits list.
    pub commits_selected: usize,

    /// Stash stack items.
    pub stashes: Vec<StashItem>,
    /// Selected index in stash list.
    pub stashes_selected: usize,

    /// Vertical scroll offset in the right Inspector pane.
    pub inspector_scroll: usize,
    /// Cached rendered diff view for the current selection.
    pub cached_diff: Option<DiffView>,

    /// Currently displayed modal dialog, if any.
    pub active_modal: ActiveModal,

    /// Status or notification banner shown in header/footer.
    pub status_message: Option<String>,
    /// Flag indicating whether the application should terminate.
    pub should_quit: bool,

    // Backwards compatibility fields
    /// Legacy tab mode for compatibility with earlier dashboards.
    pub active_tab: TabMode,
    /// Legacy flat status lines.
    pub status_lines: Vec<String>,
    /// Legacy selected index alias (points to commits_selected).
    pub selected_index: usize,
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
            repo_root: PathBuf::new(),
            git_dir: PathBuf::new(),
            branch_name: "master".to_string(),
            active_panel: Panel::Files,
            files: Vec::new(),
            files_selected: 0,
            branches: Vec::new(),
            branches_selected: 0,
            commits: Vec::new(),
            commits_selected: 0,
            stashes: Vec::new(),
            stashes_selected: 0,
            inspector_scroll: 0,
            cached_diff: None,
            active_modal: ActiveModal::None,
            status_message: None,
            should_quit: false,
            active_tab: TabMode::Commits,
            status_lines: Vec::new(),
            selected_index: 0,
        }
    }

    /// Loads repository data from disk into the multi-panel state.
    pub fn load_repository(&mut self, git_dir: &Path) -> Result<(), TuiError> {
        self.git_dir = git_dir.to_path_buf();
        self.repo_root = git_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| git_dir.to_path_buf());

        let store =
            RepoObjectStore::open(git_dir).map_err(|e| TuiError::Terminal(e.to_string()))?;
        let ref_store = RefStore::new(git_dir);

        // 1. Resolve HEAD branch and commit
        let (branch, head_oid_opt) = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        self.branch_name = branch.clone();

        // 2. Load commits
        self.commits.clear();
        if let Some(head_oid) = head_oid_opt {
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
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
                        author_email: commit.author.email.clone(),
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

        // 3. Load branches
        self.branches.clear();
        let active_branch = self.branch_name.clone();
        self.load_branches(&store, &active_branch);

        // 4. Load stash
        self.stashes.clear();
        self.load_stashes(&store);

        // 5. Load working tree & staging status
        self.files.clear();
        self.status_lines.clear();
        let index_path = git_dir.join("index");
        let index = Index::load_from(&index_path).unwrap_or_default();
        let head_tree = head_oid_opt.and_then(|oid| {
            if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                Some(c.tree)
            } else {
                None
            }
        });

        if let Ok(status) = compute_status(&self.repo_root, &index, head_tree.as_ref(), &store) {
            for staged in &status.staged {
                match staged {
                    StagedChange::New(p) => self.files.push(FileItem {
                        path: p.clone(),
                        old_path: None,
                        kind: FileStatusKind::StagedNew,
                    }),
                    StagedChange::Modified(p) => self.files.push(FileItem {
                        path: p.clone(),
                        old_path: None,
                        kind: FileStatusKind::StagedModified,
                    }),
                    StagedChange::Deleted(p) => self.files.push(FileItem {
                        path: p.clone(),
                        old_path: None,
                        kind: FileStatusKind::StagedDeleted,
                    }),
                    StagedChange::Renamed { from, to } => self.files.push(FileItem {
                        path: to.clone(),
                        old_path: Some(from.clone()),
                        kind: FileStatusKind::StagedRenamed,
                    }),
                }
            }
            for unstaged in &status.unstaged {
                match unstaged {
                    UnstagedChange::Modified(p) => self.files.push(FileItem {
                        path: p.clone(),
                        old_path: None,
                        kind: FileStatusKind::UnstagedModified,
                    }),
                    UnstagedChange::Deleted(p) => self.files.push(FileItem {
                        path: p.clone(),
                        old_path: None,
                        kind: FileStatusKind::UnstagedDeleted,
                    }),
                }
            }
            for untracked in &status.untracked {
                self.files.push(FileItem {
                    path: untracked.clone(),
                    old_path: None,
                    kind: FileStatusKind::Untracked,
                });
            }

            // Sync legacy status lines
            for f in &self.files {
                let (badge, _) = f.kind.badge();
                self.status_lines
                    .push(format!("{} {}: {}", badge, f.kind.label(), f.path));
            }
            if self.status_lines.is_empty() {
                self.status_lines.push("working tree clean".to_string());
            }
        }

        // Clamp selection indices
        self.clamp_selections();

        // Update inspector view
        self.update_inspector();

        Ok(())
    }

    /// Reloads repository data in place while preserving selection indices.
    pub fn refresh(&mut self) -> Result<(), TuiError> {
        let git_dir = self.git_dir.clone();
        if !git_dir.as_os_str().is_empty() {
            self.load_repository(&git_dir)?;
        }
        Ok(())
    }

    fn load_branches(&mut self, store: &RepoObjectStore, active_branch: &str) {
        let heads_dir = self.git_dir.join("refs").join("heads");
        let mut found_heads = Vec::new();
        collect_ref_files(&heads_dir, "", &mut found_heads);
        found_heads.sort();

        for name in found_heads {
            let ref_file = heads_dir.join(&name);
            let (commit_oid, summary) = read_ref_info(&ref_file, store);
            let is_head = name == active_branch;
            self.branches.push(BranchItem {
                name,
                is_head,
                is_remote: false,
                commit_oid,
                summary,
            });
        }

        // Remotes
        let remotes_dir = self.git_dir.join("refs").join("remotes");
        let mut found_remotes = Vec::new();
        collect_ref_files(&remotes_dir, "", &mut found_remotes);
        found_remotes.sort();

        for name in found_remotes {
            let ref_file = remotes_dir.join(&name);
            let (commit_oid, summary) = read_ref_info(&ref_file, store);
            self.branches.push(BranchItem {
                name,
                is_head: false,
                is_remote: true,
                commit_oid,
                summary,
            });
        }

        // Active HEAD branch first, then local branches, then remote branches
        self.branches.sort_by(|a, b| {
            b.is_head
                .cmp(&a.is_head)
                .then(a.is_remote.cmp(&b.is_remote))
                .then(a.name.cmp(&b.name))
        });
    }

    fn load_stashes(&mut self, store: &RepoObjectStore) {
        let stash_log = self.git_dir.join("logs").join("refs").join("stash");
        if stash_log.exists() {
            if let Ok(content) = fs::read_to_string(&stash_log) {
                let lines: Vec<&str> = content.lines().collect();
                for (idx, line) in lines.iter().rev().enumerate() {
                    let parts: Vec<&str> = line.split('\t').collect();
                    let msg = parts.get(1).unwrap_or(&"WIP on stash").to_string();
                    let mut oid = ObjectId::ZERO;
                    if let Some(left) = parts.first() {
                        let tokens: Vec<&str> = left.split_whitespace().collect();
                        if let Some(target_sha) = tokens.get(1) {
                            if let Ok(parsed) = target_sha.parse::<ObjectId>() {
                                oid = parsed;
                            }
                        }
                    }
                    self.stashes.push(StashItem {
                        index: idx,
                        oid,
                        message: msg,
                        date: String::new(),
                    });
                }
            }
        } else {
            let stash_ref = self.git_dir.join("refs").join("stash");
            if stash_ref.exists() {
                if let Ok(content) = fs::read_to_string(&stash_ref) {
                    if let Ok(oid) = content.trim().parse::<ObjectId>() {
                        let summary = if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                            c.message.lines().next().unwrap_or("").to_string()
                        } else {
                            "WIP on branch".to_string()
                        };
                        self.stashes.push(StashItem {
                            index: 0,
                            oid,
                            message: summary,
                            date: String::new(),
                        });
                    }
                }
            }
        }
    }

    fn clamp_selections(&mut self) {
        if !self.files.is_empty() {
            if self.files_selected >= self.files.len() {
                self.files_selected = self.files.len() - 1;
            }
        } else {
            self.files_selected = 0;
        }

        if !self.branches.is_empty() {
            if self.branches_selected >= self.branches.len() {
                self.branches_selected = self.branches.len() - 1;
            }
        } else {
            self.branches_selected = 0;
        }

        if !self.commits.is_empty() {
            if self.commits_selected >= self.commits.len() {
                self.commits_selected = self.commits.len() - 1;
            }
        } else {
            self.commits_selected = 0;
        }
        self.selected_index = self.commits_selected;

        if !self.stashes.is_empty() {
            if self.stashes_selected >= self.stashes.len() {
                self.stashes_selected = self.stashes.len() - 1;
            }
        } else {
            self.stashes_selected = 0;
        }
    }

    /// Selects a specific panel and refreshes the inspector.
    pub fn select_panel(&mut self, panel: Panel) {
        self.active_panel = panel;
        self.inspector_scroll = 0;
        self.active_tab = match panel {
            Panel::Files => TabMode::Status,
            _ => TabMode::Commits,
        };
        self.update_inspector();
    }

    /// Cycles to next docked panel.
    pub fn next_panel(&mut self) {
        self.select_panel(self.active_panel.next());
    }

    /// Cycles to previous docked panel.
    pub fn prev_panel(&mut self) {
        self.select_panel(self.active_panel.prev());
    }

    /// Moves selection down within the currently active panel.
    pub fn next_item(&mut self) {
        match self.active_panel {
            Panel::Files => {
                if !self.files.is_empty() && self.files_selected + 1 < self.files.len() {
                    self.files_selected += 1;
                }
            }
            Panel::Branches => {
                if !self.branches.is_empty() && self.branches_selected + 1 < self.branches.len() {
                    self.branches_selected += 1;
                }
            }
            Panel::Commits => {
                if !self.commits.is_empty() && self.commits_selected + 1 < self.commits.len() {
                    self.commits_selected += 1;
                    self.selected_index = self.commits_selected;
                }
            }
            Panel::Stash => {
                if !self.stashes.is_empty() && self.stashes_selected + 1 < self.stashes.len() {
                    self.stashes_selected += 1;
                }
            }
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Moves selection up within the currently active panel.
    pub fn prev_item(&mut self) {
        match self.active_panel {
            Panel::Files => {
                if self.files_selected > 0 {
                    self.files_selected -= 1;
                }
            }
            Panel::Branches => {
                if self.branches_selected > 0 {
                    self.branches_selected -= 1;
                }
            }
            Panel::Commits => {
                if self.commits_selected > 0 {
                    self.commits_selected -= 1;
                    self.selected_index = self.commits_selected;
                }
            }
            Panel::Stash => {
                if self.stashes_selected > 0 {
                    self.stashes_selected -= 1;
                }
            }
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Scrolls inspector view downwards by the given number of lines.
    pub fn scroll_inspector_down(&mut self, lines: usize) {
        let max_lines = self
            .cached_diff
            .as_ref()
            .map(|d| d.lines.len())
            .unwrap_or(0);
        if self.inspector_scroll + lines < max_lines {
            self.inspector_scroll += lines;
        } else if max_lines > 0 {
            self.inspector_scroll = max_lines.saturating_sub(1);
        }
    }

    /// Scrolls inspector view upwards by the given number of lines.
    pub fn scroll_inspector_up(&mut self, lines: usize) {
        self.inspector_scroll = self.inspector_scroll.saturating_sub(lines);
    }

    /// Currently highlighted file item, if any.
    pub fn selected_file(&self) -> Option<&FileItem> {
        self.files.get(self.files_selected)
    }

    /// Currently highlighted branch item, if any.
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        self.branches.get(self.branches_selected)
    }

    /// Currently highlighted commit item, if any.
    pub fn selected_commit(&self) -> Option<&CommitItem> {
        self.commits.get(self.commits_selected)
    }

    /// Currently highlighted stash item, if any.
    pub fn selected_stash(&self) -> Option<&StashItem> {
        self.stashes.get(self.stashes_selected)
    }

    /// Dynamically computes and updates the right-hand Inspector pane.
    pub fn update_inspector(&mut self) {
        if self.git_dir.as_os_str().is_empty() {
            self.cached_diff = Some(DiffView::new("No repository loaded"));
            return;
        }

        let store_res = RepoObjectStore::open(&self.git_dir);
        let store = match store_res {
            Ok(s) => s,
            Err(_) => {
                self.cached_diff = Some(DiffView::new("Failed to open object store"));
                return;
            }
        };

        match self.active_panel {
            Panel::Files => {
                if let Some(file) = self.selected_file().cloned() {
                    self.cached_diff = Some(self.compute_file_diff(&store, &file));
                } else {
                    self.cached_diff = Some(DiffView::new("No changed files in working tree"));
                }
            }
            Panel::Branches => {
                if let Some(branch) = self.selected_branch().cloned() {
                    self.cached_diff = Some(self.compute_branch_view(&store, &branch));
                } else {
                    self.cached_diff = Some(DiffView::new("No branches"));
                }
            }
            Panel::Commits => {
                if let Some(commit) = self.selected_commit().cloned() {
                    self.cached_diff = Some(self.compute_commit_diff(&store, &commit));
                } else {
                    self.cached_diff = Some(DiffView::new("No commits in repository"));
                }
            }
            Panel::Stash => {
                if let Some(stash) = self.selected_stash().cloned() {
                    self.cached_diff = Some(self.compute_stash_view(&store, &stash));
                } else {
                    self.cached_diff = Some(DiffView::new("Stash stack is empty"));
                }
            }
        }
    }

    fn compute_file_diff(&self, store: &RepoObjectStore, file: &FileItem) -> DiffView {
        let index_path = self.git_dir.join("index");
        let index = Index::load_from(&index_path).unwrap_or_default();
        let ref_store = RefStore::new(&self.git_dir);
        let head_oid_opt = ref_store.resolve_head().ok().and_then(|(_, oid)| oid);

        let head_map = head_oid_opt
            .and_then(|oid| {
                if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                    flatten_tree(store, &c.tree, "").ok()
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let title = format!("Diff: {} ({})", file.path, file.kind.label());

        let diff_text = match file.kind {
            FileStatusKind::StagedNew => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                format_unified_diff(&file.path, &file.path, "", &index_blob, 3)
            }
            FileStatusKind::StagedModified => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                let head_blob = head_map
                    .get(&file.path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                format_unified_diff(&file.path, &file.path, &head_blob, &index_blob, 3)
            }
            FileStatusKind::StagedDeleted => {
                let head_blob = head_map
                    .get(&file.path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                format_unified_diff(&file.path, &file.path, &head_blob, "", 3)
            }
            FileStatusKind::StagedRenamed => {
                let old_p = file.old_path.as_deref().unwrap_or(&file.path);
                let head_blob = head_map
                    .get(old_p)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                format_unified_diff(old_p, &file.path, &head_blob, &index_blob, 3)
            }
            FileStatusKind::UnstagedModified => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                let worktree = read_worktree_file(&self.repo_root, &file.path);
                format_unified_diff(&file.path, &file.path, &index_blob, &worktree, 3)
            }
            FileStatusKind::UnstagedDeleted => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                format_unified_diff(&file.path, &file.path, &index_blob, "", 3)
            }
            FileStatusKind::Untracked => {
                let worktree = read_worktree_file(&self.repo_root, &file.path);
                format_unified_diff(&file.path, &file.path, "", &worktree, 3)
            }
        };

        if let Some(text) = diff_text {
            DiffView::from_unified_text(title, &text)
        } else {
            let mut view = DiffView::new(title);
            view.lines.push(DiffLine::new(
                "Binary file or identical contents",
                DiffLineKind::Normal,
            ));
            view
        }
    }

    fn compute_commit_diff(&self, store: &RepoObjectStore, commit: &CommitItem) -> DiffView {
        let title = format!("Commit: {} - {}", commit.short_oid, commit.summary);
        let mut lines = Vec::new();

        lines.push(DiffLine::new(
            format!("commit {}", commit.oid),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Author: {} <{}>", commit.author, commit.author_email),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Date:   {}", commit.date),
            DiffLineKind::Header,
        ));
        if !commit.parents.is_empty() {
            let parents_str = commit
                .parents
                .iter()
                .map(|p| p.to_string()[..7].to_string())
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(DiffLine::new(
                format!("Parents: {}", parents_str),
                DiffLineKind::Header,
            ));
        }
        lines.push(DiffLine::new("", DiffLineKind::Normal));

        for line in commit.full_message.lines() {
            lines.push(DiffLine::new(format!("    {}", line), DiffLineKind::Normal));
        }
        lines.push(DiffLine::new("", DiffLineKind::Normal));

        // Compute commit diff vs parent tree
        if let Ok(Object::Commit(commit_obj)) = store.read_object(&commit.oid) {
            let commit_map = flatten_tree(store, &commit_obj.tree, "").unwrap_or_default();
            let parent_map = if let Some(parent_oid) = commit.parents.first() {
                if let Ok(Object::Commit(p_commit)) = store.read_object(parent_oid) {
                    flatten_tree(store, &p_commit.tree, "").unwrap_or_default()
                } else {
                    BTreeMap::new()
                }
            } else {
                BTreeMap::new()
            };

            let mut all_paths: Vec<&String> = commit_map.keys().chain(parent_map.keys()).collect();
            all_paths.sort();
            all_paths.dedup();

            for path in all_paths {
                let p_blob = parent_map
                    .get(path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                let c_blob = commit_map
                    .get(path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();

                if p_blob != c_blob {
                    if let Some(diff) = format_unified_diff(path, path, &p_blob, &c_blob, 3) {
                        for d_line in diff.lines() {
                            lines.push(DiffLine::from_raw_line(d_line));
                        }
                    }
                }
            }
        }

        DiffView { title, lines }
    }

    fn compute_branch_view(&self, store: &RepoObjectStore, branch: &BranchItem) -> DiffView {
        let title = format!("Branch: {}", branch.name);
        let mut lines = Vec::new();

        lines.push(DiffLine::new(
            format!("Branch Name: {}", branch.name),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Active HEAD: {}", if branch.is_head { "YES" } else { "NO" }),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Remote Ref:  {}", if branch.is_remote { "YES" } else { "NO" }),
            DiffLineKind::Header,
        ));

        if let Some(oid) = branch.commit_oid {
            lines.push(DiffLine::new(
                format!("Tip Commit:  {}", oid),
                DiffLineKind::Header,
            ));
            if let Ok(Object::Commit(commit)) = store.read_object(&oid) {
                let date_str = chrono::DateTime::from_timestamp(commit.author.time_seconds, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| commit.author.time_seconds.to_string());
                lines.push(DiffLine::new(
                    format!("Author:      {} <{}>", commit.author.name, commit.author.email),
                    DiffLineKind::Header,
                ));
                lines.push(DiffLine::new(
                    format!("Date:        {}", date_str),
                    DiffLineKind::Header,
                ));
                lines.push(DiffLine::new("", DiffLineKind::Normal));
                for line in commit.message.lines() {
                    lines.push(DiffLine::new(format!("    {}", line), DiffLineKind::Normal));
                }
            }
        } else {
            lines.push(DiffLine::new("No commits on this branch yet", DiffLineKind::Normal));
        }

        DiffView { title, lines }
    }

    fn compute_stash_view(&self, store: &RepoObjectStore, stash: &StashItem) -> DiffView {
        let title = format!("stash@{{{}}}: {}", stash.index, stash.message);
        let mut lines = Vec::new();

        lines.push(DiffLine::new(
            format!("stash@{{{}}} commit: {}", stash.index, stash.oid),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Message: {}", stash.message),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new("", DiffLineKind::Normal));

        if let Ok(Object::Commit(stash_commit)) = store.read_object(&stash.oid) {
            if let Some(parent_oid) = stash_commit.parents.first() {
                if let Ok(Object::Commit(parent_commit)) = store.read_object(parent_oid) {
                    let s_map = flatten_tree(store, &stash_commit.tree, "").unwrap_or_default();
                    let p_map = flatten_tree(store, &parent_commit.tree, "").unwrap_or_default();

                    let mut all_paths: Vec<&String> = s_map.keys().chain(p_map.keys()).collect();
                    all_paths.sort();
                    all_paths.dedup();

                    for path in all_paths {
                        let p_blob = p_map
                            .get(path)
                            .map(|(_, oid)| read_blob_text(store, oid))
                            .unwrap_or_default();
                        let s_blob = s_map
                            .get(path)
                            .map(|(_, oid)| read_blob_text(store, oid))
                            .unwrap_or_default();

                        if p_blob != s_blob {
                            if let Some(diff) = format_unified_diff(path, path, &p_blob, &s_blob, 3) {
                                for d_line in diff.lines() {
                                    lines.push(DiffLine::from_raw_line(d_line));
                                }
                            }
                        }
                    }
                }
            }
        }

        DiffView { title, lines }
    }

    /// Toggles staging for the currently selected file in the Files panel.
    pub fn toggle_stage_selected(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Files {
            return Ok(());
        }
        if let Some(file) = self.selected_file().cloned() {
            let path = file.path.clone();
            if file.kind.is_staged() {
                ops::unstage_path(&self.repo_root, &self.git_dir, &path)?;
                self.status_message = Some(format!("✓ Unstaged {}", path));
            } else {
                ops::stage_path(&self.repo_root, &self.git_dir, &path)?;
                self.status_message = Some(format!("✓ Staged {}", path));
            }
            self.refresh()?;
        }
        Ok(())
    }

    /// Stages all files if any are unstaged/untracked, or unstages all if all files are already staged.
    pub fn stage_all(&mut self) -> Result<(), TuiError> {
        let has_unstaged_or_untracked = self.files.iter().any(|f| !f.kind.is_staged());
        if has_unstaged_or_untracked {
            let to_stage: Vec<String> = self
                .files
                .iter()
                .filter(|f| !f.kind.is_staged())
                .map(|f| f.path.clone())
                .collect();
            for path in to_stage {
                ops::stage_path(&self.repo_root, &self.git_dir, &path)?;
            }
            self.status_message = Some("✓ Staged all changes".to_string());
        } else {
            let to_unstage: Vec<String> = self
                .files
                .iter()
                .filter(|f| f.kind.is_staged())
                .map(|f| f.path.clone())
                .collect();
            for path in to_unstage {
                ops::unstage_path(&self.repo_root, &self.git_dir, &path)?;
            }
            self.status_message = Some("✓ Unstaged all changes".to_string());
        }
        self.refresh()?;
        Ok(())
    }

    /// Discards unstaged modifications to the currently selected file.
    pub fn discard_selected_file(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Files {
            return Ok(());
        }
        if let Some(file) = self.selected_file().cloned() {
            if !file.kind.is_staged() {
                let path = file.path.clone();
                ops::discard_path(&self.repo_root, &self.git_dir, &path)?;
                self.status_message = Some(format!("✓ Discarded {}", path));
                self.refresh()?;
            }
        }
        Ok(())
    }

    /// Opens the commit modal if staged changes exist.
    pub fn open_commit_modal(&mut self) {
        let has_staged = self.files.iter().any(|f| f.kind.is_staged());
        if has_staged {
            self.active_modal = ActiveModal::CommitPrompt {
                message: String::new(),
                cursor: 0,
            };
        } else {
            self.status_message = Some("Cannot commit: No files are staged (press Space to stage files)".to_string());
        }
    }

    /// Closes any currently active modal without applying.
    pub fn close_modal(&mut self) {
        self.active_modal = ActiveModal::None;
    }

    /// Appends or inserts a character at the modal cursor.
    pub fn handle_modal_char(&mut self, c: char) {
        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref mut message,
                ref mut cursor,
            } => {
                if *cursor <= message.len() {
                    message.insert(*cursor, c);
                    *cursor += 1;
                }
            }
            ActiveModal::BranchCreate {
                ref mut name,
                ref mut cursor,
            } => {
                if *cursor <= name.len() {
                    name.insert(*cursor, c);
                    *cursor += 1;
                }
            }
            _ => {}
        }
    }

    /// Handles Backspace key inside an active modal text field.
    pub fn handle_modal_backspace(&mut self) {
        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref mut message,
                ref mut cursor,
            } => {
                if *cursor > 0 && *cursor <= message.len() {
                    message.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
            ActiveModal::BranchCreate {
                ref mut name,
                ref mut cursor,
            } => {
                if *cursor > 0 && *cursor <= name.len() {
                    name.remove(*cursor - 1);
                    *cursor -= 1;
                }
            }
            _ => {}
        }
    }

    /// Moves text cursor left inside an active modal.
    pub fn handle_modal_left(&mut self) {
        match self.active_modal {
            ActiveModal::CommitPrompt { ref mut cursor, .. }
            | ActiveModal::BranchCreate { ref mut cursor, .. } => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            _ => {}
        }
    }

    /// Moves text cursor right inside an active modal.
    pub fn handle_modal_right(&mut self) {
        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref message,
                ref mut cursor,
            } => {
                if *cursor < message.len() {
                    *cursor += 1;
                }
            }
            ActiveModal::BranchCreate {
                ref name,
                ref mut cursor,
            } => {
                if *cursor < name.len() {
                    *cursor += 1;
                }
            }
            _ => {}
        }
    }

    /// Submits the active modal dialog action.
    pub fn submit_modal(&mut self) -> Result<(), TuiError> {
        match self.active_modal.clone() {
            ActiveModal::CommitPrompt { message, .. } => {
                let trimmed = message.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Commit aborted: empty commit message".to_string());
                    self.active_modal = ActiveModal::None;
                    return Ok(());
                }

                let commit_oid = ops::create_commit(&self.repo_root, &self.git_dir, trimmed)?;
                let short_sha = &commit_oid.to_string()[..7];
                self.status_message = Some(format!(
                    "✓ [{} {}] {}",
                    self.branch_name, short_sha, trimmed
                ));
                self.active_modal = ActiveModal::None;
                self.refresh()?;
            }
            ActiveModal::BranchCreate { name, .. } => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Branch creation aborted: empty branch name".to_string());
                    self.active_modal = ActiveModal::None;
                    return Ok(());
                }

                ops::create_and_checkout_branch(&self.repo_root, &self.git_dir, trimmed)?;
                self.status_message = Some(format!(
                    "✓ Created and switched to branch '{}'",
                    trimmed
                ));
                self.active_modal = ActiveModal::None;
                self.refresh()?;
            }
            ActiveModal::Help => {
                self.active_modal = ActiveModal::None;
            }
            _ => {
                self.active_modal = ActiveModal::None;
            }
        }
        Ok(())
    }

    /// Checks out the currently selected branch in the Branches panel.
    pub fn checkout_selected_branch(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Branches {
            return Ok(());
        }
        if let Some(b) = self.selected_branch().cloned() {
            if b.is_head {
                self.status_message = Some(format!("Already on branch '{}'", b.name));
                return Ok(());
            }
            if b.is_remote {
                self.status_message = Some(format!("Cannot directly checkout remote branch '{}'", b.name));
                return Ok(());
            }

            ops::checkout_branch(&self.repo_root, &self.git_dir, &b.name)?;
            self.status_message = Some(format!("✓ Switched to branch '{}'", b.name));
            self.refresh()?;
        }
        Ok(())
    }

    /// Opens the modal prompt to create and switch to a new branch.
    pub fn open_create_branch_modal(&mut self) {
        self.active_modal = ActiveModal::BranchCreate {
            name: String::new(),
            cursor: 0,
        };
    }

    /// Deletes the currently selected branch in the Branches panel.
    pub fn delete_selected_branch(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Branches {
            return Ok(());
        }
        if let Some(b) = self.selected_branch().cloned() {
            if b.is_head {
                self.status_message = Some(format!("Cannot delete checked-out branch '{}'", b.name));
                return Ok(());
            }
            if b.is_remote {
                self.status_message = Some(format!("Cannot delete remote tracking branch '{}'", b.name));
                return Ok(());
            }

            ops::delete_branch(&self.git_dir, &b.name)?;
            self.status_message = Some(format!("✓ Deleted branch '{}'", b.name));
            self.refresh()?;
        }
        Ok(())
    }

    /// Pops the selected stash into the working tree.
    pub fn pop_selected_stash(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Stash {
            return Ok(());
        }
        if let Some(s) = self.selected_stash().cloned() {
            ops::pop_stash(&self.repo_root, &self.git_dir, &s.oid)?;
            self.status_message = Some(format!("✓ Popped stash@{{{}}}", s.index));
            self.refresh()?;
        }
        Ok(())
    }

    /// Drops the stash stack.
    pub fn drop_selected_stash(&mut self) -> Result<(), TuiError> {
        if self.active_panel != Panel::Stash {
            return Ok(());
        }
        if let Some(s) = self.selected_stash().cloned() {
            ops::drop_stash(&self.git_dir)?;
            self.status_message = Some(format!("✓ Dropped stash@{{{}}}", s.index));
            self.refresh()?;
        }
        Ok(())
    }

    // Backwards compatibility helpers
    /// Selects next commit or item.
    pub fn next(&mut self) {
        self.next_item();
    }

    /// Selects previous commit or item.
    pub fn previous(&mut self) {
        self.prev_item();
    }

    /// Toggles active tab / panel.
    pub fn toggle_tab(&mut self) {
        self.next_panel();
    }
}

fn collect_ref_files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let full_name = if prefix.is_empty() {
                name
            } else {
                format!("{}/{}", prefix, name)
            };

            if path.is_dir() {
                collect_ref_files(&path, &full_name, out);
            } else if path.is_file() {
                out.push(full_name);
            }
        }
    }
}

fn read_ref_info(ref_file: &Path, store: &RepoObjectStore) -> (Option<ObjectId>, Option<String>) {
    if let Ok(content) = fs::read_to_string(ref_file) {
        if let Ok(oid) = content.trim().parse::<ObjectId>() {
            let summary = if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                c.message.lines().next().map(|s| s.to_string())
            } else {
                None
            };
            return (Some(oid), summary);
        }
    }
    (None, None)
}

fn read_blob_text(store: &RepoObjectStore, oid: &ObjectId) -> String {
    if let Ok(Object::Blob(b)) = store.read_object(oid) {
        String::from_utf8_lossy(&b.data).to_string()
    } else {
        String::new()
    }
}

fn read_worktree_file(repo_root: &Path, rel_path: &str) -> String {
    let full_path = repo_root.join(rel_path);
    if let Ok(data) = fs::read(full_path) {
        String::from_utf8_lossy(&data).to_string()
    } else {
        String::new()
    }
}
