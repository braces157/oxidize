use crate::model::{
    ActiveModal, BranchItem, BranchesTab, CommandPaletteItem, CommitDecoration, CommitItem,
    CommitsTab, ConfirmAction, DiffLine, DiffLineKind, DiffView, FileItem, FileStatusKind,
    FocusedWindow, Panel, ReflogItem, RemoteItem, ResetMode, StashItem, TabMode, TagItem,
};
use crate::ops;
use crate::TuiError;

/// Built-in command palette actions registry.
pub const PALETTE_COMMANDS: &[CommandPaletteItem] = &[
    CommandPaletteItem {
        id: "commit",
        title: "Commit staged changes",
        category: "Commits",
        keybinding: "c",
    },
    CommandPaletteItem {
        id: "amend",
        title: "Amend last commit",
        category: "Commits",
        keybinding: "A",
    },
    CommandPaletteItem {
        id: "push",
        title: "Push to upstream remote",
        category: "Remote",
        keybinding: "P",
    },
    CommandPaletteItem {
        id: "push_force_lease",
        title: "Force-push with lease",
        category: "Remote",
        keybinding: "F",
    },
    CommandPaletteItem {
        id: "pull",
        title: "Pull from remote upstream",
        category: "Remote",
        keybinding: "p",
    },
    CommandPaletteItem {
        id: "fetch_all",
        title: "Fetch all remotes and prune",
        category: "Remote",
        keybinding: "f",
    },
    CommandPaletteItem {
        id: "add_remote",
        title: "Add new remote repository",
        category: "Remote",
        keybinding: "a",
    },
    CommandPaletteItem {
        id: "branch_create",
        title: "Create new branch",
        category: "Branches",
        keybinding: "n",
    },
    CommandPaletteItem {
        id: "branch_rename",
        title: "Rename active branch",
        category: "Branches",
        keybinding: "R",
    },
    CommandPaletteItem {
        id: "tag_create",
        title: "Create new tag on selected commit",
        category: "Tags",
        keybinding: "t",
    },
    CommandPaletteItem {
        id: "stash_save",
        title: "Save stash",
        category: "Stash",
        keybinding: "s",
    },
    CommandPaletteItem {
        id: "stash_pop",
        title: "Pop latest stash",
        category: "Stash",
        keybinding: "g",
    },
    CommandPaletteItem {
        id: "worktrees",
        title: "Manage worktrees",
        category: "Worktree",
        keybinding: "w",
    },
    CommandPaletteItem {
        id: "submodules",
        title: "Manage submodules",
        category: "Submodules",
        keybinding: "S",
    },
    CommandPaletteItem {
        id: "bisect",
        title: "Git bisect menu",
        category: "Bisect",
        keybinding: "B",
    },
    CommandPaletteItem {
        id: "custom_patches",
        title: "Custom patches basket",
        category: "Diff",
        keybinding: "Ctrl+P",
    },
    CommandPaletteItem {
        id: "provider_links",
        title: "Open web provider links",
        category: "Web",
        keybinding: "O",
    },
    CommandPaletteItem {
        id: "help",
        title: "Show keybindings help",
        category: "General",
        keybinding: "?",
    },
    CommandPaletteItem {
        id: "refresh",
        title: "Refresh repository status",
        category: "General",
        keybinding: "r",
    },
    CommandPaletteItem {
        id: "quit",
        title: "Quit Oxidize TUI",
        category: "General",
        keybinding: "q",
    },
];
use oxidize_config::GitIgnore;
use oxidize_core::id::ObjectId;
use oxidize_core::object::Object;
use oxidize_diff::format_unified_diff;
use oxidize_index::{
    compute_status_with_ignore, flatten_tree, Index, StagedChange, UnstagedChange,
};
use oxidize_pack::RepoObjectStore;
use oxidize_refs::{RefError, RefStore};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// Result of an asynchronous background operation.
#[derive(Debug)]
pub enum BackgroundJobResult {
    Success(String),
    Error(String),
}

/// Active background task handle.
pub struct BackgroundJob {
    pub description: String,
    pub receiver: std::sync::mpsc::Receiver<BackgroundJobResult>,
}

/// Core state machine for the multi-panel interactive TUI.
pub struct App {
    /// Root directory of the repository working tree.
    pub repo_root: PathBuf,
    /// Path to the per-worktree `.git` directory.
    pub git_dir: PathBuf,
    /// Path to the common `.git` directory containing shared refs and objects.
    pub common_dir: PathBuf,
    /// Whether this repository is bare.
    pub is_bare: bool,
    /// Currently checked-out branch or "HEAD (detached)".
    pub branch_name: String,
    /// Active docked panel in focus.
    pub active_panel: Panel,
    /// Active sub-tab inside Branches panel (Local, Remotes, Tags).
    pub branches_tab: BranchesTab,
    /// Active sub-tab inside Commits panel (Commits, Reflog).
    pub commits_tab: CommitsTab,
    /// Focused window (Sidebar or Inspector for Vim h/l navigation).
    pub focused_window: FocusedWindow,

    /// Changed files in working tree and staging area.
    pub files: Vec<FileItem>,
    /// Selected index in files list.
    pub files_selected: usize,

    /// Local and remote branches.
    pub branches: Vec<BranchItem>,
    /// Selected index in branches list.
    pub branches_selected: usize,

    /// Configured remote repositories.
    pub remotes: Vec<RemoteItem>,
    /// Selected index in remotes list.
    pub remotes_selected: usize,

    /// Repository tags.
    pub tags: Vec<TagItem>,
    /// Selected index in tags list.
    pub tags_selected: usize,

    /// Loaded commit history.
    pub commits: Vec<CommitItem>,
    /// Selected index in commits list.
    pub commits_selected: usize,

    /// Reflog history entries.
    pub reflog: Vec<ReflogItem>,
    /// Selected index in reflog list.
    pub reflog_selected: usize,

    /// Stash stack items.
    pub stashes: Vec<StashItem>,
    /// Selected index in stash list.
    pub stashes_selected: usize,

    /// Ahead/behind commits count compared to remote upstream.
    pub ahead_behind: (usize, usize),

    /// Vertical scroll offset in the right Inspector pane.
    pub inspector_scroll: usize,
    /// Cached rendered diff view for the current selection.
    pub cached_diff: Option<DiffView>,
    /// Cache of rendered commit diffs indexed by immutable commit OID.
    pub commit_diff_cache: HashMap<ObjectId, DiffView>,
    /// Search or filter string for commits list.
    pub commit_search_filter: Option<String>,
    /// Active rebase or replay sequencer state, if an operation is currently in progress.
    pub sequencer_state: Option<crate::sequencer::SequencerState>,
    /// Persistent custom patch basket collecting hunks across commits and files.
    pub custom_patch_basket: oxidize_diff::CustomPatchBasket,
    /// Submodules discovered in repository.
    pub submodules: Vec<crate::ops::SubmoduleItem>,
    /// Active git bisect state and progress.
    pub bisect_state: crate::ops::BisectState,
    /// Navigation history stack of repository roots (for submodules navigation).
    pub repo_history: Vec<PathBuf>,

    /// Currently displayed modal dialog, if any.
    pub active_modal: ActiveModal,
    /// One-line ephemeral notification or feedback message.
    pub status_message: Option<String>,
    /// Termination request flag.
    pub should_quit: bool,
    /// Active asynchronous background job.
    pub active_job: Option<BackgroundJob>,

    // Legacy tab mode compatibility
    pub active_tab: TabMode,
    pub status_lines: Vec<String>,
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
            common_dir: PathBuf::new(),
            is_bare: false,
            branch_name: "master".to_string(),
            active_panel: Panel::Files,
            branches_tab: BranchesTab::Local,
            commits_tab: CommitsTab::Commits,
            focused_window: FocusedWindow::Sidebar,
            files: Vec::new(),
            files_selected: 0,
            branches: Vec::new(),
            branches_selected: 0,
            remotes: Vec::new(),
            remotes_selected: 0,
            tags: Vec::new(),
            tags_selected: 0,
            commits: Vec::new(),
            commits_selected: 0,
            reflog: Vec::new(),
            reflog_selected: 0,
            stashes: Vec::new(),
            stashes_selected: 0,
            ahead_behind: (0, 0),
            inspector_scroll: 0,
            cached_diff: None,
            commit_diff_cache: HashMap::new(),
            commit_search_filter: None,
            sequencer_state: None,
            custom_patch_basket: oxidize_diff::CustomPatchBasket::new(),
            submodules: Vec::new(),
            bisect_state: crate::ops::BisectState::default(),
            repo_history: Vec::new(),
            active_modal: ActiveModal::None,
            status_message: None,
            should_quit: false,
            active_job: None,
            active_tab: TabMode::Commits,
            status_lines: Vec::new(),
            selected_index: 0,
        }
    }

    /// Loads repository data from disk into the multi-panel state.
    pub fn load_repository(&mut self, git_dir: &Path) -> Result<(), TuiError> {
        let ctx = oxidize_core::RepoContext::discover(git_dir).map_err(TuiError::Core)?;
        self.git_dir = ctx.git_dir.clone();
        self.common_dir = ctx.common_dir.clone();
        self.is_bare = ctx.is_bare;
        self.repo_root = ctx.worktree.unwrap_or_else(|| self.git_dir.clone());
        self.sequencer_state = crate::sequencer::SequencerState::load(&self.git_dir)
            .ok()
            .flatten();

        let store = RepoObjectStore::open_with_common_dir(&self.git_dir, &self.common_dir)
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        let ref_store = RefStore::with_common_dir(&self.git_dir, &self.common_dir);

        // 1. Resolve HEAD branch and commit
        let (branch, head_oid_opt) = ref_store
            .resolve_head()
            .map_err(|e| TuiError::Terminal(e.to_string()))?;
        self.branch_name = branch.clone();

        // 2. Load commits with topological graph lanes & decorations
        self.load_commits_with_graph(&store, &ref_store, head_oid_opt);

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
        let index_path = self.git_dir.join("index");
        let index = if index_path.exists() {
            Index::load_from(&index_path)
                .map_err(|e| TuiError::Terminal(format!("Corrupt or unreadable index: {}", e)))?
        } else {
            Index::default()
        };
        let head_tree = head_oid_opt.and_then(|oid| {
            if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                Some(c.tree)
            } else {
                None
            }
        });

        let gitignore = GitIgnore::load_from_dir(&self.repo_root)
            .map_err(|e| TuiError::Terminal(format!("Failed to load ignore rules: {}", e)))?;
        let status = compute_status_with_ignore(
            &self.repo_root,
            &index,
            head_tree.as_ref(),
            &store,
            Some(&|p, is_dir| gitignore.is_ignored(p, is_dir)),
        )
        .map_err(|e| TuiError::Terminal(format!("Status computation failed: {}", e)))?;
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
        for unmerged in &status.unmerged {
            self.files.push(FileItem {
                path: unmerged.clone(),
                old_path: None,
                kind: FileStatusKind::Conflicted,
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

        // 6. Load remotes, tags, and reflog
        self.remotes = ops::read_remotes(&self.common_dir);
        self.tags = ops::read_tags(&self.common_dir);
        self.reflog = ops::read_reflog(&self.git_dir);
        self.bisect_state = ops::get_bisect_state(&self.git_dir)?;
        self.submodules = ops::list_submodules(&self.repo_root, &self.git_dir)?;

        // 7. Calculate ahead / behind relative to upstream
        self.ahead_behind = self.compute_ahead_behind(&store);

        // Clamp selection indices
        self.clamp_selections();

        // Update inspector view
        self.update_inspector();

        Ok(())
    }

    /// Reloads repository data in place while preserving active selection identifiers.
    pub fn refresh(&mut self) -> Result<(), TuiError> {
        let path = if !self.repo_root.as_os_str().is_empty() {
            self.repo_root.clone()
        } else {
            self.git_dir.clone()
        };
        if !path.as_os_str().is_empty() {
            let sel_file = self.selected_file().map(|f| f.path.clone());
            let sel_branch = self.selected_branch().map(|b| b.name.clone());
            let sel_commit = self.selected_commit().map(|c| c.oid);
            let sel_stash = self.selected_stash().map(|s| s.index);

            self.load_repository(&path)?;

            if let Some(path) = sel_file {
                if let Some(pos) = self.files.iter().position(|f| f.path == path) {
                    self.files_selected = pos;
                }
            }
            if let Some(name) = sel_branch {
                let local_branches: Vec<&BranchItem> =
                    self.branches.iter().filter(|b| !b.is_remote).collect();
                if let Some(pos) = local_branches.iter().position(|b| b.name == name) {
                    self.branches_selected = pos;
                }
            }
            if let Some(oid) = sel_commit {
                if let Some(pos) = self.commits.iter().position(|c| c.oid == oid) {
                    self.commits_selected = pos;
                }
            }
            if let Some(idx) = sel_stash {
                if let Some(pos) = self.stashes.iter().position(|s| s.index == idx) {
                    self.stashes_selected = pos;
                }
            }
            self.clamp_selections();
            self.update_inspector();
        }
        Ok(())
    }

    fn load_commits_with_graph(
        &mut self,
        store: &RepoObjectStore,
        ref_store: &RefStore,
        head_oid_opt: Option<ObjectId>,
    ) {
        self.commits.clear();

        // 1. Gather roots: HEAD, all local branches, all tags
        let mut root_oids = Vec::new();
        if let Some(h) = head_oid_opt {
            root_oids.push(h);
        }
        if let Ok(branches) = ref_store.list_branches() {
            for oid in branches.values() {
                if !root_oids.contains(oid) {
                    root_oids.push(*oid);
                }
            }
        }
        if let Ok(tags) = ref_store.list_tags() {
            for oid in tags.values() {
                if !root_oids.contains(oid) {
                    root_oids.push(*oid);
                }
            }
        }

        if root_oids.is_empty() {
            return;
        }

        struct RawCommit {
            parents: Vec<ObjectId>,
            committer_time: i64,
            author_name: String,
            author_email: String,
            date_str: String,
            summary: String,
            full_message: String,
        }

        let mut commits_map: HashMap<ObjectId, RawCommit> = HashMap::new();
        let mut child_counts: HashMap<ObjectId, usize> = HashMap::new();
        let mut visited: HashSet<ObjectId> = HashSet::new();
        let mut queue: VecDeque<ObjectId> = VecDeque::new();

        for root in &root_oids {
            if visited.insert(*root) {
                queue.push_back(*root);
            }
        }

        while let Some(oid) = queue.pop_front() {
            if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                for &p in &c.parents {
                    if visited.insert(p) {
                        queue.push_back(p);
                    }
                    *child_counts.entry(p).or_insert(0) += 1;
                }
                let first_line = c.message.lines().next().unwrap_or("").to_string();
                let date_str = chrono::DateTime::from_timestamp(c.author.time_seconds, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| c.author.time_seconds.to_string());
                commits_map.insert(
                    oid,
                    RawCommit {
                        parents: c.parents,
                        committer_time: c.committer.time_seconds,
                        author_name: c.author.name,
                        author_email: c.author.email,
                        date_str,
                        summary: first_line,
                        full_message: c.message,
                    },
                );
            }
        }

        // 2. Topological sort with committer timestamp priority
        for oid in commits_map.keys() {
            child_counts.entry(*oid).or_insert(0);
        }

        let mut heap = std::collections::BinaryHeap::new();
        for (&oid, raw) in &commits_map {
            if child_counts.get(&oid).copied().unwrap_or(0) == 0 {
                heap.push((raw.committer_time, oid));
            }
        }

        let mut ordered_oids = Vec::new();
        while let Some((_ts, oid)) = heap.pop() {
            ordered_oids.push(oid);
            if let Some(raw) = commits_map.get(&oid) {
                for &p in &raw.parents {
                    if let Some(cnt) = child_counts.get_mut(&p) {
                        *cnt = cnt.saturating_sub(1);
                        if *cnt == 0 {
                            if let Some(p_raw) = commits_map.get(&p) {
                                heap.push((p_raw.committer_time, p));
                            }
                        }
                    }
                }
            }
        }

        // Add any remaining commits (in case of cycles or orphans)
        for &oid in commits_map.keys() {
            if !ordered_oids.contains(&oid) {
                ordered_oids.push(oid);
            }
        }

        // 3. Collect decorations
        let mut decorations_map: HashMap<ObjectId, Vec<CommitDecoration>> = HashMap::new();
        let (active_branch, head_oid) = ref_store
            .resolve_head()
            .unwrap_or_else(|_| ("HEAD".to_string(), None));
        if let Some(h_oid) = head_oid {
            if active_branch == "HEAD" || active_branch.is_empty() {
                decorations_map
                    .entry(h_oid)
                    .or_default()
                    .push(CommitDecoration::Head("HEAD (detached)".to_string()));
            } else {
                decorations_map
                    .entry(h_oid)
                    .or_default()
                    .push(CommitDecoration::Head(active_branch.clone()));
            }
        }

        if let Ok(branches) = ref_store.list_branches() {
            for (b_name, b_oid) in branches {
                if Some(b_oid) == head_oid && b_name == active_branch {
                    continue;
                }
                decorations_map
                    .entry(b_oid)
                    .or_default()
                    .push(CommitDecoration::Branch(b_name));
            }
        }

        if let Ok(remotes) = ref_store.list_remotes() {
            for (r_name, r_oid) in remotes {
                decorations_map
                    .entry(r_oid)
                    .or_default()
                    .push(CommitDecoration::Remote(r_name));
            }
        }

        if let Ok(tags) = ref_store.list_tags() {
            for (t_name, t_oid) in tags {
                decorations_map
                    .entry(t_oid)
                    .or_default()
                    .push(CommitDecoration::Tag(t_name));
            }
        }

        // 4. DAG Lane assignment and prefix rendering
        let mut active_lanes: Vec<Option<ObjectId>> = Vec::new();
        for oid in ordered_oids {
            let raw = match commits_map.get(&oid) {
                Some(r) => r,
                None => continue,
            };

            let lane = if let Some(idx) = active_lanes.iter().position(|l| *l == Some(oid)) {
                active_lanes[idx] = None;
                idx
            } else if let Some(empty_idx) = active_lanes.iter().position(|l| l.is_none()) {
                empty_idx
            } else {
                active_lanes.push(None);
                active_lanes.len() - 1
            };

            let last_some = active_lanes.iter().rposition(|l| l.is_some()).unwrap_or(0);
            let max_col = lane.max(last_some);
            let mut prefix = String::new();
            for c in 0..=max_col {
                if c == lane {
                    prefix.push('*');
                    prefix.push(' ');
                } else if active_lanes.get(c).copied().flatten().is_some() {
                    prefix.push('|');
                    prefix.push(' ');
                } else {
                    prefix.push(' ');
                    prefix.push(' ');
                }
            }

            // Update active_lanes for next rows
            if raw.parents.is_empty() {
                if lane < active_lanes.len() {
                    active_lanes[lane] = None;
                }
            } else if raw.parents.len() == 1 {
                let p = raw.parents[0];
                if active_lanes.contains(&Some(p)) {
                    active_lanes[lane] = None;
                } else {
                    active_lanes[lane] = Some(p);
                }
            } else {
                // Merge commit
                let first_p = raw.parents[0];
                if active_lanes.contains(&Some(first_p)) {
                    active_lanes[lane] = None;
                } else {
                    active_lanes[lane] = Some(first_p);
                }
                for &other_p in &raw.parents[1..] {
                    if !active_lanes.contains(&Some(other_p)) {
                        if let Some(empty_idx) = active_lanes.iter().position(|l| l.is_none()) {
                            active_lanes[empty_idx] = Some(other_p);
                        } else {
                            active_lanes.push(Some(other_p));
                        }
                    }
                }
            }

            while active_lanes.last() == Some(&None) {
                active_lanes.pop();
            }

            let decs = decorations_map.remove(&oid).unwrap_or_default();

            self.commits.push(CommitItem {
                oid,
                short_oid: oid.to_string()[..7].to_string(),
                author: raw.author_name.clone(),
                author_email: raw.author_email.clone(),
                date: raw.date_str.clone(),
                summary: raw.summary.clone(),
                full_message: raw.full_message.clone(),
                parents: raw.parents.clone(),
                graph_prefix: prefix,
                lane,
                decorations: decs,
            });
        }
    }

    fn load_branches(&mut self, store: &RepoObjectStore, active_branch: &str) {
        let ref_store = RefStore::with_common_dir(&self.git_dir, &self.common_dir);

        // 1. Local branches (reads loose & packed refs from common_dir and git_dir)
        if let Ok(local_branches) = ref_store.list_branches() {
            for (name, oid) in local_branches {
                let is_head = name == active_branch;
                let summary = if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                    c.message.lines().next().unwrap_or("").trim().to_string()
                } else {
                    String::new()
                };
                self.branches.push(BranchItem {
                    name,
                    is_head,
                    is_remote: false,
                    commit_oid: Some(oid),
                    summary: if summary.is_empty() {
                        None
                    } else {
                        Some(summary)
                    },
                });
            }
        }

        // 2. Remote branches
        if let Ok(remote_branches) = ref_store.list_remotes() {
            for (name, oid) in remote_branches {
                let summary = if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                    c.message.lines().next().unwrap_or("").trim().to_string()
                } else {
                    String::new()
                };
                self.branches.push(BranchItem {
                    name,
                    is_head: false,
                    is_remote: true,
                    commit_oid: Some(oid),
                    summary: if summary.is_empty() {
                        None
                    } else {
                        Some(summary)
                    },
                });
            }
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
        let stash_log = {
            let common_log = self.common_dir.join("logs").join("refs").join("stash");
            if common_log.exists() {
                common_log
            } else {
                self.git_dir.join("logs").join("refs").join("stash")
            }
        };

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
            let stash_ref = {
                let common_ref = self.common_dir.join("refs").join("stash");
                if common_ref.exists() {
                    common_ref
                } else {
                    self.git_dir.join("refs").join("stash")
                }
            };
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

        let local_count = self.branches.iter().filter(|b| !b.is_remote).count();
        if local_count > 0 {
            if self.branches_selected >= local_count {
                self.branches_selected = local_count - 1;
            }
        } else {
            self.branches_selected = 0;
        }

        if !self.remotes.is_empty() {
            if self.remotes_selected >= self.remotes.len() {
                self.remotes_selected = self.remotes.len() - 1;
            }
        } else {
            self.remotes_selected = 0;
        }

        if !self.tags.is_empty() {
            if self.tags_selected >= self.tags.len() {
                self.tags_selected = self.tags.len() - 1;
            }
        } else {
            self.tags_selected = 0;
        }

        if !self.commits.is_empty() {
            if self.commits_selected >= self.commits.len() {
                self.commits_selected = self.commits.len() - 1;
            }
        } else {
            self.commits_selected = 0;
        }
        self.selected_index = self.commits_selected;

        if !self.reflog.is_empty() {
            if self.reflog_selected >= self.reflog.len() {
                self.reflog_selected = self.reflog.len() - 1;
            }
        } else {
            self.reflog_selected = 0;
        }

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

    /// Selects a specific file item by index and refreshes the inspector.
    pub fn select_file_index(&mut self, idx: usize) {
        if idx < self.files.len() {
            self.files_selected = idx;
            self.inspector_scroll = 0;
            self.update_inspector();
        }
    }

    /// Selects a specific branch/remote/tag item by index and refreshes the inspector.
    pub fn select_branch_index(&mut self, idx: usize) {
        match self.branches_tab {
            BranchesTab::Local => {
                let local_count = self.branches.iter().filter(|b| !b.is_remote).count();
                if idx < local_count {
                    self.branches_selected = idx;
                    self.inspector_scroll = 0;
                    self.update_inspector();
                }
            }
            BranchesTab::Remotes => {
                if idx < self.remotes.len() {
                    self.remotes_selected = idx;
                    self.inspector_scroll = 0;
                    self.update_inspector();
                }
            }
            BranchesTab::Tags => {
                if idx < self.tags.len() {
                    self.tags_selected = idx;
                    self.inspector_scroll = 0;
                    self.update_inspector();
                }
            }
        }
    }

    /// Selects a specific commit or reflog item by index and refreshes the inspector.
    pub fn select_commit_index(&mut self, idx: usize) {
        match self.commits_tab {
            CommitsTab::Commits => {
                if idx < self.commits.len() {
                    self.commits_selected = idx;
                    self.selected_index = idx;
                    self.inspector_scroll = 0;
                    self.update_inspector();
                }
            }
            CommitsTab::Reflog => {
                if idx < self.reflog.len() {
                    self.reflog_selected = idx;
                    self.inspector_scroll = 0;
                    self.update_inspector();
                }
            }
        }
    }

    /// Selects a specific stash item by index and refreshes the inspector.
    pub fn select_stash_index(&mut self, idx: usize) {
        if idx < self.stashes.len() {
            self.stashes_selected = idx;
            self.inspector_scroll = 0;
            self.update_inspector();
        }
    }

    /// Moves selection down within the currently active panel or scrolls inspector.
    pub fn next_item(&mut self) {
        if self.focused_window == FocusedWindow::Inspector {
            self.scroll_inspector_down(1);
            return;
        }

        match self.active_panel {
            Panel::Status => {}
            Panel::Files => {
                if !self.files.is_empty() && self.files_selected + 1 < self.files.len() {
                    self.files_selected += 1;
                }
            }
            Panel::Branches => match self.branches_tab {
                BranchesTab::Local => {
                    let local_count = self.branches.iter().filter(|b| !b.is_remote).count();
                    if local_count > 0 && self.branches_selected + 1 < local_count {
                        self.branches_selected += 1;
                    }
                }
                BranchesTab::Remotes => {
                    if !self.remotes.is_empty() && self.remotes_selected + 1 < self.remotes.len() {
                        self.remotes_selected += 1;
                    }
                }
                BranchesTab::Tags => {
                    if !self.tags.is_empty() && self.tags_selected + 1 < self.tags.len() {
                        self.tags_selected += 1;
                    }
                }
            },
            Panel::Commits => match self.commits_tab {
                CommitsTab::Commits => {
                    let indices = self.filtered_commit_indices();
                    if let Some(pos) = indices.iter().position(|&i| i == self.commits_selected) {
                        if pos + 1 < indices.len() {
                            self.commits_selected = indices[pos + 1];
                            self.selected_index = self.commits_selected;
                        }
                    } else if let Some(&first) = indices.first() {
                        self.commits_selected = first;
                        self.selected_index = self.commits_selected;
                    }
                }
                CommitsTab::Reflog => {
                    if !self.reflog.is_empty() && self.reflog_selected + 1 < self.reflog.len() {
                        self.reflog_selected += 1;
                    }
                }
            },
            Panel::Stash => {
                if !self.stashes.is_empty() && self.stashes_selected + 1 < self.stashes.len() {
                    self.stashes_selected += 1;
                }
            }
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Moves selection up within the currently active panel or scrolls inspector.
    pub fn prev_item(&mut self) {
        if self.focused_window == FocusedWindow::Inspector {
            self.scroll_inspector_up(1);
            return;
        }

        match self.active_panel {
            Panel::Status => {}
            Panel::Files => {
                if self.files_selected > 0 {
                    self.files_selected -= 1;
                }
            }
            Panel::Branches => match self.branches_tab {
                BranchesTab::Local => {
                    if self.branches_selected > 0 {
                        self.branches_selected -= 1;
                    }
                }
                BranchesTab::Remotes => {
                    if self.remotes_selected > 0 {
                        self.remotes_selected -= 1;
                    }
                }
                BranchesTab::Tags => {
                    if self.tags_selected > 0 {
                        self.tags_selected -= 1;
                    }
                }
            },
            Panel::Commits => match self.commits_tab {
                CommitsTab::Commits => {
                    let indices = self.filtered_commit_indices();
                    if let Some(pos) = indices.iter().position(|&i| i == self.commits_selected) {
                        if pos > 0 {
                            self.commits_selected = indices[pos - 1];
                            self.selected_index = self.commits_selected;
                        }
                    } else if let Some(&first) = indices.first() {
                        self.commits_selected = first;
                        self.selected_index = self.commits_selected;
                    }
                }
                CommitsTab::Reflog => {
                    if self.reflog_selected > 0 {
                        self.reflog_selected -= 1;
                    }
                }
            },
            Panel::Stash => {
                if self.stashes_selected > 0 {
                    self.stashes_selected -= 1;
                }
            }
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Switches to next tab within the active panel (bound to ']').
    pub fn next_tab(&mut self) {
        match self.active_panel {
            Panel::Branches => {
                self.branches_tab = self.branches_tab.next();
            }
            Panel::Commits => {
                self.commits_tab = self.commits_tab.next();
            }
            _ => {}
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Switches to previous tab within the active panel (bound to '[').
    pub fn prev_tab(&mut self) {
        match self.active_panel {
            Panel::Branches => {
                self.branches_tab = self.branches_tab.prev();
            }
            Panel::Commits => {
                self.commits_tab = self.commits_tab.prev();
            }
            _ => {}
        }
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Focuses the right-hand Inspector window (Vim 'l' or Enter).
    pub fn focus_inspector(&mut self) {
        self.focused_window = FocusedWindow::Inspector;
    }

    /// Focuses the left sidebar panels (Vim 'h' or Esc).
    pub fn focus_sidebar(&mut self) {
        self.focused_window = FocusedWindow::Sidebar;
    }

    /// Toggles focus between sidebar and inspector.
    pub fn toggle_focus(&mut self) {
        self.focused_window = match self.focused_window {
            FocusedWindow::Sidebar => FocusedWindow::Inspector,
            FocusedWindow::Inspector => FocusedWindow::Sidebar,
        };
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

    /// Scrolls inspector to the top line (line 0).
    pub fn scroll_inspector_top(&mut self) {
        self.inspector_scroll = 0;
    }

    /// Scrolls inspector to the bottom line.
    pub fn scroll_inspector_bottom(&mut self) {
        let max_lines = self
            .cached_diff
            .as_ref()
            .map(|d| d.lines.len())
            .unwrap_or(0);
        self.inspector_scroll = max_lines.saturating_sub(1);
    }

    /// Currently highlighted file item, if any.
    pub fn selected_file(&self) -> Option<&FileItem> {
        self.files.get(self.files_selected)
    }

    /// Currently highlighted branch item, if any.
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        let local_branches: Vec<&BranchItem> =
            self.branches.iter().filter(|b| !b.is_remote).collect();
        local_branches.get(self.branches_selected).copied()
    }

    /// Currently highlighted remote item, if any.
    pub fn selected_remote(&self) -> Option<&RemoteItem> {
        self.remotes.get(self.remotes_selected)
    }

    /// Currently highlighted tag item, if any.
    pub fn selected_tag(&self) -> Option<&TagItem> {
        self.tags.get(self.tags_selected)
    }

    /// Returns indices of commits matching the search filter, or all commits if no filter is active.
    pub fn filtered_commit_indices(&self) -> Vec<usize> {
        match &self.commit_search_filter {
            None => (0..self.commits.len()).collect(),
            Some(query) => self
                .commits
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    c.summary.to_lowercase().contains(query)
                        || c.short_oid.to_lowercase().contains(query)
                        || c.oid.to_string().to_lowercase().contains(query)
                        || c.author.to_lowercase().contains(query)
                })
                .map(|(i, _)| i)
                .collect(),
        }
    }

    /// Currently highlighted commit item, if any.
    pub fn selected_commit(&self) -> Option<&CommitItem> {
        self.commits.get(self.commits_selected)
    }

    /// Currently highlighted reflog item, if any.
    pub fn selected_reflog(&self) -> Option<&ReflogItem> {
        self.reflog.get(self.reflog_selected)
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
            Panel::Status => {
                self.cached_diff = Some(self.compute_status_overview());
            }
            Panel::Files => {
                if let Some(file) = self.selected_file().cloned() {
                    self.cached_diff = Some(self.compute_file_diff(&store, &file));
                } else {
                    self.cached_diff = Some(DiffView::new("No changed files in working tree"));
                }
            }
            Panel::Branches => match self.branches_tab {
                BranchesTab::Local => {
                    if let Some(branch) = self.selected_branch().cloned() {
                        self.cached_diff = Some(self.compute_branch_view(&store, &branch));
                    } else {
                        self.cached_diff = Some(DiffView::new("No local branches"));
                    }
                }
                BranchesTab::Remotes => {
                    self.cached_diff = Some(self.compute_remotes_view());
                }
                BranchesTab::Tags => {
                    self.cached_diff = Some(self.compute_tags_view(&store));
                }
            },
            Panel::Commits => match self.commits_tab {
                CommitsTab::Commits => {
                    if let Some(commit) = self.selected_commit().cloned() {
                        self.cached_diff = Some(self.compute_commit_diff(&store, &commit));
                    } else {
                        self.cached_diff = Some(DiffView::new("No commits in repository"));
                    }
                }
                CommitsTab::Reflog => {
                    self.cached_diff = Some(self.compute_reflog_view(&store));
                }
            },
            Panel::Stash => {
                if let Some(stash) = self.selected_stash().cloned() {
                    self.cached_diff = Some(self.compute_stash_view(&store, &stash));
                } else {
                    self.cached_diff = Some(DiffView::new("Stash stack is empty"));
                }
            }
        }
    }

    fn compute_ahead_behind(&self, store: &RepoObjectStore) -> (usize, usize) {
        let config_path = self.common_dir.join("config");
        let config = if config_path.exists() {
            oxidize_config::GitConfig::load_from_file(&config_path).unwrap_or_default()
        } else {
            oxidize_config::GitConfig::new()
        };

        let remote_name = config
            .get("branch", Some(&self.branch_name), "remote")
            .unwrap_or("origin");
        let merge_ref = config.get("branch", Some(&self.branch_name), "merge");
        let target_remote_branch = if let Some(m) = merge_ref {
            let short = m.strip_prefix("refs/heads/").unwrap_or(m);
            format!("{}/{}", remote_name, short)
        } else {
            format!("{}/{}", remote_name, self.branch_name)
        };

        let remote_branch = self.branches.iter().find(|b| {
            b.is_remote
                && (b.name == target_remote_branch
                    || b.name == format!("origin/{}", self.branch_name)
                    || b.name.ends_with(&format!("/{}", self.branch_name)))
        });

        if let Some(rb) = remote_branch {
            if let (Some(head_commit), Some(remote_oid)) = (self.commits.first(), rb.commit_oid) {
                if head_commit.oid == remote_oid {
                    return (0, 0);
                }

                let head_set: HashSet<ObjectId> = self.commits.iter().map(|c| c.oid).collect();
                let mut remote_set = HashSet::new();
                let mut queue = VecDeque::new();
                queue.push_back(remote_oid);
                remote_set.insert(remote_oid);

                while let Some(oid) = queue.pop_front() {
                    if let Ok(Object::Commit(c)) = store.read_object(&oid) {
                        for p in c.parents {
                            if remote_set.insert(p) {
                                queue.push_back(p);
                            }
                        }
                    }
                }

                let ahead = head_set
                    .iter()
                    .filter(|oid| !remote_set.contains(oid))
                    .count();
                let behind = remote_set
                    .iter()
                    .filter(|oid| !head_set.contains(oid))
                    .count();
                return (ahead, behind);
            }
        }
        (0, 0)
    }

    fn compute_status_overview(&self) -> DiffView {
        let mut lines = Vec::new();
        let repo_name = self
            .repo_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Repository".to_string());

        lines.push(DiffLine::new(
            format!("Repository:   {}", repo_name),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!("Path:         {}", self.repo_root.display()),
            DiffLineKind::Normal,
        ));
        lines.push(DiffLine::new(
            format!("Branch:       * {}", self.branch_name),
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new(
            format!(
                "Upstream:     ↑{} ahead, ↓{} behind",
                self.ahead_behind.0, self.ahead_behind.1
            ),
            DiffLineKind::Normal,
        ));

        let staged_count = self.files.iter().filter(|f| f.kind.is_staged()).count();
        let unstaged_count = self
            .files
            .iter()
            .filter(|f| !f.kind.is_staged() && f.kind != FileStatusKind::Untracked)
            .count();
        let untracked_count = self
            .files
            .iter()
            .filter(|f| f.kind == FileStatusKind::Untracked)
            .count();

        lines.push(DiffLine::new(
            format!(
                "Working Tree: {} staged, {} unstaged, {} untracked",
                staged_count, unstaged_count, untracked_count
            ),
            if staged_count + unstaged_count + untracked_count == 0 {
                DiffLineKind::Addition
            } else {
                DiffLineKind::Deletion
            },
        ));
        lines.push(DiffLine::new(
            format!("Total Commits: {}", self.commits.len()),
            DiffLineKind::Normal,
        ));
        lines.push(DiffLine::new(
            format!(
                "Engine:        Oxidize LazyOx v{} (Pure Rust Git)",
                env!("CARGO_PKG_VERSION")
            ),
            DiffLineKind::Normal,
        ));

        lines.push(DiffLine::new("", DiffLineKind::Normal));
        lines.push(DiffLine::new(
            "--- Configured Remotes ---",
            DiffLineKind::Header,
        ));
        if self.remotes.is_empty() {
            lines.push(DiffLine::new(
                "  (no remotes configured)",
                DiffLineKind::Context,
            ));
        } else {
            for r in &self.remotes {
                lines.push(DiffLine::new(
                    format!("  {} -> {}", r.name, r.url),
                    DiffLineKind::Normal,
                ));
            }
        }

        lines.push(DiffLine::new("", DiffLineKind::Normal));
        lines.push(DiffLine::new(
            "--- Recent Commits ---",
            DiffLineKind::Header,
        ));
        if self.commits.is_empty() {
            lines.push(DiffLine::new("  (no commits yet)", DiffLineKind::Context));
        } else {
            for c in self.commits.iter().take(5) {
                lines.push(DiffLine::new(
                    format!("  * {} {} ({})", c.short_oid, c.summary, c.author),
                    DiffLineKind::Normal,
                ));
            }
        }

        DiffView {
            title: "Repository Status Overview".to_string(),
            lines,
            ..Default::default()
        }
    }

    fn compute_remotes_view(&self) -> DiffView {
        let mut lines = Vec::new();
        lines.push(DiffLine::new(
            "Configured Remote Repositories",
            DiffLineKind::Header,
        ));
        lines.push(DiffLine::new("", DiffLineKind::Normal));

        if self.remotes.is_empty() {
            lines.push(DiffLine::new(
                "No remotes configured in .git/config",
                DiffLineKind::Context,
            ));
            lines.push(DiffLine::new(
                "Add a remote with: git remote add <name> <url>",
                DiffLineKind::Normal,
            ));
        } else {
            for (idx, r) in self.remotes.iter().enumerate() {
                let is_sel = idx == self.remotes_selected;
                let prefix = if is_sel { "> " } else { "  " };
                lines.push(DiffLine::new(
                    format!("{}{}", prefix, r.name),
                    if is_sel {
                        DiffLineKind::Addition
                    } else {
                        DiffLineKind::Header
                    },
                ));
                lines.push(DiffLine::new(
                    format!("    URL: {}", r.url),
                    DiffLineKind::Normal,
                ));
            }
        }

        let title = if let Some(r) = self.selected_remote() {
            format!("Remote: {}", r.name)
        } else {
            "Remotes Overview".to_string()
        };

        DiffView {
            title,
            lines,
            ..Default::default()
        }
    }

    fn compute_tags_view(&self, store: &RepoObjectStore) -> DiffView {
        let mut lines = Vec::new();
        let title = if let Some(tag) = self.selected_tag() {
            format!("Tag: {}", tag.name)
        } else {
            "Tags Inspector".to_string()
        };

        if let Some(tag) = self.selected_tag() {
            lines.push(DiffLine::new(
                format!("Tag:    {}", tag.name),
                DiffLineKind::Header,
            ));
            lines.push(DiffLine::new(
                format!("Commit: {}", tag.oid),
                DiffLineKind::Header,
            ));

            if let Ok(Object::Commit(c)) = store.read_object(&tag.oid) {
                lines.push(DiffLine::new(
                    format!("Author: {} <{}>", c.author.name, c.author.email),
                    DiffLineKind::Normal,
                ));
                lines.push(DiffLine::new("", DiffLineKind::Normal));
                for msg_line in c.message.lines() {
                    lines.push(DiffLine::new(
                        format!("    {}", msg_line),
                        DiffLineKind::Normal,
                    ));
                }
            }
        } else {
            lines.push(DiffLine::new(
                "No tags in repository",
                DiffLineKind::Context,
            ));
            lines.push(DiffLine::new(
                "Create a tag with: git tag <tagname>",
                DiffLineKind::Normal,
            ));
        }

        DiffView {
            title,
            lines,
            ..Default::default()
        }
    }

    fn compute_reflog_view(&self, store: &RepoObjectStore) -> DiffView {
        let mut lines = Vec::new();
        let title = if let Some(entry) = self.selected_reflog() {
            format!("Reflog: {}", entry.selector)
        } else {
            "Reflog Inspector".to_string()
        };

        if let Some(entry) = self.selected_reflog() {
            lines.push(DiffLine::new(
                format!("Reflog:  {}", entry.selector),
                DiffLineKind::Header,
            ));
            lines.push(DiffLine::new(
                format!("Action:  {}", entry.action),
                DiffLineKind::Header,
            ));
            lines.push(DiffLine::new(
                format!("Commit:  {}", entry.oid),
                DiffLineKind::Normal,
            ));
            lines.push(DiffLine::new(
                format!("Message: {}", entry.message),
                DiffLineKind::Normal,
            ));

            if let Ok(Object::Commit(c)) = store.read_object(&entry.oid) {
                lines.push(DiffLine::new("", DiffLineKind::Normal));
                lines.push(DiffLine::new(
                    format!("Author:  {} <{}>", c.author.name, c.author.email),
                    DiffLineKind::Normal,
                ));
                lines.push(DiffLine::new("", DiffLineKind::Normal));
                for msg_line in c.message.lines() {
                    lines.push(DiffLine::new(
                        format!("    {}", msg_line),
                        DiffLineKind::Normal,
                    ));
                }
            }
        } else {
            lines.push(DiffLine::new("Reflog is empty", DiffLineKind::Context));
        }

        DiffView {
            title,
            lines,
            ..Default::default()
        }
    }

    fn compute_file_diff(&self, store: &RepoObjectStore, file: &FileItem) -> DiffView {
        let index_path = self.git_dir.join("index");
        let index = Index::load_from(&index_path).unwrap_or_default();
        let ref_store = RefStore::with_common_dir(&self.git_dir, &self.common_dir);
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

        let (old_content, new_content) = match file.kind {
            FileStatusKind::StagedNew => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                (String::new(), index_blob)
            }
            FileStatusKind::StagedModified => {
                let head_blob = head_map
                    .get(&file.path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                (head_blob, index_blob)
            }
            FileStatusKind::StagedDeleted => {
                let head_blob = head_map
                    .get(&file.path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                (head_blob, String::new())
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
                (head_blob, index_blob)
            }
            FileStatusKind::UnstagedModified => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                let worktree = read_worktree_file(&self.repo_root, &file.path);
                (index_blob, worktree)
            }
            FileStatusKind::UnstagedDeleted => {
                let index_blob = index
                    .find_entry(&file.path)
                    .map(|e| read_blob_text(store, &e.oid))
                    .unwrap_or_default();
                (index_blob, String::new())
            }
            FileStatusKind::Untracked => {
                let worktree = read_worktree_file(&self.repo_root, &file.path);
                (String::new(), worktree)
            }
            FileStatusKind::Conflicted => {
                let worktree = read_worktree_file(&self.repo_root, &file.path);
                (String::new(), worktree)
            }
        };

        let old_path = file.old_path.as_deref().unwrap_or(&file.path);
        let diff_text = format_unified_diff(old_path, &file.path, &old_content, &new_content, 3);
        let hunks = oxidize_diff::compute_structured_diff(&old_content, &new_content, 3);

        if let Some(text) = diff_text {
            let mut view = DiffView::from_unified_text(title, &text);
            view.selected_hunk = if hunks.is_empty() { None } else { Some(0) };
            view.hunks = hunks;
            view.file_path = Some(file.path.clone());
            view.is_staged = file.kind.is_staged();
            view
        } else {
            let mut view = DiffView::new(title);
            view.lines.push(DiffLine::new(
                "Binary file or identical contents",
                DiffLineKind::Normal,
            ));
            view.file_path = Some(file.path.clone());
            view.is_staged = file.kind.is_staged();
            view
        }
    }

    pub fn compute_commit_diff(
        &mut self,
        store: &RepoObjectStore,
        commit: &CommitItem,
    ) -> DiffView {
        if let Some(cached) = self.commit_diff_cache.get(&commit.oid) {
            return cached.clone();
        }

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

            // 1. Changed files summary header and filter out unchanged files
            let mut changed_paths = Vec::new();
            for path in &all_paths {
                let p_entry = parent_map.get(*path);
                let c_entry = commit_map.get(*path);
                if p_entry != c_entry {
                    let status_char = match (p_entry.is_some(), c_entry.is_some()) {
                        (false, true) => "A",
                        (true, false) => "D",
                        (true, true) => "M",
                        _ => " ",
                    };
                    lines.push(DiffLine::new(
                        format!("  [{}] {}", status_char, path),
                        DiffLineKind::Header,
                    ));
                    changed_paths.push(*path);
                }
            }

            if !changed_paths.is_empty() {
                lines.push(DiffLine::new("", DiffLineKind::Normal));
            }

            // 2. Diff bodies: ONLY read blobs for files that actually changed!
            for path in changed_paths {
                let p_blob = parent_map
                    .get(path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();
                let c_blob = commit_map
                    .get(path)
                    .map(|(_, oid)| read_blob_text(store, oid))
                    .unwrap_or_default();

                if let Some(diff) = format_unified_diff(path, path, &p_blob, &c_blob, 3) {
                    for d_line in diff.lines() {
                        lines.push(DiffLine::from_raw_line(d_line));
                    }
                }
            }
        }

        let diff_view = DiffView {
            title,
            lines,
            ..Default::default()
        };

        self.commit_diff_cache.insert(commit.oid, diff_view.clone());
        diff_view
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
            format!(
                "Remote Ref:  {}",
                if branch.is_remote { "YES" } else { "NO" }
            ),
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
                    format!(
                        "Author:      {} <{}>",
                        commit.author.name, commit.author.email
                    ),
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
            lines.push(DiffLine::new(
                "No commits on this branch yet",
                DiffLineKind::Normal,
            ));
        }

        DiffView {
            title,
            lines,
            ..Default::default()
        }
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
                            if let Some(diff) = format_unified_diff(path, path, &p_blob, &s_blob, 3)
                            {
                                for d_line in diff.lines() {
                                    lines.push(DiffLine::from_raw_line(d_line));
                                }
                            }
                        }
                    }
                }
            }
        }

        DiffView {
            title,
            lines,
            ..Default::default()
        }
    }

    /// Verifies that no background task is actively mutating repository state.
    pub fn ensure_no_active_job(&mut self) -> Result<(), TuiError> {
        if let Some(ref job) = self.active_job {
            let msg = format!(
                "⚠ Cannot perform operation: '{}' is in progress",
                job.description
            );
            self.status_message = Some(msg);
            return Err(TuiError::Terminal(
                "Background operation in progress".to_string(),
            ));
        }
        Ok(())
    }

    /// Sets an error banner in status_message.
    pub fn set_error(&mut self, err: impl std::fmt::Display) {
        self.status_message = Some(format!("✗ {}", err));
    }

    /// Toggles staging for the currently selected file in the Files panel.
    pub fn toggle_stage_selected(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
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
        self.ensure_no_active_job()?;
        let has_unstaged_or_untracked = self.files.iter().any(|f| !f.kind.is_staged());
        if has_unstaged_or_untracked {
            let to_stage: Vec<&str> = self
                .files
                .iter()
                .filter(|f| !f.kind.is_staged())
                .map(|f| f.path.as_str())
                .collect();
            ops::stage_paths(&self.repo_root, &self.git_dir, &to_stage)?;
            self.status_message = Some("✓ Staged all changes".to_string());
        } else {
            let to_unstage: Vec<&str> = self
                .files
                .iter()
                .filter(|f| f.kind.is_staged())
                .map(|f| f.path.as_str())
                .collect();
            ops::unstage_paths(&self.repo_root, &self.git_dir, &to_unstage)?;
            self.status_message = Some("✓ Unstaged all changes".to_string());
        }
        self.refresh()?;
        Ok(())
    }

    /// Discards unstaged modifications to the currently selected file immediately.
    pub fn discard_selected_file(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
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

    /// Prompts for confirmation before discarding the selected file.
    pub fn prompt_discard_selected_file(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Files {
            return;
        }
        if let Some(file) = self.selected_file() {
            if !file.kind.is_staged() {
                let path = file.path.clone();
                self.active_modal = ActiveModal::Confirm {
                    title: " ⚠ Confirm Discard Changes ".to_string(),
                    prompt: format!(
                        "Discard all unstaged changes in '{}'? (This cannot be undone!)",
                        path
                    ),
                    action: crate::model::ConfirmAction::DiscardFile(path),
                };
            } else {
                self.status_message = Some(
                    "Cannot discard staged file directly: unstage it first (press Space)"
                        .to_string(),
                );
            }
        }
    }

    /// Discards a specific file by relative path.
    pub fn discard_file_by_path(&mut self, path: &str) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        ops::discard_path(&self.repo_root, &self.git_dir, path)?;
        self.status_message = Some(format!("✓ Discarded {}", path));
        self.refresh()?;
        Ok(())
    }

    /// Returns whether the currently displayed diff has any structured hunks.
    pub fn has_hunks(&self) -> bool {
        self.cached_diff
            .as_ref()
            .is_some_and(|d| !d.hunks.is_empty())
    }

    /// Index of the currently selected hunk, if any.
    pub fn selected_hunk(&self) -> Option<usize> {
        self.cached_diff.as_ref().and_then(|d| d.selected_hunk)
    }

    /// Advances to the next diff hunk and scrolls to make it visible.
    pub fn next_hunk(&mut self) {
        if let Some(ref mut diff) = self.cached_diff {
            if !diff.hunks.is_empty() {
                let next_idx = match diff.selected_hunk {
                    Some(cur) => (cur + 1).min(diff.hunks.len() - 1),
                    None => 0,
                };
                diff.selected_hunk = Some(next_idx);
            }
        }
        self.scroll_to_selected_hunk();
    }

    /// Moves to the previous diff hunk and scrolls to make it visible.
    pub fn prev_hunk(&mut self) {
        if let Some(ref mut diff) = self.cached_diff {
            if !diff.hunks.is_empty() {
                let prev_idx = match diff.selected_hunk {
                    Some(cur) => cur.saturating_sub(1),
                    None => 0,
                };
                diff.selected_hunk = Some(prev_idx);
            }
        }
        self.scroll_to_selected_hunk();
    }

    /// Selects a specific hunk index and scrolls to make it visible.
    pub fn select_hunk(&mut self, idx: usize) {
        if let Some(ref mut diff) = self.cached_diff {
            if idx < diff.hunks.len() {
                diff.selected_hunk = Some(idx);
            }
        }
        self.scroll_to_selected_hunk();
    }

    /// Scrolls inspector view so that the currently selected hunk is visible at or near the top.
    pub fn scroll_to_selected_hunk(&mut self) {
        if let Some(diff) = &self.cached_diff {
            if let Some(hunk_idx) = diff.selected_hunk {
                let starts = diff.hunk_start_lines();
                if let Some(&line_idx) = starts.get(hunk_idx) {
                    self.inspector_scroll = line_idx.saturating_sub(1);
                }
            }
        }
    }

    /// Toggles staging for the currently selected hunk in the Inspector.
    pub fn toggle_stage_selected_hunk(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        let (rel_path, hunk, is_staged) = match self.cached_diff.as_ref() {
            Some(diff) => match (diff.file_path.as_ref(), diff.selected_hunk) {
                (Some(path), Some(idx)) if idx < diff.hunks.len() => {
                    (path.clone(), diff.hunks[idx].clone(), diff.is_staged)
                }
                _ => return Ok(()),
            },
            None => return Ok(()),
        };

        if is_staged {
            ops::unstage_hunk(&self.repo_root, &self.git_dir, &rel_path, &hunk)?;
            self.status_message = Some(format!("✓ Unstaged hunk from {}", rel_path));
        } else {
            ops::stage_hunk(&self.repo_root, &self.git_dir, &rel_path, &hunk)?;
            self.status_message = Some(format!("✓ Staged hunk in {}", rel_path));
        }
        self.refresh()?;
        Ok(())
    }

    /// Prompts confirmation dialog to discard the currently selected hunk.
    pub fn prompt_discard_selected_hunk(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if let Some(diff) = &self.cached_diff {
            if diff.is_staged {
                self.status_message =
                    Some("Cannot discard staged hunk: unstage it first (press Space)".to_string());
                return;
            }
            if let (Some(path), Some(idx)) = (&diff.file_path, diff.selected_hunk) {
                if idx < diff.hunks.len() {
                    let path = path.clone();
                    self.active_modal = ActiveModal::Confirm {
                        title: " ⚠ Confirm Discard Hunk ".to_string(),
                        prompt: format!(
                            "Discard hunk {} of {} in '{}'? (This cannot be undone!)",
                            idx + 1,
                            diff.hunks.len(),
                            path
                        ),
                        action: crate::model::ConfirmAction::DiscardHunk {
                            path,
                            hunk_idx: idx,
                        },
                    };
                }
            }
        }
    }

    /// Discards a hunk by index.
    pub fn discard_hunk_by_index(&mut self, path: &str, hunk_idx: usize) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        let hunk_to_discard = if let Some(diff) = &self.cached_diff {
            if diff.file_path.as_deref() == Some(path) && hunk_idx < diff.hunks.len() {
                Some(diff.hunks[hunk_idx].clone())
            } else {
                None
            }
        } else {
            None
        };

        let hunk = match hunk_to_discard {
            Some(h) => h,
            None => {
                let store = RepoObjectStore::open(&self.git_dir)?;
                let index_path = self.git_dir.join("index");
                let index = if index_path.exists() {
                    Index::load_from(&index_path)
                        .map_err(|e| TuiError::Terminal(format!("Failed to load index: {}", e)))?
                } else {
                    Index::default()
                };
                let index_text = if let Some(e) = index.find_entry(path) {
                    read_blob_text(&store, &e.oid)
                } else {
                    String::new()
                };
                let worktree_full = self.repo_root.join(path);
                let worktree_text = fs::read_to_string(&worktree_full)?;
                let hunks = oxidize_diff::compute_structured_diff(&index_text, &worktree_text, 3);
                if hunk_idx >= hunks.len() {
                    return Err(TuiError::Terminal(format!(
                        "Hunk index {} out of range",
                        hunk_idx
                    )));
                }
                hunks[hunk_idx].clone()
            }
        };

        ops::discard_hunk(&self.repo_root, path, &hunk)?;
        self.status_message = Some(format!("✓ Discarded hunk {} in {}", hunk_idx + 1, path));
        self.refresh()?;
        Ok(())
    }

    /// Opens the commit modal if staged changes exist.
    pub fn open_commit_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        let has_staged = self.files.iter().any(|f| f.kind.is_staged());
        if has_staged {
            self.active_modal = ActiveModal::CommitPrompt {
                message: String::new(),
                cursor: 0,
            };
        } else {
            self.status_message =
                Some("Cannot commit: No files are staged (press Space to stage files)".to_string());
        }
    }

    /// Closes any currently active modal without applying.
    pub fn close_modal(&mut self) {
        self.active_modal = ActiveModal::None;
    }

    /// Appends or inserts a character at the modal cursor.
    pub fn handle_modal_char(&mut self, c: char) {
        let update_text = |text: &mut String, cursor: &mut usize| {
            let char_count = text.chars().count();
            let safe_cursor = (*cursor).min(char_count);
            let byte_offset = text
                .char_indices()
                .nth(safe_cursor)
                .map(|(i, _)| i)
                .unwrap_or(text.len());
            text.insert(byte_offset, c);
            *cursor = safe_cursor + 1;
        };

        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref mut message,
                ref mut cursor,
            }
            | ActiveModal::CommitAmend {
                ref mut message,
                ref mut cursor,
            } => {
                update_text(message, cursor);
            }
            ActiveModal::StashSave {
                ref mut message,
                ref mut cursor,
                focused_field: 0,
                ..
            } => {
                update_text(message, cursor);
            }
            ActiveModal::StashBranch {
                ref mut branch_name,
                ref mut cursor,
                ..
            } => {
                update_text(branch_name, cursor);
            }
            ActiveModal::WorktreeAdd {
                ref mut path,
                ref mut branch,
                focused_field,
                ref mut cursor,
                ..
            } => {
                if focused_field == 0 {
                    update_text(path, cursor);
                } else if focused_field == 1 {
                    update_text(branch, cursor);
                }
            }
            ActiveModal::BranchCreate {
                ref mut name,
                ref mut cursor,
            }
            | ActiveModal::TagCreate {
                ref mut name,
                ref mut cursor,
                ..
            } => {
                update_text(name, cursor);
            }
            ActiveModal::BranchRename {
                ref mut new_name,
                ref mut cursor,
                ..
            } => {
                update_text(new_name, cursor);
            }
            ActiveModal::SearchFilter {
                ref mut query,
                ref mut cursor,
            } => {
                update_text(query, cursor);
            }
            ActiveModal::RemoteAdd {
                ref mut name,
                ref mut url,
                focused_field,
                ref mut cursor,
            } => {
                if focused_field == 0 {
                    update_text(name, cursor);
                } else if focused_field == 1 {
                    update_text(url, cursor);
                }
            }
            ActiveModal::CommandPalette {
                ref mut query,
                ref mut cursor,
                ref mut selected,
                ref mut commands,
            } => {
                update_text(query, cursor);
                *selected = 0;
                let q = query.to_lowercase();
                *commands = PALETTE_COMMANDS
                    .iter()
                    .filter(|cmd| {
                        cmd.title.to_lowercase().contains(&q)
                            || cmd.category.to_lowercase().contains(&q)
                            || cmd.id.to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect();
            }
            _ => {}
        }
    }

    /// Handles Backspace key inside an active modal text field.
    pub fn handle_modal_backspace(&mut self) {
        let backspace_text = |text: &mut String, cursor: &mut usize| {
            let char_count = text.chars().count();
            let safe_cursor = (*cursor).min(char_count);
            if safe_cursor > 0 {
                let target_idx = safe_cursor - 1;
                if let Some((byte_offset, _)) = text.char_indices().nth(target_idx) {
                    text.remove(byte_offset);
                    *cursor = target_idx;
                }
            }
        };

        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref mut message,
                ref mut cursor,
            }
            | ActiveModal::CommitAmend {
                ref mut message,
                ref mut cursor,
            } => {
                backspace_text(message, cursor);
            }
            ActiveModal::StashSave {
                ref mut message,
                ref mut cursor,
                focused_field: 0,
                ..
            } => {
                backspace_text(message, cursor);
            }
            ActiveModal::StashBranch {
                ref mut branch_name,
                ref mut cursor,
                ..
            } => {
                backspace_text(branch_name, cursor);
            }
            ActiveModal::WorktreeAdd {
                ref mut path,
                ref mut branch,
                focused_field,
                ref mut cursor,
                ..
            } => {
                if focused_field == 0 {
                    backspace_text(path, cursor);
                } else if focused_field == 1 {
                    backspace_text(branch, cursor);
                }
            }
            ActiveModal::BranchCreate {
                ref mut name,
                ref mut cursor,
            }
            | ActiveModal::TagCreate {
                ref mut name,
                ref mut cursor,
                ..
            } => {
                backspace_text(name, cursor);
            }
            ActiveModal::BranchRename {
                ref mut new_name,
                ref mut cursor,
                ..
            } => {
                backspace_text(new_name, cursor);
            }
            ActiveModal::SearchFilter {
                ref mut query,
                ref mut cursor,
            } => {
                backspace_text(query, cursor);
            }
            ActiveModal::RemoteAdd {
                ref mut name,
                ref mut url,
                focused_field,
                ref mut cursor,
            } => {
                if focused_field == 0 {
                    backspace_text(name, cursor);
                } else if focused_field == 1 {
                    backspace_text(url, cursor);
                }
            }
            ActiveModal::CommandPalette {
                ref mut query,
                ref mut cursor,
                ref mut selected,
                ref mut commands,
            } => {
                backspace_text(query, cursor);
                *selected = 0;
                let q = query.to_lowercase();
                *commands = PALETTE_COMMANDS
                    .iter()
                    .filter(|cmd| {
                        cmd.title.to_lowercase().contains(&q)
                            || cmd.category.to_lowercase().contains(&q)
                            || cmd.id.to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect();
            }
            _ => {}
        }
    }

    /// Moves cursor one character left in the currently active text input modal.
    pub fn handle_modal_left(&mut self) {
        fn retreat(cursor: &mut usize) {
            *cursor = cursor.saturating_sub(1);
        }

        match self.active_modal {
            ActiveModal::CommitPrompt { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::CommitAmend { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::StashSave { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::StashBranch { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::WorktreeAdd { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::BranchCreate { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::TagCreate { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::BranchRename { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::SearchFilter { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::RemoteAdd { ref mut cursor, .. } => retreat(cursor),
            ActiveModal::CommandPalette { ref mut cursor, .. } => retreat(cursor),
            _ => {}
        }
    }

    /// Moves cursor one character right in the currently active text input modal.
    pub fn handle_modal_right(&mut self) {
        fn advance(text: &str, cursor: &mut usize) {
            let len = text.chars().count();
            if *cursor < len {
                *cursor += 1;
            }
        }

        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref message,
                ref mut cursor,
            } => {
                advance(message, cursor);
            }
            ActiveModal::CommitAmend {
                ref message,
                ref mut cursor,
            } => {
                advance(message, cursor);
            }
            ActiveModal::StashSave {
                ref message,
                ref mut cursor,
                focused_field: 0,
                ..
            } => {
                advance(message, cursor);
            }
            ActiveModal::StashBranch {
                ref branch_name,
                ref mut cursor,
                ..
            } => {
                advance(branch_name, cursor);
            }
            ActiveModal::WorktreeAdd {
                ref path,
                ref branch,
                focused_field,
                ref mut cursor,
                ..
            } => {
                if focused_field == 0 {
                    advance(path, cursor);
                } else if focused_field == 1 {
                    advance(branch, cursor);
                }
            }
            ActiveModal::BranchCreate {
                ref name,
                ref mut cursor,
            }
            | ActiveModal::TagCreate {
                ref name,
                ref mut cursor,
                ..
            } => {
                advance(name, cursor);
            }
            ActiveModal::BranchRename {
                ref new_name,
                ref mut cursor,
                ..
            } => {
                advance(new_name, cursor);
            }
            ActiveModal::SearchFilter {
                ref query,
                ref mut cursor,
            } => {
                advance(query, cursor);
            }
            ActiveModal::RemoteAdd {
                ref name,
                ref url,
                focused_field,
                ref mut cursor,
            } => {
                if focused_field == 0 {
                    advance(name, cursor);
                } else if focused_field == 1 {
                    advance(url, cursor);
                }
            }
            ActiveModal::CommandPalette {
                ref query,
                ref mut cursor,
                ..
            } => {
                advance(query, cursor);
            }
            _ => {}
        }
    }

    /// Sets the cursor position in an active modal text field, clamped to the character length.
    pub fn set_modal_cursor(&mut self, pos: usize) {
        match self.active_modal {
            ActiveModal::CommitPrompt {
                ref message,
                ref mut cursor,
            }
            | ActiveModal::CommitAmend {
                ref message,
                ref mut cursor,
            }
            | ActiveModal::StashSave {
                ref message,
                ref mut cursor,
                ..
            } => {
                *cursor = pos.min(message.chars().count());
            }
            ActiveModal::StashBranch {
                ref branch_name,
                ref mut cursor,
                ..
            } => {
                *cursor = pos.min(branch_name.chars().count());
            }
            ActiveModal::WorktreeAdd {
                ref path,
                ref branch,
                focused_field,
                ref mut cursor,
                ..
            } => {
                let target_len = if focused_field == 0 {
                    path.chars().count()
                } else {
                    branch.chars().count()
                };
                *cursor = pos.min(target_len);
            }
            ActiveModal::BranchCreate {
                ref name,
                ref mut cursor,
            }
            | ActiveModal::TagCreate {
                ref name,
                ref mut cursor,
                ..
            } => {
                *cursor = pos.min(name.chars().count());
            }
            ActiveModal::BranchRename {
                ref new_name,
                ref mut cursor,
                ..
            } => {
                *cursor = pos.min(new_name.chars().count());
            }
            ActiveModal::SearchFilter {
                ref query,
                ref mut cursor,
            } => {
                *cursor = pos.min(query.chars().count());
            }
            ActiveModal::RemoteAdd {
                ref name,
                ref url,
                focused_field,
                ref mut cursor,
            } => {
                let target_len = if focused_field == 0 {
                    name.chars().count()
                } else {
                    url.chars().count()
                };
                *cursor = pos.min(target_len);
            }
            ActiveModal::CommandPalette {
                ref query,
                ref mut cursor,
                ..
            } => {
                *cursor = pos.min(query.chars().count());
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

                match ops::create_commit(&self.repo_root, &self.git_dir, trimmed) {
                    Ok(commit_oid) => {
                        let short_sha = &commit_oid.to_string()[..7];
                        self.status_message = Some(format!(
                            "✓ [{} {}] {}",
                            self.branch_name, short_sha, trimmed
                        ));
                        self.active_modal = ActiveModal::None;
                        if let Err(e) = self.refresh() {
                            self.status_message = Some(format!(
                                "✓ [{} {}] {} (Warning: refresh failed: {})",
                                self.branch_name, short_sha, trimmed, e
                            ));
                        }
                        self.commits_selected = 0;
                        self.update_inspector();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("✗ Commit failed: {}", e));
                    }
                }
            }
            ActiveModal::CommitAmend { message, .. } => {
                let trimmed = message.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Amend aborted: empty commit message".to_string());
                    self.active_modal = ActiveModal::None;
                    return Ok(());
                }

                match ops::amend_commit(&self.repo_root, &self.git_dir, trimmed) {
                    Ok(commit_oid) => {
                        let short_sha = &commit_oid.to_string()[..7];
                        self.status_message =
                            Some(format!("✓ Amended commit [{}] {}", short_sha, trimmed));
                        self.active_modal = ActiveModal::None;
                        if let Err(e) = self.refresh() {
                            self.status_message = Some(format!(
                                "✓ Amended commit [{}] {} (Warning: refresh failed: {})",
                                short_sha, trimmed, e
                            ));
                        }
                        self.commits_selected = 0;
                        self.update_inspector();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("✗ Amend failed: {}", e));
                    }
                }
            }
            ActiveModal::BranchCreate { name, .. } => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    self.status_message =
                        Some("Branch creation aborted: empty branch name".to_string());
                    self.active_modal = ActiveModal::None;
                    return Ok(());
                }

                match ops::create_and_checkout_branch(&self.repo_root, &self.git_dir, trimmed) {
                    Ok(()) => {
                        self.status_message =
                            Some(format!("✓ Created and switched to branch '{}'", trimmed));
                        self.active_modal = ActiveModal::None;
                        if let Err(e) = self.refresh() {
                            self.status_message = Some(format!(
                                "✓ Created and switched to branch '{}' (Warning: refresh failed: {})",
                                trimmed, e
                            ));
                        }
                    }
                    Err(e) => {
                        self.status_message = Some(format!("✗ Branch creation failed: {}", e));
                    }
                }
            }
            ActiveModal::StashSave {
                message,
                include_untracked,
                staged_only,
                keep_index,
                ..
            } => {
                let opts = ops::StashSaveOptions {
                    message: message.trim().to_string(),
                    include_untracked,
                    staged_only,
                    keep_index,
                };
                match ops::stash_save_with_options(&self.repo_root, &self.git_dir, &opts) {
                    Ok(stash_oid) => {
                        let short_sha = &stash_oid.to_string()[..7];
                        self.status_message = Some(format!("✓ Saved stash [{}]", short_sha));
                        self.active_modal = ActiveModal::None;
                        if let Err(e) = self.refresh() {
                            self.status_message = Some(format!(
                                "✓ Saved stash [{}] (Warning: refresh failed: {})",
                                short_sha, e
                            ));
                        }
                    }
                    Err(e) => {
                        self.status_message = Some(format!("✗ Stash save failed: {}", e));
                    }
                }
            }
            ActiveModal::StashBranch {
                stash_idx,
                branch_name,
                ..
            } => {
                let trimmed = branch_name.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Branch name cannot be empty".to_string());
                } else {
                    match ops::stash_branch(&self.repo_root, &self.git_dir, stash_idx, trimmed) {
                        Ok(()) => {
                            self.status_message = Some(format!(
                                "✓ Created and switched to branch '{}' from stash@{{{}}}",
                                trimmed, stash_idx
                            ));
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message =
                                Some(format!("✗ Stash branch creation failed: {}", e));
                        }
                    }
                }
            }
            ActiveModal::CustomPatchMenu { selected } => match selected {
                0 => {
                    match ops::apply_custom_patch_to_worktree(
                        &self.repo_root,
                        &self.custom_patch_basket,
                        false,
                    ) {
                        Ok(()) => {
                            self.status_message =
                                Some("✓ Applied custom patch to worktree".to_string());
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message = Some(format!("✗ Apply patch failed: {}", e))
                        }
                    }
                }
                1 => {
                    match ops::apply_custom_patch_to_index(
                        &self.repo_root,
                        &self.git_dir,
                        &self.custom_patch_basket,
                        false,
                    ) {
                        Ok(()) => {
                            self.status_message =
                                Some("✓ Applied custom patch to index".to_string());
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message =
                                Some(format!("✗ Apply patch to index failed: {}", e))
                        }
                    }
                }
                2 => {
                    match ops::apply_custom_patch_to_worktree(
                        &self.repo_root,
                        &self.custom_patch_basket,
                        true,
                    ) {
                        Ok(()) => {
                            self.status_message =
                                Some("✓ Reverted custom patch in worktree".to_string());
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message = Some(format!("✗ Revert patch failed: {}", e))
                        }
                    }
                }
                3 => {
                    match ops::apply_custom_patch_to_index(
                        &self.repo_root,
                        &self.git_dir,
                        &self.custom_patch_basket,
                        true,
                    ) {
                        Ok(()) => {
                            self.status_message =
                                Some("✓ Reverted custom patch in index".to_string());
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message =
                                Some(format!("✗ Revert patch in index failed: {}", e))
                        }
                    }
                }
                4 => {
                    let msg = format!("custom patch: {} hunks", self.custom_patch_basket.len());
                    match ops::create_commit_from_custom_patch(
                        &self.repo_root,
                        &self.git_dir,
                        &self.custom_patch_basket,
                        &msg,
                    ) {
                        Ok(oid) => {
                            self.status_message =
                                Some(format!("✓ Committed custom patch [{:.7}]", oid));
                            self.custom_patch_basket.clear();
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message =
                                Some(format!("✗ Commit custom patch failed: {}", e))
                        }
                    }
                }
                5 => {
                    self.custom_patch_basket.clear();
                    self.status_message = Some("✓ Cleared custom patch basket".to_string());
                    self.active_modal = ActiveModal::None;
                }
                _ => {
                    self.active_modal = ActiveModal::None;
                }
            },
            ActiveModal::WorktreeList { items, selected } => {
                if let Some(item) = items.get(selected) {
                    let path = item.path.clone();
                    self.active_modal = ActiveModal::None;
                    if let Err(e) = self.switch_worktree(&path) {
                        self.status_message = Some(format!("✗ Failed to switch worktree: {}", e));
                    }
                }
            }
            ActiveModal::WorktreeAdd {
                path,
                branch,
                create_branch,
                ..
            } => {
                let p_trim = path.trim();
                let b_trim = branch.trim();
                if p_trim.is_empty() {
                    self.status_message = Some("Worktree path cannot be empty".to_string());
                } else if b_trim.is_empty() {
                    self.status_message = Some("Branch name cannot be empty".to_string());
                } else {
                    let wt_path = PathBuf::from(p_trim);
                    match ops::create_worktree(
                        &self.repo_root,
                        &self.git_dir,
                        &self.common_dir,
                        &wt_path,
                        b_trim,
                        create_branch,
                    ) {
                        Ok(item) => {
                            self.status_message = Some(format!(
                                "✓ Created worktree '{}' at '{}'",
                                item.name,
                                item.path.display()
                            ));
                            self.active_modal = ActiveModal::None;
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message =
                                Some(format!("✗ Failed to create worktree: {}", e));
                        }
                    }
                }
            }
            ActiveModal::BranchRename {
                old_name, new_name, ..
            } => {
                let trimmed = new_name.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Branch name cannot be empty".to_string());
                } else {
                    match ops::rename_branch(
                        &self.repo_root,
                        &self.git_dir,
                        &self.common_dir,
                        &old_name,
                        trimmed,
                    ) {
                        Ok(()) => {
                            self.status_message =
                                Some(format!("✓ Renamed branch '{}' to '{}'", old_name, trimmed));
                            self.active_modal = ActiveModal::None;
                            if let Err(e) = self.refresh() {
                                self.status_message = Some(format!(
                                    "✓ Renamed branch '{}' to '{}' (Warning: refresh failed: {})",
                                    old_name, trimmed, e
                                ));
                            }
                        }
                        Err(e) => {
                            self.status_message = Some(format!("✗ Branch rename failed: {}", e));
                        }
                    }
                }
            }
            ActiveModal::TagCreate {
                target_oid, name, ..
            } => {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    self.status_message = Some("Tag name cannot be empty".to_string());
                } else {
                    match ops::create_tag(&self.git_dir, &self.common_dir, trimmed, &target_oid) {
                        Ok(()) => {
                            self.status_message = Some(format!("✓ Created tag '{}'", trimmed));
                            self.active_modal = ActiveModal::None;
                            if let Err(e) = self.refresh() {
                                self.status_message = Some(format!(
                                    "✓ Created tag '{}' (Warning: refresh failed: {})",
                                    trimmed, e
                                ));
                            }
                        }
                        Err(e) => {
                            self.status_message = Some(format!("✗ Tag creation failed: {}", e));
                        }
                    }
                }
            }
            ActiveModal::SearchFilter { query, .. } => {
                let trimmed = query.trim();
                self.commit_search_filter = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_lowercase())
                };
                self.active_modal = ActiveModal::None;
                let indices = self.filtered_commit_indices();
                if let Some(&first) = indices.first() {
                    self.commits_selected = first;
                } else {
                    self.commits_selected = 0;
                }
                self.selected_index = self.commits_selected;
                self.inspector_scroll = 0;
                self.update_inspector();
            }
            ActiveModal::Confirm { action, .. } => {
                self.active_modal = ActiveModal::None;
                match action {
                    ConfirmAction::DiscardFile(path) => {
                        self.discard_file_by_path(&path)?;
                    }
                    ConfirmAction::DeleteBranch(name) => {
                        self.delete_branch_by_name(&name)?;
                    }
                    ConfirmAction::DropStash(idx) => {
                        self.drop_stash_by_index(idx)?;
                    }
                    ConfirmAction::DiscardHunk { path, hunk_idx } => {
                        self.discard_hunk_by_index(&path, hunk_idx)?;
                    }
                    ConfirmAction::ResetToCommit {
                        target_oid,
                        short_oid,
                        mode,
                    } => {
                        match ops::reset_to_commit(
                            &self.repo_root,
                            &self.git_dir,
                            &self.common_dir,
                            &target_oid,
                            mode,
                        ) {
                            Ok(()) => {
                                self.status_message =
                                    Some(format!("✓ Reset ({:?}) to {}", mode, short_oid));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message = Some(format!("✗ Reset failed: {}", e));
                            }
                        }
                    }
                    ConfirmAction::CherryPick(commit_id) => {
                        match ops::cherry_pick_commit(
                            &self.repo_root,
                            &self.git_dir,
                            &self.common_dir,
                            &commit_id,
                        ) {
                            Ok(new_oid) => {
                                self.status_message = Some(format!(
                                    "✓ Cherry-picked commit {}",
                                    &new_oid.to_string()[..7]
                                ));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message = Some(format!("✗ Cherry-pick failed: {}", e));
                            }
                        }
                    }
                    ConfirmAction::DeleteTag(tag_name) => {
                        match ops::delete_tag(&self.git_dir, &self.common_dir, &tag_name) {
                            Ok(()) => {
                                self.status_message = Some(format!("✓ Deleted tag '{}'", tag_name));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to delete tag: {}", e));
                            }
                        }
                    }
                    ConfirmAction::Revert(commit_oid) => {
                        match ops::revert_commit(
                            &self.repo_root,
                            &self.git_dir,
                            &self.common_dir,
                            &commit_oid,
                        ) {
                            Ok(new_oid) => {
                                self.status_message = Some(format!(
                                    "✓ Reverted commit {}",
                                    &new_oid.to_string()[..7]
                                ));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message = Some(format!("✗ Revert failed: {}", e));
                            }
                        }
                    }
                    ConfirmAction::RebaseAbort => {
                        match ops::rebase_abort(&self.repo_root, &self.git_dir, &self.common_dir) {
                            Ok(()) => {
                                self.status_message =
                                    Some("✓ Rebase aborted; restored original HEAD".to_string());
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to abort rebase: {}", e));
                            }
                        }
                    }
                    ConfirmAction::RemoveWorktree { name, force } => {
                        match ops::remove_worktree(&self.common_dir, &name, force) {
                            Ok(()) => {
                                self.status_message =
                                    Some(format!("✓ Removed worktree '{}'", name));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to remove worktree: {}", e));
                            }
                        }
                    }
                    ConfirmAction::DeleteRemote(name) => {
                        match ops::remove_remote(&self.git_dir, &name) {
                            Ok(()) => {
                                self.status_message = Some(format!("✓ Removed remote '{}'", name));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to remove remote: {}", e));
                            }
                        }
                    }
                    ConfirmAction::DeleteRemoteBranch { remote, branch } => {
                        match ops::delete_remote_branch(&self.git_dir, Some(&remote), &branch) {
                            Ok(msg) => {
                                self.status_message = Some(format!("✓ {}", msg));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to delete remote branch: {}", e));
                            }
                        }
                    }
                }
            }
            ActiveModal::RebaseTodo {
                items, onto_oid, ..
            } => {
                self.active_modal = ActiveModal::None;
                match ops::start_interactive_rebase(
                    &self.repo_root,
                    &self.git_dir,
                    &self.common_dir,
                    &onto_oid,
                    items,
                ) {
                    Ok(ops::ReplayStepOutcome::Finished) => {
                        self.status_message = Some("✓ Rebase finished successfully".to_string());
                    }
                    Ok(ops::ReplayStepOutcome::Conflict { conflicted_paths }) => {
                        self.status_message = Some(format!(
                            "✗ Rebase conflict in: {}. Resolve conflicts and continue.",
                            conflicted_paths.join(", ")
                        ));
                    }
                    Ok(ops::ReplayStepOutcome::StoppedForEditing { commit_oid }) => {
                        let short = if commit_oid.to_string().len() >= 7 {
                            &commit_oid.to_string()[..7]
                        } else {
                            &commit_oid.to_string()
                        };
                        self.status_message = Some(format!(
                            "Stopped at {} for editing. Make changes and continue.",
                            short
                        ));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("✗ Rebase failed: {}", e));
                    }
                }
                let _ = self.refresh();
            }
            ActiveModal::RemoteAdd { name, url, .. } => {
                let name = name.trim().to_string();
                let url = url.trim().to_string();
                self.active_modal = ActiveModal::None;
                if name.is_empty() || url.is_empty() {
                    self.status_message = Some("✗ Remote name and URL cannot be empty".to_string());
                } else {
                    match ops::add_remote(&self.git_dir, &name, &url) {
                        Ok(()) => {
                            self.status_message = Some(format!("✓ Added remote '{}'", name));
                            let _ = self.refresh();
                        }
                        Err(e) => {
                            self.status_message = Some(format!("✗ Failed to add remote: {}", e));
                        }
                    }
                }
            }
            ActiveModal::SubmoduleList { items, selected } => {
                self.active_modal = ActiveModal::None;
                if let Some(sub) = items.get(selected) {
                    if sub.is_initialized {
                        if let Err(e) = self.enter_submodule(&sub.path) {
                            self.status_message = Some(format!("✗ {}", e));
                        }
                    } else {
                        match ops::submodule_init(&self.repo_root, &self.git_dir, &sub.name) {
                            Ok(()) => {
                                self.status_message =
                                    Some(format!("✓ Initialized submodule '{}'", sub.name));
                                let _ = self.refresh();
                            }
                            Err(e) => {
                                self.status_message =
                                    Some(format!("✗ Failed to initialize submodule: {}", e));
                            }
                        }
                    }
                }
            }
            ActiveModal::BisectMenu { state, selected } => {
                self.active_modal = ActiveModal::None;
                if state.is_active {
                    match selected {
                        0 => self.bisect_mark_bad(),
                        1 => self.bisect_mark_good(),
                        2 => self.bisect_skip(),
                        3 => self.bisect_reset(),
                        _ => {}
                    }
                } else {
                    match selected {
                        0 => self.bisect_start(),
                        1 => self.bisect_mark_good(),
                        _ => {}
                    }
                }
            }
            ActiveModal::CommandPalette {
                commands, selected, ..
            } => {
                self.active_modal = ActiveModal::None;
                if let Some(item) = commands.get(selected) {
                    let cmd_id = item.id;
                    self.execute_palette_command(cmd_id);
                }
            }
            ActiveModal::ProviderLinks { urls, selected } => {
                self.active_modal = ActiveModal::None;
                let list = [
                    ("Repository", urls.repo_url.as_deref()),
                    ("Commit", urls.commit_url.as_deref()),
                    ("Branch", urls.branch_url.as_deref()),
                    ("Pull Request", urls.pr_url.as_deref()),
                ];
                let available: Vec<(&str, &str)> = list
                    .iter()
                    .filter_map(|(l, u)| u.map(|url| (*l, url)))
                    .collect();
                if let Some((label, url)) = available.get(selected) {
                    self.status_message = Some(format!("Copied {} URL: {}", label, url));
                }
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

    /// Opens the amend commit modal dialog.
    pub fn open_amend_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if let Some(last_commit) = self.commits.first() {
            let msg = last_commit.full_message.trim().to_string();
            let len = msg.chars().count();
            self.active_modal = ActiveModal::CommitAmend {
                message: msg,
                cursor: len,
            };
        } else {
            self.status_message = Some("Cannot amend: repository has no commits".to_string());
        }
    }

    /// Opens the save stash modal dialog.
    pub fn open_stash_save_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::StashSave {
            message: String::new(),
            cursor: 0,
            include_untracked: false,
            staged_only: false,
            keep_index: false,
            focused_field: 0,
        };
    }

    /// Opens the stash branch modal dialog for the currently selected stash.
    pub fn open_stash_branch_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Stash {
            return;
        }
        if let Some(stash) = self.selected_stash() {
            let stash_idx = stash.index;
            let default_branch = format!("stash-{}", stash_idx);
            let len = default_branch.chars().count();
            self.active_modal = ActiveModal::StashBranch {
                stash_idx,
                branch_name: default_branch,
                cursor: len,
            };
        }
    }

    /// Opens the worktree list modal.
    pub fn open_worktree_list_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        let items = ops::list_worktrees(&self.git_dir, &self.common_dir, Some(&self.repo_root))
            .unwrap_or_default();
        self.active_modal = ActiveModal::WorktreeList { items, selected: 0 };
    }

    /// Opens the add worktree modal dialog.
    pub fn open_worktree_add_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::WorktreeAdd {
            path: String::new(),
            branch: String::new(),
            create_branch: true,
            focused_field: 0,
            cursor: 0,
        };
    }

    /// Prompts confirmation dialog to remove a linked worktree.
    pub fn prompt_remove_worktree(&mut self, name: String, force: bool) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::Confirm {
            title: " ⚠ Confirm Remove Worktree ".to_string(),
            prompt: format!("Are you sure you want to remove linked worktree '{}'?\nWorking tree directory and metadata will be deleted.", name),
            action: ConfirmAction::RemoveWorktree { name, force },
        };
    }

    /// Opens the custom patch menu dialog.
    pub fn open_custom_patch_menu(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::CustomPatchMenu { selected: 0 };
    }

    /// Switches the active repository context to the specified linked worktree directory.
    pub fn switch_worktree(&mut self, target_path: &Path) -> Result<(), TuiError> {
        let dot_git = target_path.join(".git");
        let git_dir = if dot_git.exists() {
            dot_git
        } else {
            target_path.to_path_buf()
        };
        self.load_repository(&git_dir)?;
        self.status_message = Some(format!(
            "✓ Switched to worktree at '{}'",
            target_path.display()
        ));
        Ok(())
    }

    /// Toggles the currently selected hunk in the custom patch basket.
    pub fn toggle_current_hunk_in_patch_basket(&mut self) {
        let (path, hunk) = match self.cached_diff.as_ref() {
            Some(diff) => match (diff.file_path.as_ref(), diff.selected_hunk) {
                (Some(p), Some(idx)) if idx < diff.hunks.len() => {
                    (p.clone(), diff.hunks[idx].clone())
                }
                _ => {
                    self.status_message =
                        Some("No hunk selected to add to patch basket".to_string());
                    return;
                }
            },
            None => {
                self.status_message = Some("No hunk selected to add to patch basket".to_string());
                return;
            }
        };

        if self.custom_patch_basket.contains_hunk(&path, &hunk) {
            self.custom_patch_basket.remove_matching_hunk(&path, &hunk);
            self.status_message = Some(format!(
                "✓ Removed hunk from patch basket ({} hunks total)",
                self.custom_patch_basket.len()
            ));
        } else {
            self.custom_patch_basket.add_hunk(&path, hunk);
            self.status_message = Some(format!(
                "✓ Added hunk to patch basket ({} hunks total)",
                self.custom_patch_basket.len()
            ));
        }
    }

    /// Clears all hunks from the custom patch basket.
    pub fn clear_patch_basket(&mut self) {
        let count = self.custom_patch_basket.len();
        self.custom_patch_basket.clear();
        self.status_message = Some(format!("✓ Cleared patch basket (removed {} hunks)", count));
    }

    /// Applies the selected stash entry without dropping it.
    pub fn apply_selected_stash(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Stash {
            return Ok(());
        }
        if let Some(stash) = self.selected_stash().cloned() {
            let clean = ops::apply_stash(&self.repo_root, &self.git_dir, &stash.oid)?;
            if clean {
                self.status_message = Some(format!("✓ Applied stash@{{{}}}", stash.index));
            } else {
                self.status_message = Some(format!(
                    "⚠ Applied stash@{{{}}} with conflicts (staged in index)",
                    stash.index
                ));
            }
            if let Err(e) = self.refresh() {
                self.status_message = Some(format!(
                    "{} (Warning: refresh failed: {})",
                    self.status_message.as_deref().unwrap_or(""),
                    e
                ));
            }
        }
        Ok(())
    }

    /// Checks out the currently selected branch in the Branches panel.
    pub fn checkout_selected_branch(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Branches || self.branches_tab != BranchesTab::Local {
            return Ok(());
        }
        if let Some(b) = self.selected_branch().cloned() {
            if b.is_head {
                self.status_message = Some(format!("Already on branch '{}'", b.name));
                return Ok(());
            }
            if b.is_remote {
                self.status_message = Some(format!(
                    "Cannot directly checkout remote branch '{}'",
                    b.name
                ));
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
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::BranchCreate {
            name: String::new(),
            cursor: 0,
        };
    }

    /// Prompts for confirmation before deleting the selected local branch.
    pub fn prompt_delete_selected_branch(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Branches || self.branches_tab != BranchesTab::Local {
            return;
        }
        if let Some(b) = self.selected_branch().cloned() {
            if b.is_head {
                self.status_message =
                    Some(format!("Cannot delete checked-out branch '{}'", b.name));
                return;
            }
            if b.is_remote {
                self.status_message =
                    Some(format!("Cannot delete remote tracking branch '{}'", b.name));
                return;
            }
            let name = b.name.clone();
            self.active_modal = ActiveModal::Confirm {
                title: " ⚠ Confirm Delete Branch ".to_string(),
                prompt: format!("Are you sure you want to delete branch '{}'?", name),
                action: crate::model::ConfirmAction::DeleteBranch(name),
            };
        }
    }

    /// Deletes a specific branch by name.
    pub fn delete_branch_by_name(&mut self, name: &str) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        match ops::delete_branch(&self.git_dir, name, false) {
            Ok(()) => {
                self.status_message = Some(format!("✓ Deleted branch '{}'", name));
                self.refresh()?;
            }
            Err(e) => {
                self.status_message = Some(format!("✗ Cannot delete branch: {}", e));
            }
        }
        Ok(())
    }

    /// Deletes the currently selected branch in the Branches panel immediately.
    pub fn delete_selected_branch(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Branches || self.branches_tab != BranchesTab::Local {
            return Ok(());
        }
        if let Some(b) = self.selected_branch().cloned() {
            if b.is_head {
                self.status_message =
                    Some(format!("Cannot delete checked-out branch '{}'", b.name));
                return Ok(());
            }
            if b.is_remote {
                self.status_message =
                    Some(format!("Cannot delete remote tracking branch '{}'", b.name));
                return Ok(());
            }

            match ops::delete_branch(&self.git_dir, &b.name, false) {
                Ok(()) => {
                    self.status_message = Some(format!("✓ Deleted branch '{}'", b.name));
                    self.refresh()?;
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Cannot delete branch: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Pops the selected stash into the working tree.
    pub fn pop_selected_stash(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Stash {
            return Ok(());
        }
        if let Some(s) = self.selected_stash().cloned() {
            let clean = ops::pop_stash(&self.repo_root, &self.git_dir, s.index, &s.oid)?;
            if clean {
                self.status_message = Some(format!("✓ Popped stash@{{{}}}", s.index));
            } else {
                self.status_message = Some(format!(
                    "⚠ Conflict popping stash@{{{}}}, stash kept",
                    s.index
                ));
            }
            self.refresh()?;
        }
        Ok(())
    }

    /// Prompts for confirmation before dropping the selected stash.
    pub fn prompt_drop_selected_stash(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Stash {
            return;
        }
        if let Some(s) = self.selected_stash() {
            let idx = s.index;
            let msg = s.message.clone();
            self.active_modal = ActiveModal::Confirm {
                title: " ⚠ Confirm Drop Stash ".to_string(),
                prompt: format!(
                    "Drop stash@{{{}}} (\"{}\")? (This cannot be undone!)",
                    idx, msg
                ),
                action: crate::model::ConfirmAction::DropStash(idx),
            };
        }
    }

    /// Drops a stash entry by numeric index.
    pub fn drop_stash_by_index(&mut self, index: usize) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        ops::drop_stash(&self.git_dir, index)?;
        self.status_message = Some(format!("✓ Dropped stash@{{{}}}", index));
        self.refresh()?;
        Ok(())
    }

    /// Drops the stash stack item immediately.
    pub fn drop_selected_stash(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Stash {
            return Ok(());
        }
        if let Some(s) = self.selected_stash().cloned() {
            ops::drop_stash(&self.git_dir, s.index)?;
            self.status_message = Some(format!("✓ Dropped stash@{{{}}}", s.index));
            self.refresh()?;
        }
        Ok(())
    }

    /// Checks out the selected commit in detached HEAD state.
    pub fn checkout_selected_commit(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Commits {
            return Ok(());
        }
        if let Some(c) = self.selected_commit().cloned() {
            ops::checkout_commit(&self.repo_root, &self.git_dir, &self.common_dir, &c.oid)?;
            self.status_message = Some(format!("✓ Checked out commit {}", &c.oid.to_string()[..7]));
            self.refresh()?;
        }
        Ok(())
    }

    /// Prompts for confirmation before cherry-picking the selected commit.
    pub fn prompt_cherry_pick_selected_commit(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Commits {
            return;
        }
        if let Some(c) = self.selected_commit().cloned() {
            self.active_modal = ActiveModal::Confirm {
                title: " 🍒 Confirm Cherry-Pick ".to_string(),
                prompt: format!(
                    "Cherry-pick commit {} (\"{}\") onto current branch?",
                    &c.oid.to_string()[..7],
                    c.summary
                ),
                action: crate::model::ConfirmAction::CherryPick(c.oid),
            };
        }
    }

    /// Prompts for confirmation before resetting HEAD to the selected commit with the given mode.
    pub fn prompt_reset_selected_commit(&mut self, mode: ResetMode) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Commits {
            return;
        }
        if let Some(c) = self.selected_commit().cloned() {
            let mode_str = match mode {
                ResetMode::Soft => "soft (keep index & working tree)",
                ResetMode::Mixed => "mixed (reset index, keep working tree)",
                ResetMode::Hard => "hard (DISCARD index and working tree changes!)",
            };
            self.active_modal = ActiveModal::Confirm {
                title: format!(" ⚠ Confirm Reset ({:?}) ", mode),
                prompt: format!(
                    "Reset HEAD to {} using {}?",
                    &c.oid.to_string()[..7],
                    mode_str
                ),
                action: crate::model::ConfirmAction::ResetToCommit {
                    target_oid: c.oid,
                    short_oid: c.oid.to_string()[..7].to_string(),
                    mode,
                },
            };
        }
    }

    /// Prompts to rename the currently selected local branch.
    pub fn prompt_rename_selected_branch(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Branches || self.branches_tab != BranchesTab::Local {
            return;
        }
        if let Some(b) = self.selected_branch().cloned() {
            let cur_len = b.name.chars().count();
            self.active_modal = ActiveModal::BranchRename {
                old_name: b.name.clone(),
                new_name: b.name,
                cursor: cur_len,
            };
        }
    }

    /// Fast-forwards the current HEAD to the selected local or remote branch.
    pub fn fast_forward_selected_branch(&mut self) -> Result<(), TuiError> {
        self.ensure_no_active_job()?;
        if self.active_panel != Panel::Branches {
            return Ok(());
        }
        if let Some(b) = self.selected_branch().cloned() {
            match ops::fast_forward_merge(&self.repo_root, &self.git_dir, &self.common_dir, &b.name)
            {
                Ok(()) => {
                    self.status_message = Some(format!("✓ Fast-forwarded to {}", b.name));
                    self.refresh()?;
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Fast-forward failed: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Opens modal prompt to create a lightweight tag at the selected commit or HEAD.
    pub fn prompt_create_tag(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        let target_commit = match self.active_panel {
            Panel::Commits => self.selected_commit().map(|c| c.oid),
            _ => self.commits.first().map(|c| c.oid),
        };
        if let Some(oid) = target_commit {
            self.active_modal = ActiveModal::TagCreate {
                target_oid: oid,
                name: String::new(),
                cursor: 0,
            };
        } else {
            self.status_message = Some("Cannot create tag: no commits available".to_string());
        }
    }

    /// Prompts for confirmation before deleting the selected tag.
    pub fn prompt_delete_selected_tag(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel != Panel::Branches || self.branches_tab != BranchesTab::Tags {
            return;
        }
        if let Some(t) = self.selected_tag().cloned() {
            let name = t.name.clone();
            self.active_modal = ActiveModal::Confirm {
                title: " ⚠ Confirm Delete Tag ".to_string(),
                prompt: format!("Are you sure you want to delete tag '{}'?", name),
                action: crate::model::ConfirmAction::DeleteTag(name),
            };
        }
    }

    /// Opens the commit search/filter modal dialog.
    pub fn open_search_filter_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        let cur = self.commit_search_filter.clone().unwrap_or_default();
        let len = cur.chars().count();
        self.active_modal = ActiveModal::SearchFilter {
            query: cur,
            cursor: len,
        };
    }

    /// Clears the active commit search filter.
    pub fn clear_search_filter(&mut self) {
        self.commit_search_filter = None;
        let indices = self.filtered_commit_indices();
        if let Some(&first) = indices.first() {
            self.commits_selected = first;
        } else {
            self.commits_selected = 0;
        }
        self.selected_index = self.commits_selected;
        self.inspector_scroll = 0;
        self.update_inspector();
    }

    /// Pushes commits to remote repository in a non-blocking background task (bound to 'P').
    pub fn push(&mut self) -> Result<(), TuiError> {
        if self.active_job.is_some() {
            self.status_message =
                Some("⚠ A repository operation is already in progress".to_string());
            return Ok(());
        }

        let repo_root = self.repo_root.clone();
        let git_dir = self.git_dir.clone();
        let branch = self.branch_name.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        self.status_message = Some(format!("Pushing branch '{}' to remote...", branch));

        let branch_for_thread = branch.clone();
        std::thread::spawn(move || {
            let res =
                ops::push_to_remote(&repo_root, &git_dir, None, Some(&branch_for_thread), false);
            let job_res = match res {
                Ok(msg) => BackgroundJobResult::Success(msg),
                Err(e) => BackgroundJobResult::Error(e.to_string()),
            };
            let _ = tx.send(job_res);
        });

        self.active_job = Some(BackgroundJob {
            description: format!("Push {}", branch),
            receiver: rx,
        });
        Ok(())
    }

    /// Pulls latest commits from remote repository in a non-blocking background task (bound to 'p').
    pub fn pull(&mut self) -> Result<(), TuiError> {
        if self.active_job.is_some() {
            self.status_message =
                Some("⚠ A repository operation is already in progress".to_string());
            return Ok(());
        }

        let repo_root = self.repo_root.clone();
        let git_dir = self.git_dir.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        self.status_message = Some("Pulling from remote...".to_string());

        std::thread::spawn(move || {
            let res = ops::pull_from_remote(&repo_root, &git_dir, None, None);
            let job_res = match res {
                Ok(msg) => BackgroundJobResult::Success(msg),
                Err(e) => BackgroundJobResult::Error(e.to_string()),
            };
            let _ = tx.send(job_res);
        });

        self.active_job = Some(BackgroundJob {
            description: "Pull".to_string(),
            receiver: rx,
        });
        Ok(())
    }

    /// Ticks the background event loop, processing any completed asynchronous jobs.
    pub fn tick(&mut self) -> Result<(), TuiError> {
        if let Some(ref job) = self.active_job {
            match job.receiver.try_recv() {
                Ok(BackgroundJobResult::Success(msg)) => {
                    self.status_message = Some(format!("✓ {}", msg));
                    self.active_job = None;
                    let _ = self.refresh();
                }
                Ok(BackgroundJobResult::Error(err)) => {
                    self.status_message = Some(format!("✗ {}", err));
                    self.active_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Job running in background
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.status_message =
                        Some("✗ Background job terminated unexpectedly".to_string());
                    self.active_job = None;
                }
            }
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

    /// Opens the interactive rebase todo modal for the selected commit.
    pub fn open_rebase_todo_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.commits.is_empty() {
            return;
        }
        let sel_idx = self.commits_selected;
        let selected_commit = &self.commits[sel_idx];
        let onto_oid = if !selected_commit.parents.is_empty() {
            selected_commit.parents[0]
        } else {
            ObjectId::ZERO
        };

        let ref_store = RefStore::with_common_dir(&self.git_dir, &self.common_dir);
        let head_oid = match ref_store.resolve_head().ok().and_then(|(_, h)| h) {
            Some(h) => h,
            None => return,
        };
        let store = match RepoObjectStore::open_with_common_dir(&self.git_dir, &self.common_dir) {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut chain = Vec::new();
        let mut curr = head_oid;
        let mut found = false;

        while !curr.is_zero() {
            let summary = self
                .commits
                .iter()
                .find(|c| c.oid == curr)
                .map(|c| c.summary.clone())
                .unwrap_or_else(|| {
                    if let Ok(Object::Commit(c)) = store.read_object(&curr) {
                        c.message.lines().next().unwrap_or("").to_string()
                    } else {
                        String::new()
                    }
                });

            chain.push(crate::sequencer::RebaseTodoItem::new(
                crate::sequencer::RebaseAction::Pick,
                curr,
                summary,
            ));

            if curr == selected_commit.oid {
                found = true;
                break;
            }

            if let Ok(Object::Commit(c)) = store.read_object(&curr) {
                if let Some(&first_parent) = c.parents.first() {
                    curr = first_parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        if !found {
            return;
        }

        chain.reverse();
        let items = chain;

        self.active_modal = ActiveModal::RebaseTodo {
            items,
            selected: 0,
            onto_oid,
        };
    }

    /// Handles keyboard interaction within the RebaseTodo modal.
    pub fn handle_rebase_todo_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) {
        if let ActiveModal::RebaseTodo {
            ref mut items,
            ref mut selected,
            ..
        } = self.active_modal
        {
            if items.is_empty() {
                return;
            }
            match code {
                crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
                        || code == crossterm::event::KeyCode::Char('J')
                    {
                        if *selected + 1 < items.len() {
                            items.swap(*selected, *selected + 1);
                            *selected += 1;
                        }
                    } else {
                        *selected = (*selected + 1).min(items.len().saturating_sub(1));
                    }
                }
                crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
                        || code == crossterm::event::KeyCode::Char('K')
                    {
                        if *selected > 0 {
                            items.swap(*selected, *selected - 1);
                            *selected -= 1;
                        }
                    } else {
                        *selected = selected.saturating_sub(1);
                    }
                }
                crossterm::event::KeyCode::Char('J') if *selected + 1 < items.len() => {
                    items.swap(*selected, *selected + 1);
                    *selected += 1;
                }
                crossterm::event::KeyCode::Char('K') if *selected > 0 => {
                    items.swap(*selected, *selected - 1);
                    *selected -= 1;
                }
                crossterm::event::KeyCode::Char('p') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Pick;
                }
                crossterm::event::KeyCode::Char('r') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Reword;
                }
                crossterm::event::KeyCode::Char('e') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Edit;
                }
                crossterm::event::KeyCode::Char('s') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Squash;
                }
                crossterm::event::KeyCode::Char('f') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Fixup;
                }
                crossterm::event::KeyCode::Char('d') => {
                    items[*selected].action = crate::sequencer::RebaseAction::Drop;
                }
                crossterm::event::KeyCode::Char(' ') => {
                    items[*selected].action = match items[*selected].action {
                        crate::sequencer::RebaseAction::Pick => {
                            crate::sequencer::RebaseAction::Reword
                        }
                        crate::sequencer::RebaseAction::Reword => {
                            crate::sequencer::RebaseAction::Edit
                        }
                        crate::sequencer::RebaseAction::Edit => {
                            crate::sequencer::RebaseAction::Squash
                        }
                        crate::sequencer::RebaseAction::Squash => {
                            crate::sequencer::RebaseAction::Fixup
                        }
                        crate::sequencer::RebaseAction::Fixup => {
                            crate::sequencer::RebaseAction::Drop
                        }
                        crate::sequencer::RebaseAction::Drop => {
                            crate::sequencer::RebaseAction::Pick
                        }
                    };
                }
                _ => {}
            }
        }
    }

    /// Continues an active interactive rebase after resolving conflicts or finishing an edit.
    pub fn rebase_continue(&mut self) -> Result<(), TuiError> {
        if self.ensure_no_active_job().is_err() {
            return Ok(());
        }
        match ops::rebase_continue(&self.repo_root, &self.git_dir, &self.common_dir) {
            Ok(ops::ReplayStepOutcome::Finished) => {
                self.status_message = Some("✓ Rebase finished successfully".to_string());
            }
            Ok(ops::ReplayStepOutcome::Conflict { conflicted_paths }) => {
                self.status_message = Some(format!(
                    "✗ Rebase conflict in: {}. Resolve conflicts and continue.",
                    conflicted_paths.join(", ")
                ));
            }
            Ok(ops::ReplayStepOutcome::StoppedForEditing { commit_oid }) => {
                let short = if commit_oid.to_string().len() >= 7 {
                    &commit_oid.to_string()[..7]
                } else {
                    &commit_oid.to_string()
                };
                self.status_message = Some(format!("Stopped at {} for editing.", short));
            }
            Err(e) => {
                self.status_message = Some(format!("✗ Cannot continue rebase: {}", e));
            }
        }
        let _ = self.refresh();
        Ok(())
    }

    /// Skips the current conflicted/stopped step in rebase.
    pub fn rebase_skip(&mut self) -> Result<(), TuiError> {
        if self.ensure_no_active_job().is_err() {
            return Ok(());
        }
        match ops::rebase_skip(&self.repo_root, &self.git_dir, &self.common_dir) {
            Ok(ops::ReplayStepOutcome::Finished) => {
                self.status_message = Some("✓ Rebase finished successfully".to_string());
            }
            Ok(ops::ReplayStepOutcome::Conflict { conflicted_paths }) => {
                self.status_message = Some(format!(
                    "✗ Rebase conflict in: {}",
                    conflicted_paths.join(", ")
                ));
            }
            Ok(ops::ReplayStepOutcome::StoppedForEditing { commit_oid }) => {
                let short = if commit_oid.to_string().len() >= 7 {
                    &commit_oid.to_string()[..7]
                } else {
                    &commit_oid.to_string()
                };
                self.status_message = Some(format!("Stopped at {} for editing.", short));
            }
            Err(e) => {
                self.status_message = Some(format!("✗ Cannot skip step: {}", e));
            }
        }
        let _ = self.refresh();
        Ok(())
    }

    /// Prompts confirmation to abort the active rebase.
    pub fn prompt_rebase_abort(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::Confirm {
            title: "Abort Rebase".to_string(),
            prompt: "Abort active rebase and return to original HEAD?".to_string(),
            action: crate::model::ConfirmAction::RebaseAbort,
        };
    }

    /// Prompts confirmation to revert the selected commit.
    pub fn prompt_revert_selected_commit(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if let Some(commit) = self.commits.get(self.commits_selected) {
            self.active_modal = ActiveModal::Confirm {
                title: "Revert Commit".to_string(),
                prompt: format!(
                    "Revert commit {} (\"{}\")?",
                    commit.short_oid, commit.summary
                ),
                action: crate::model::ConfirmAction::Revert(commit.oid),
            };
        }
    }

    /// Resolves the currently selected conflicted file using the specified choice (Ours, Theirs, Both).
    pub fn resolve_selected_file_conflict(
        &mut self,
        choice: crate::model::ConflictChoice,
    ) -> Result<(), TuiError> {
        if self.ensure_no_active_job().is_err() {
            return Ok(());
        }
        if let Some(file) = self.files.get(self.files_selected) {
            let rel_path = file.path.clone();
            match ops::resolve_conflict_choice(
                &self.repo_root,
                &self.git_dir,
                &self.common_dir,
                &rel_path,
                choice,
            ) {
                Ok(()) => {
                    self.status_message =
                        Some(format!("✓ Resolved {} using {:?}", rel_path, choice));
                    let _ = self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Failed to resolve {}: {}", rel_path, e));
                }
            }
        }
        Ok(())
    }

    /// Handles keyboard interaction within the StashSave modal dialog.
    pub fn handle_stash_save_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::StashSave {
            ref mut message,
            ref mut cursor,
            ref mut include_untracked,
            ref mut staged_only,
            ref mut keep_index,
            ref mut focused_field,
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Down => {
                    *focused_field = (*focused_field + 1) % 4;
                }
                crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Up => {
                    *focused_field = (*focused_field + 3) % 4;
                }
                crossterm::event::KeyCode::Char(' ') => match *focused_field {
                    1 => {
                        *include_untracked = !*include_untracked;
                        if *include_untracked {
                            *staged_only = false;
                        }
                    }
                    2 => {
                        *staged_only = !*staged_only;
                        if *staged_only {
                            *include_untracked = false;
                            *keep_index = false;
                        }
                    }
                    3 => {
                        *keep_index = !*keep_index;
                        if *keep_index {
                            *staged_only = false;
                        }
                    }
                    _ => {
                        let idx = (*cursor).min(message.chars().count());
                        let byte_pos = message
                            .char_indices()
                            .nth(idx)
                            .map(|(i, _)| i)
                            .unwrap_or(message.len());
                        message.insert(byte_pos, ' ');
                        *cursor += 1;
                    }
                },
                _ => {}
            }
        }
    }

    /// Handles keyboard interaction within the WorktreeAdd modal dialog.
    pub fn handle_worktree_add_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::WorktreeAdd {
            ref mut create_branch,
            ref mut focused_field,
            ..
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Down => {
                    *focused_field = (*focused_field + 1) % 3;
                }
                crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Up => {
                    *focused_field = (*focused_field + 2) % 3;
                }
                crossterm::event::KeyCode::Char(' ') if *focused_field == 2 => {
                    *create_branch = !*create_branch;
                }
                _ => {}
            }
        }
    }

    /// Handles keyboard interaction within the WorktreeList modal dialog.
    pub fn handle_worktree_list_key(&mut self, code: crossterm::event::KeyCode) {
        let mut action_prompt: Option<(String, bool)> = None;
        let mut open_add = false;
        if let ActiveModal::WorktreeList {
            ref items,
            ref mut selected,
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down
                    if !items.is_empty() =>
                {
                    *selected = (*selected + 1).min(items.len() - 1);
                }
                crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                crossterm::event::KeyCode::Char('a') | crossterm::event::KeyCode::Char('n') => {
                    open_add = true;
                }
                crossterm::event::KeyCode::Char('d') => {
                    if let Some(item) = items.get(*selected) {
                        if !item.is_main {
                            action_prompt = Some((item.name.clone(), item.is_locked));
                        }
                    }
                }
                _ => {}
            }
        }
        if open_add {
            self.open_worktree_add_modal();
        } else if let Some((name, locked)) = action_prompt {
            self.prompt_remove_worktree(name, locked);
        }
    }

    /// Handles keyboard interaction within the CustomPatchMenu modal dialog.
    pub fn handle_custom_patch_menu_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::CustomPatchMenu { ref mut selected } = self.active_modal {
            match code {
                crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                    *selected = (*selected + 1).min(5);
                }
                crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    /// Opens the command palette modal.
    pub fn open_command_palette(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::CommandPalette {
            query: String::new(),
            cursor: 0,
            selected: 0,
            commands: PALETTE_COMMANDS.to_vec(),
        };
    }

    /// Opens the submodules list dialog.
    pub fn open_submodule_list_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.submodules = ops::list_submodules(&self.repo_root, &self.git_dir).unwrap_or_default();
        self.active_modal = ActiveModal::SubmoduleList {
            items: self.submodules.clone(),
            selected: 0,
        };
    }

    /// Opens the Git bisect control menu.
    pub fn open_bisect_menu_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.bisect_state = ops::get_bisect_state(&self.git_dir).unwrap_or_default();
        self.active_modal = ActiveModal::BisectMenu {
            state: self.bisect_state.clone(),
            selected: 0,
        };
    }

    /// Opens the add remote modal dialog.
    pub fn open_remote_add_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::RemoteAdd {
            name: String::new(),
            url: String::new(),
            focused_field: 0,
            cursor: 0,
        };
    }

    /// Opens the web provider links modal dialog.
    pub fn open_provider_links_modal(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        let commit_oid = self.selected_commit().map(|c| c.oid);
        let branch_opt = if self.branch_name.is_empty() {
            None
        } else {
            Some(self.branch_name.as_str())
        };
        let urls = ops::get_provider_urls(&self.git_dir, commit_oid, branch_opt);
        self.active_modal = ActiveModal::ProviderLinks { urls, selected: 0 };
    }

    /// Navigates the TUI into a submodule working directory.
    pub fn enter_submodule(&mut self, rel_path: &str) -> Result<(), TuiError> {
        let sub_root = self.repo_root.join(rel_path);
        let sub_git = sub_root.join(".git");
        if !sub_git.exists() {
            return Err(TuiError::Terminal(format!(
                "Submodule at '{}' not initialized",
                rel_path
            )));
        }
        self.repo_history.push(self.repo_root.clone());
        self.load_repository(&sub_git)?;
        self.status_message = Some(format!(
            "Navigated into submodule '{}' (press u to return)",
            rel_path
        ));
        Ok(())
    }

    /// Returns to the parent repository if navigating inside a submodule.
    pub fn return_to_parent_repo(&mut self) -> Result<(), TuiError> {
        if let Some(parent) = self.repo_history.pop() {
            let parent_git = parent.join(".git");
            self.load_repository(&parent_git)?;
            self.status_message = Some("Returned to parent repository".to_string());
        }
        Ok(())
    }

    /// Yanks the identifier of the currently selected item into clipboard / status.
    pub fn yank_selected(&mut self) {
        let yanked = match self.active_panel {
            Panel::Files => self.files.get(self.files_selected).map(|f| f.path.clone()),
            Panel::Branches => match self.branches_tab {
                BranchesTab::Local => self
                    .branches
                    .get(self.branches_selected)
                    .map(|b| b.name.clone()),
                BranchesTab::Remotes => self
                    .remotes
                    .get(self.remotes_selected)
                    .map(|r| r.name.clone()),
                BranchesTab::Tags => self.tags.get(self.tags_selected).map(|t| t.name.clone()),
            },
            Panel::Commits => match self.commits_tab {
                CommitsTab::Commits => self
                    .commits
                    .get(self.commits_selected)
                    .map(|c| c.oid.to_string()),
                CommitsTab::Reflog => self
                    .reflog
                    .get(self.reflog_selected)
                    .map(|r| r.oid.to_string()),
            },
            Panel::Stash => self
                .stashes
                .get(self.stashes_selected)
                .map(|s| format!("stash@{{{}}}", s.index)),
            _ => None,
        };
        if let Some(val) = yanked {
            self.status_message = Some(format!("Yanked '{}' to clipboard", val));
        }
    }

    /// Pushes the currently selected tag to the remote repository.
    pub fn push_selected_tag(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if let Some(tag) = self.tags.get(self.tags_selected) {
            let tag_name = tag.name.clone();
            match ops::push_tag_to_remote(&self.git_dir, None, &tag_name) {
                Ok(msg) => {
                    self.status_message = Some(format!("✓ {}", msg));
                    let _ = self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Push tag failed: {}", e));
                }
            }
        }
    }

    /// Performs a safe force-push with lease (`--force-with-lease`).
    pub fn push_force_lease(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }

        let config_path = self.common_dir.join("config");
        let config = if config_path.exists() {
            match oxidize_config::GitConfig::load_from_file(&config_path) {
                Ok(config) => config,
                Err(error) => {
                    self.status_message = Some(format!(
                        "✗ Force-push (with lease) failed to load repository config: {}",
                        error
                    ));
                    return;
                }
            }
        } else {
            oxidize_config::GitConfig::new()
        };

        let remote_name = config
            .get("branch", Some(&self.branch_name), "remote")
            .unwrap_or("origin");
        let merge_ref = config.get("branch", Some(&self.branch_name), "merge");
        let remote_branch_short = if let Some(m) = merge_ref {
            m.strip_prefix("refs/heads/").unwrap_or(m)
        } else {
            &self.branch_name
        };

        let tracking_ref = format!("refs/remotes/{}/{}", remote_name, remote_branch_short);
        let ref_store = RefStore::with_common_dir(&self.git_dir, &self.common_dir);
        let expected_oid = match ref_store.read_ref(&tracking_ref) {
            Ok(oid) => Some(oid),
            Err(RefError::NotFound(_)) => None,
            Err(error) => {
                self.status_message = Some(format!(
                    "✗ Force-push (with lease) failed to read tracking ref '{}': {}",
                    tracking_ref, error
                ));
                return;
            }
        };

        match ops::push_to_remote_ext(
            &self.repo_root,
            &self.git_dir,
            Some(remote_name),
            Some(remote_branch_short),
            false,
            expected_oid,
        ) {
            Ok(msg) => {
                self.status_message = Some(format!("✓ {}", msg));
                let _ = self.refresh();
            }
            Err(e) => {
                self.status_message = Some(format!("✗ Force-push (with lease) failed: {}", e));
            }
        }
    }

    /// Prompts confirmation to delete the selected remote branch.
    pub fn prompt_delete_selected_remote_branch(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        if self.active_panel == Panel::Branches && self.branches_tab == BranchesTab::Remotes {
            if let Some(remote) = self.remotes.get(self.remotes_selected) {
                if let Some((r_name, b_name)) = remote.name.split_once('/') {
                    self.active_modal = ActiveModal::Confirm {
                        title: "Delete Remote Branch".to_string(),
                        prompt: format!("Delete branch '{}' on remote '{}'?", b_name, r_name),
                        action: ConfirmAction::DeleteRemoteBranch {
                            remote: r_name.to_string(),
                            branch: b_name.to_string(),
                        },
                    };
                }
            }
        }
    }

    /// Prompts confirmation to remove a configured remote.
    pub fn prompt_remove_remote(&mut self, remote_name: String) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        self.active_modal = ActiveModal::Confirm {
            title: "Delete Remote".to_string(),
            prompt: format!(
                "Remove remote '{}' and all its tracking branches?",
                remote_name
            ),
            action: ConfirmAction::DeleteRemote(remote_name),
        };
    }

    /// Starts git bisect with the selected or first commit as bad.
    pub fn bisect_start(&mut self) {
        if let Some(commit) = self.commits.first() {
            match ops::bisect_start(&self.repo_root, &self.git_dir, Some(commit.oid), None) {
                Ok(state) => {
                    self.status_message = Some(format!(
                        "✓ Bisect started (marked {} as bad)",
                        commit.short_oid
                    ));
                    self.bisect_state = state;
                    let _ = self.refresh();
                }
                Err(e) => self.status_message = Some(format!("✗ Bisect start failed: {}", e)),
            }
        }
    }

    /// Marks the current commit as bad in the active bisect session.
    pub fn bisect_mark_bad(&mut self) {
        match ops::bisect_mark(&self.repo_root, &self.git_dir, true) {
            Ok(state) => {
                if let Some(culprit) = state.culprit_oid {
                    self.status_message = Some(format!(
                        "✓ Bisect finished! Culprit: {}",
                        &culprit.to_string()[..7]
                    ));
                } else {
                    self.status_message = Some(format!(
                        "✓ Marked bad (~{} steps left)",
                        state.remaining_steps
                    ));
                }
                self.bisect_state = state;
                let _ = self.refresh();
            }
            Err(e) => self.status_message = Some(format!("✗ Bisect bad failed: {}", e)),
        }
    }

    /// Marks the current commit as good in the active bisect session.
    pub fn bisect_mark_good(&mut self) {
        match ops::bisect_mark(&self.repo_root, &self.git_dir, false) {
            Ok(state) => {
                if let Some(culprit) = state.culprit_oid {
                    self.status_message = Some(format!(
                        "✓ Bisect finished! Culprit: {}",
                        &culprit.to_string()[..7]
                    ));
                } else {
                    self.status_message = Some(format!(
                        "✓ Marked good (~{} steps left)",
                        state.remaining_steps
                    ));
                }
                self.bisect_state = state;
                let _ = self.refresh();
            }
            Err(e) => self.status_message = Some(format!("✗ Bisect good failed: {}", e)),
        }
    }

    /// Skips the current commit in the active bisect session.
    pub fn bisect_skip(&mut self) {
        match ops::bisect_skip(&self.repo_root, &self.git_dir) {
            Ok(state) => {
                self.status_message = Some(format!(
                    "✓ Skipped commit (~{} steps left)",
                    state.remaining_steps
                ));
                self.bisect_state = state;
                let _ = self.refresh();
            }
            Err(e) => self.status_message = Some(format!("✗ Bisect skip failed: {}", e)),
        }
    }

    /// Resets / ends the active bisect session.
    pub fn bisect_reset(&mut self) {
        match ops::bisect_reset(&self.repo_root, &self.git_dir) {
            Ok(()) => {
                self.status_message = Some("✓ Bisect reset; returned to original HEAD".to_string());
                self.bisect_state = ops::get_bisect_state(&self.git_dir).unwrap_or_default();
                let _ = self.refresh();
            }
            Err(e) => self.status_message = Some(format!("✗ Bisect reset failed: {}", e)),
        }
    }

    /// Fetches all remotes and prunes deleted tracking branches.
    pub fn fetch_all(&mut self) {
        if self.ensure_no_active_job().is_err() {
            return;
        }
        match ops::fetch_all_remotes(&self.git_dir, true) {
            Ok(msg) => {
                let summary = if msg.is_empty() {
                    "✓ Fetched all remotes (up to date)".to_string()
                } else {
                    format!("✓ Fetched all remotes: {}", msg)
                };
                self.status_message = Some(summary);
                let _ = self.refresh();
            }
            Err(e) => {
                self.status_message = Some(format!("✗ Fetch failed: {}", e));
            }
        }
    }

    /// Executes an action chosen from the global Command Palette.
    pub fn execute_palette_command(&mut self, id: &str) {
        match id {
            "commit" => self.open_commit_modal(),
            "amend" => self.open_amend_modal(),
            "push" => {
                if let Err(e) = self.push() {
                    self.set_error(e);
                }
            }
            "push_force_lease" => self.push_force_lease(),
            "pull" => {
                if let Err(e) = self.pull() {
                    self.set_error(e);
                }
            }
            "fetch_all" => self.fetch_all(),
            "add_remote" => self.open_remote_add_modal(),
            "branch_create" => self.open_create_branch_modal(),
            "branch_rename" => self.prompt_rename_selected_branch(),
            "tag_create" => self.prompt_create_tag(),
            "stash_save" => self.open_stash_save_modal(),
            "stash_pop" => {
                if let Err(e) = self.pop_selected_stash() {
                    self.set_error(e);
                }
            }
            "worktrees" => self.open_worktree_list_modal(),
            "submodules" => self.open_submodule_list_modal(),
            "bisect" => self.open_bisect_menu_modal(),
            "custom_patches" => self.open_custom_patch_menu(),
            "provider_links" => self.open_provider_links_modal(),
            "help" => self.active_modal = ActiveModal::Help,
            "refresh" => {
                if let Err(e) = self.refresh() {
                    self.set_error(e);
                }
            }
            "quit" => self.should_quit = true,
            _ => {}
        }
    }

    /// Handles keyboard interaction within the CommandPalette modal dialog.
    pub fn handle_command_palette_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::CommandPalette {
            ref mut selected,
            ref commands,
            ..
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Down if !commands.is_empty() => {
                    *selected = (*selected + 1).min(commands.len() - 1);
                }
                crossterm::event::KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    /// Handles keyboard interaction within the SubmoduleList modal dialog.
    pub fn handle_submodule_list_key(&mut self, code: crossterm::event::KeyCode) {
        let mut do_update: Option<String> = None;
        let mut do_init: Option<String> = None;
        if let ActiveModal::SubmoduleList {
            ref items,
            ref mut selected,
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down
                    if !items.is_empty() =>
                {
                    *selected = (*selected + 1).min(items.len() - 1);
                }
                crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                crossterm::event::KeyCode::Char('u') => {
                    if let Some(sub) = items.get(*selected) {
                        do_update = Some(sub.name.clone());
                    }
                }
                crossterm::event::KeyCode::Char('i') => {
                    if let Some(sub) = items.get(*selected) {
                        do_init = Some(sub.name.clone());
                    }
                }
                _ => {}
            }
        }
        if let Some(name) = do_update {
            match ops::submodule_update(&self.repo_root, &self.git_dir, &name) {
                Ok(()) => {
                    self.status_message = Some(format!("✓ Updated submodule '{}'", name));
                    self.open_submodule_list_modal();
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Failed to update submodule: {}", e));
                }
            }
        } else if let Some(name) = do_init {
            match ops::submodule_init(&self.repo_root, &self.git_dir, &name) {
                Ok(()) => {
                    self.status_message = Some(format!("✓ Initialized submodule '{}'", name));
                    self.open_submodule_list_modal();
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Failed to init submodule: {}", e));
                }
            }
        }
    }

    /// Handles keyboard interaction within the BisectMenu modal dialog.
    pub fn handle_bisect_menu_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::BisectMenu {
            ref mut selected,
            ref state,
        } = self.active_modal
        {
            let max_idx = if state.is_active { 3 } else { 1 };
            match code {
                crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                    *selected = (*selected + 1).min(max_idx);
                }
                crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                _ => {}
            }
        }
    }

    /// Handles keyboard interaction within the RemoteAdd modal dialog.
    pub fn handle_remote_add_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::RemoteAdd {
            ref mut focused_field,
            ref mut cursor,
            ref name,
            ref url,
            ..
        } = self.active_modal
        {
            match code {
                crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Down => {
                    *focused_field = (*focused_field + 1) % 2;
                    *cursor = if *focused_field == 0 {
                        name.chars().count()
                    } else {
                        url.chars().count()
                    };
                }
                crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Up => {
                    *focused_field = (*focused_field + 1) % 2;
                    *cursor = if *focused_field == 0 {
                        name.chars().count()
                    } else {
                        url.chars().count()
                    };
                }
                _ => {}
            }
        }
    }

    /// Handles keyboard interaction within the ProviderLinks modal dialog.
    pub fn handle_provider_links_key(&mut self, code: crossterm::event::KeyCode) {
        if let ActiveModal::ProviderLinks {
            ref mut selected,
            ref urls,
        } = self.active_modal
        {
            let count = [
                urls.repo_url.is_some(),
                urls.commit_url.is_some(),
                urls.branch_url.is_some(),
                urls.pr_url.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count();
            if count > 0 {
                match code {
                    crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                        *selected = (*selected + 1).min(count - 1);
                    }
                    crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                        *selected = selected.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }
    }
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
