//! Git reference management, atomic ref updates, packed-refs, and rev-parse.

use crate::signature::get_default_signature;
use crate::RefError;
use oxidize_core::id::ObjectId;
use oxidize_core::lock::LockFile;
use oxidize_core::object::{Object, Signature};
use oxidize_core::store::ObjectReader;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// An entry in a reference log (`.git/logs/*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogEntry {
    /// Previous target object ID.
    pub old_oid: ObjectId,
    /// New target object ID.
    pub new_oid: ObjectId,
    /// Committer signature and timestamp.
    pub signature: Signature,
    /// Action and message.
    pub message: String,
}

fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            c => normalized.push(c),
        }
    }
    normalized
}

fn resolve_common_dir(git_dir: &Path) -> PathBuf {
    let commondir_file = git_dir.join("commondir");
    if commondir_file.is_file() {
        if let Ok(rel) = fs::read_to_string(&commondir_file) {
            let rel = rel.trim();
            if !rel.is_empty() {
                let joined = git_dir.join(rel);
                if let Ok(canon) = joined.canonicalize() {
                    return canon;
                }
                return normalize_path_components(&joined);
            }
        }
    }
    git_dir.to_path_buf()
}

/// Access and manipulation of repository references.
#[derive(Debug, Clone)]
pub struct RefStore {
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl RefStore {
    /// Creates a `RefStore` for the given `.git` directory, automatically resolving `commondir` if present.
    pub fn new(git_dir: impl Into<PathBuf>) -> Self {
        let git_dir = git_dir.into();
        let common_dir = resolve_common_dir(&git_dir);
        Self {
            git_dir,
            common_dir,
        }
    }

    /// Creates a `RefStore` with an explicit `common_dir`.
    pub fn with_common_dir(git_dir: impl Into<PathBuf>, common_dir: impl Into<PathBuf>) -> Self {
        Self {
            git_dir: git_dir.into(),
            common_dir: common_dir.into(),
        }
    }

    /// Returns the path to the per-worktree `.git` directory.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Returns the path to the common `.git` directory containing shared refs and objects.
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    /// Reads and resolves `HEAD`. Returns `(ref_name_or_detached, Option<ObjectId>)`.
    pub fn resolve_head(&self) -> Result<(String, Option<ObjectId>), RefError> {
        let head_path = self.git_dir.join("HEAD");
        if !head_path.exists() {
            return Ok(("master".to_string(), None));
        }

        let content = fs::read_to_string(head_path)?;
        let content = content.trim();

        if let Some(rest) = content.strip_prefix("ref: refs/heads/") {
            let branch = rest.to_string();
            let oid = self.read_ref(&format!("refs/heads/{}", branch)).ok();
            Ok((branch, oid))
        } else if let Some(rest) = content.strip_prefix("ref: ") {
            let full_ref = rest.to_string();
            let oid = self.read_ref(&full_ref).ok();
            Ok((full_ref, oid))
        } else if let Ok(oid) = content.parse::<ObjectId>() {
            Ok(("HEAD".to_string(), Some(oid)))
        } else {
            Ok(("master".to_string(), None))
        }
    }

    /// Reads a reference by full or short name, checking loose refs then `packed-refs`.
    pub fn read_ref(&self, ref_name: &str) -> Result<ObjectId, RefError> {
        let normalized = self.normalize_ref_name(ref_name);

        // 1. Check per-worktree loose ref file (e.g. for HEAD, or worktree-local refs)
        let loose_path = self.git_dir.join(&normalized);
        if loose_path.is_file() {
            let content = fs::read_to_string(loose_path)?;
            let content = content.trim();
            if let Ok(oid) = content.parse::<ObjectId>() {
                return Ok(oid);
            }
        }

        // 2. Check common_dir loose ref file (for shared refs)
        if self.common_dir != self.git_dir {
            let common_path = self.common_dir.join(&normalized);
            if common_path.is_file() {
                let content = fs::read_to_string(common_path)?;
                let content = content.trim();
                if let Ok(oid) = content.parse::<ObjectId>() {
                    return Ok(oid);
                }
            }
        }

        // 3. Check packed-refs in common_dir
        let packed = self.read_packed_refs()?;
        if let Some(oid) = packed.get(&normalized) {
            return Ok(*oid);
        }

        Err(RefError::NotFound(ref_name.to_string()))
    }

    /// Lists all local branch names (`refs/heads/*`) and their target commit IDs.
    pub fn list_branches(&self) -> Result<BTreeMap<String, ObjectId>, RefError> {
        let mut branches = BTreeMap::new();

        // 1. Packed refs (from common_dir)
        let packed = self.read_packed_refs()?;
        for (name, oid) in packed {
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                branches.insert(branch.to_string(), oid);
            }
        }

        // 2. Loose refs in common_dir (override packed)
        let heads_dir = self.common_dir.join("refs/heads");
        if heads_dir.exists() {
            self.scan_refs_dir(&heads_dir, "refs/heads", &mut branches)?;
        }

        // 3. Loose refs in git_dir if differing from common_dir
        if self.git_dir != self.common_dir {
            let wt_heads_dir = self.git_dir.join("refs/heads");
            if wt_heads_dir.exists() {
                self.scan_refs_dir(&wt_heads_dir, "refs/heads", &mut branches)?;
            }
        }

        Ok(branches)
    }

    /// Lists all tags (`refs/tags/*`) and their target object IDs.
    pub fn list_tags(&self) -> Result<BTreeMap<String, ObjectId>, RefError> {
        let mut tags = BTreeMap::new();

        // 1. Packed refs
        let packed = self.read_packed_refs()?;
        for (name, oid) in packed {
            if let Some(tag) = name.strip_prefix("refs/tags/") {
                tags.insert(tag.to_string(), oid);
            }
        }

        // 2. Loose refs in common_dir (override packed)
        let tags_dir = self.common_dir.join("refs/tags");
        if tags_dir.exists() {
            self.scan_refs_dir(&tags_dir, "refs/tags", &mut tags)?;
        }

        Ok(tags)
    }

    /// Lists all remote branches (`refs/remotes/*`) and their target commit IDs.
    pub fn list_remotes(&self) -> Result<BTreeMap<String, ObjectId>, RefError> {
        let mut remotes = BTreeMap::new();

        // 1. Packed refs
        let packed = self.read_packed_refs()?;
        for (name, oid) in packed {
            if let Some(remote) = name.strip_prefix("refs/remotes/") {
                remotes.insert(remote.to_string(), oid);
            }
        }

        // 2. Loose refs in common_dir (override packed)
        let remotes_dir = self.common_dir.join("refs/remotes");
        if remotes_dir.exists() {
            self.scan_refs_dir(&remotes_dir, "refs/remotes", &mut remotes)?;
        }

        Ok(remotes)
    }

    fn scan_refs_dir(
        &self,
        dir: &Path,
        prefix: &str,
        out: &mut BTreeMap<String, ObjectId>,
    ) -> Result<(), RefError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                self.scan_refs_dir(&path, &format!("{}/{}", prefix, name), out)?;
            } else if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(oid) = content.trim().parse::<ObjectId>() {
                        let item_name = if let Some(stripped) = prefix
                            .strip_prefix("refs/heads")
                            .or_else(|| prefix.strip_prefix("refs/tags"))
                            .or_else(|| prefix.strip_prefix("refs/remotes"))
                        {
                            if stripped.is_empty() {
                                name
                            } else {
                                format!("{}/{}", stripped.trim_start_matches('/'), name)
                            }
                        } else {
                            name
                        };
                        out.insert(item_name, oid);
                    }
                }
            }
        }
        Ok(())
    }

    /// Updates a reference atomically with a `.lock` file and writes a reflog entry.
    pub fn update_ref(
        &self,
        ref_name: &str,
        new_oid: &ObjectId,
        old_oid: Option<&ObjectId>,
        message: &str,
    ) -> Result<(), RefError> {
        let normalized = self.normalize_ref_name(ref_name);
        oxidize_core::validate_ref_name(&normalized)
            .map_err(|e| RefError::InvalidName(e.to_string()))?;

        let ref_path = if normalized == "HEAD" {
            self.git_dir.join("HEAD")
        } else {
            self.common_dir.join(&normalized)
        };
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut lock = oxidize_core::LockFile::acquire(&ref_path).map_err(|e| match e {
            oxidize_core::CoreError::LockError(msg) => RefError::RevParseError(msg),
            other => RefError::Core(other),
        })?;

        let current_oid = self.read_ref(&normalized).ok();

        if let Some(expected) = old_oid {
            if current_oid.as_ref() != Some(expected) {
                return Err(RefError::RevParseError(format!(
                    "ref {} does not match expected old value {}",
                    ref_name, expected
                )));
            }
        }

        use std::io::Write;
        writeln!(lock, "{}", new_oid)?;
        lock.commit()?;

        // Append reflog
        let from_oid = current_oid.unwrap_or(ObjectId::ZERO);
        let sig = get_default_signature(Some(&self.git_dir));
        self.append_reflog(&normalized, &from_oid, new_oid, &sig, message)?;

        // Also append to HEAD reflog if updating the active branch
        let head_info = self.resolve_head()?;
        if head_info.0 == ref_name || format!("refs/heads/{}", head_info.0) == normalized {
            self.append_reflog("HEAD", &from_oid, new_oid, &sig, message)?;
        }

        Ok(())
    }

    /// Appends an entry to `.git/logs/<ref_name>`.
    pub fn append_reflog(
        &self,
        ref_name: &str,
        old_oid: &ObjectId,
        new_oid: &ObjectId,
        sig: &Signature,
        message: &str,
    ) -> Result<(), RefError> {
        let log_base = if ref_name == "HEAD" {
            &self.git_dir
        } else {
            &self.common_dir
        };
        let log_path = log_base.join("logs").join(ref_name);
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;

        writeln!(file, "{} {} {}\t{}", old_oid, new_oid, sig, message)?;
        file.flush()?;
        Ok(())
    }

    /// Reads all entries from `.git/logs/<ref_name>`.
    pub fn read_reflog(&self, ref_name: &str) -> Result<Vec<ReflogEntry>, RefError> {
        let log_path = if ref_name == "HEAD" {
            self.git_dir.join("logs").join("HEAD")
        } else {
            let common_log = self.common_dir.join("logs").join(ref_name);
            if common_log.exists() {
                common_log
            } else {
                self.git_dir.join("logs").join(ref_name)
            }
        };
        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(log_path)?;
        let mut entries = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let (meta_part, message) = match line.split_once('\t') {
                Some((meta, msg)) => (meta, msg.to_string()),
                None => (line, String::new()),
            };

            let mut parts = meta_part.split_whitespace();
            let old_str = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            let new_str = match parts.next() {
                Some(s) => s,
                None => continue,
            };

            let old_oid = match old_str.parse::<ObjectId>() {
                Ok(oid) => oid,
                Err(_) => continue,
            };
            let new_oid = match new_str.parse::<ObjectId>() {
                Ok(oid) => oid,
                Err(_) => continue,
            };

            let sig_start = (old_str.len() + 1 + new_str.len()).min(meta_part.len());
            let sig_str = meta_part[sig_start..].trim();
            let signature = match Signature::parse(sig_str) {
                Ok(sig) => sig,
                Err(_) => Signature {
                    name: String::new(),
                    email: String::new(),
                    time_seconds: 0,
                    tz_offset: "+0000".to_string(),
                },
            };

            entries.push(ReflogEntry {
                old_oid,
                new_oid,
                signature,
                message,
            });
        }

        Ok(entries)
    }

    /// Returns the stash entries in Git stack order (index 0 is newest `stash@{0}`).
    pub fn stash_list(&self) -> Result<Vec<ReflogEntry>, RefError> {
        let mut entries = self.read_reflog("refs/stash")?;
        entries.reverse();
        Ok(entries)
    }

    /// Drops a stash entry by index (0 is newest `stash@{0}`).
    /// Rewrites the reflog and updates `refs/stash` to maintain stack continuity,
    /// or removes both if the last stash was dropped.
    pub fn stash_drop(&self, index: usize) -> Result<ObjectId, RefError> {
        let mut entries = self.read_reflog("refs/stash")?;
        if entries.is_empty() || index >= entries.len() {
            return Err(RefError::NotFound(format!("refs/stash@{{{}}}", index)));
        }

        let real_idx = entries.len() - 1 - index;
        let removed = entries.remove(real_idx);

        let stash_ref = self.common_dir.join("refs").join("stash");
        let stash_log = self.common_dir.join("logs").join("refs").join("stash");

        if entries.is_empty() {
            let _ = fs::remove_file(&stash_ref);
            let _ = fs::remove_file(&stash_log);
        } else {
            // Rewrite the reflog: line 0 has old_oid = 0, subsequent lines have old_oid = previous new_oid
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&stash_log)?;

            let mut prev_oid = ObjectId::ZERO;
            for entry in &entries {
                writeln!(
                    file,
                    "{} {} {}\t{}",
                    prev_oid, entry.new_oid, entry.signature, entry.message
                )?;
                prev_oid = entry.new_oid;
            }
            file.flush()?;

            // Update refs/stash to the newest remaining entry
            let newest = entries.last().unwrap();
            let mut lock = LockFile::acquire(&stash_ref).map_err(RefError::Core)?;
            lock.write_all(format!("{}\n", newest.new_oid).as_bytes())?;
            lock.commit().map_err(RefError::Core)?;
        }

        Ok(removed.new_oid)
    }

    /// Resolves a revision specifier (`HEAD`, `HEAD~2`, branch name, short/full SHA) to an `ObjectId`.
    pub fn resolve_rev(&self, rev: &str, store: &impl ObjectReader) -> Result<ObjectId, RefError> {
        let rev = rev.trim();

        // Check for parent / ancestor navigation like `HEAD~1` or `HEAD^`
        if let Some((base, count)) = rev.split_once('~') {
            let base_oid = self.resolve_rev(base, store)?;
            let n: usize = count.parse().map_err(|_| {
                RefError::RevParseError(format!("invalid ancestor count in {}", rev))
            })?;
            return self.walk_ancestors(base_oid, n, store);
        }

        if let Some((base, parent_num)) = rev.split_once('^') {
            let base_oid = self.resolve_rev(base, store)?;
            let p_idx: usize = if parent_num.is_empty() {
                1
            } else {
                parent_num.parse().map_err(|_| {
                    RefError::RevParseError(format!("invalid parent index in {}", rev))
                })?
            };
            return self.get_nth_parent(base_oid, p_idx, store);
        }

        if rev == "HEAD" {
            let (_name, oid_opt) = self.resolve_head()?;
            return oid_opt.ok_or_else(|| RefError::RevParseError("HEAD has no commit".into()));
        }

        // Try direct ref name
        if let Ok(oid) = self.read_ref(rev) {
            return Ok(oid);
        }

        // Try object prefix
        if let Ok(oid) = store.find_by_prefix(rev) {
            return Ok(oid);
        }

        Err(RefError::RevParseError(format!(
            "revision '{}' not found",
            rev
        )))
    }

    fn walk_ancestors(
        &self,
        start: ObjectId,
        n: usize,
        store: &impl ObjectReader,
    ) -> Result<ObjectId, RefError> {
        let mut curr = start;
        for _ in 0..n {
            curr = self.get_nth_parent(curr, 1, store)?;
        }
        Ok(curr)
    }

    fn get_nth_parent(
        &self,
        oid: ObjectId,
        parent_num: usize,
        store: &impl ObjectReader,
    ) -> Result<ObjectId, RefError> {
        let obj = store.read_object(&oid)?;
        let commit = match obj {
            Object::Commit(c) => c,
            _ => {
                return Err(RefError::RevParseError(format!(
                    "object {} is not a commit",
                    oid
                )))
            }
        };

        if parent_num == 0 || parent_num > commit.parents.len() {
            return Err(RefError::RevParseError(format!(
                "commit {} has only {} parents, requested parent {}",
                oid,
                commit.parents.len(),
                parent_num
            )));
        }

        Ok(commit.parents[parent_num - 1])
    }

    fn read_packed_refs(&self) -> Result<BTreeMap<String, ObjectId>, RefError> {
        let mut map = BTreeMap::new();
        let packed_path = self.common_dir.join("packed-refs");
        if !packed_path.exists() {
            return Ok(map);
        }

        let content = fs::read_to_string(packed_path)?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                continue;
            }

            if let Some((sha, name)) = line.split_once(' ') {
                if let Ok(oid) = sha.parse::<ObjectId>() {
                    map.insert(name.trim().to_string(), oid);
                }
            }
        }

        Ok(map)
    }

    /// Creates a new branch reference pointing to `target_oid`.
    pub fn create_branch(&self, name: &str, target_oid: &ObjectId) -> Result<(), RefError> {
        oxidize_core::validate_branch_name(name)?;
        let branch_path = self.common_dir.join("refs/heads").join(name);
        if let Some(parent) = branch_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut lock = oxidize_core::LockFile::acquire(&branch_path).map_err(|e| match e {
            oxidize_core::CoreError::LockError(msg) => RefError::InvalidName(msg),
            other => RefError::Core(other),
        })?;

        if branch_path.exists() {
            return Err(RefError::InvalidName(format!(
                "a branch named '{}' already exists",
                name
            )));
        }

        let packed = self.read_packed_refs()?;
        if packed.contains_key(&format!("refs/heads/{}", name)) {
            return Err(RefError::InvalidName(format!(
                "a branch named '{}' already exists",
                name
            )));
        }

        use std::io::Write;
        writeln!(lock, "{}", target_oid)?;
        lock.commit()?;
        Ok(())
    }

    /// Deletes any reference by full name (e.g. `refs/heads/foo`, `refs/remotes/origin/bar`, `refs/tags/v1.0`),
    /// removing loose files from `common_dir` and `git_dir`, and removing the entry from `packed-refs`.
    pub fn delete_ref(&self, ref_name: &str) -> Result<(), RefError> {
        let mut deleted = false;

        // 1. Try deleting loose ref file from common_dir
        let ref_path = self.common_dir.join(ref_name);
        if ref_path.exists() {
            fs::remove_file(&ref_path)?;
            deleted = true;
        }

        // 2. Try deleting loose ref file from git_dir if different
        if self.git_dir != self.common_dir {
            let wt_ref_path = self.git_dir.join(ref_name);
            if wt_ref_path.exists() {
                fs::remove_file(&wt_ref_path)?;
                deleted = true;
            }
        }

        // 3. Remove from packed-refs if present
        let packed_path = self.common_dir.join("packed-refs");
        if packed_path.exists() {
            let content = fs::read_to_string(&packed_path)?;
            let mut new_lines = Vec::new();
            let mut in_packed = false;
            let mut skip_next_peeled = false;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('^') && skip_next_peeled {
                    skip_next_peeled = false;
                    continue;
                }
                skip_next_peeled = false;

                if trimmed.is_empty() || trimmed.starts_with('#') {
                    new_lines.push(line.to_string());
                    continue;
                }

                if let Some((_sha, rname)) = trimmed.split_once(' ') {
                    if rname.trim() == ref_name {
                        in_packed = true;
                        skip_next_peeled = true;
                        continue;
                    }
                }
                new_lines.push(line.to_string());
            }

            if in_packed {
                let mut lock = LockFile::acquire(&packed_path).map_err(RefError::Core)?;
                use std::io::Write;
                for line in new_lines {
                    writeln!(lock, "{}", line)?;
                }
                lock.commit().map_err(RefError::Core)?;
                deleted = true;
            }
        }

        if !deleted {
            return Err(RefError::NotFound(format!("ref '{}' not found", ref_name)));
        }

        Ok(())
    }

    /// Deletes a branch reference, removing loose ref files and purging from packed-refs if present.
    pub fn delete_branch(&self, name: &str) -> Result<(), RefError> {
        oxidize_core::validate_branch_name(name)?;
        let ref_full = format!("refs/heads/{}", name);
        self.delete_ref(&ref_full)
    }

    /// Sets `HEAD` to point symbolically to a branch.
    pub fn set_head_symbolic(&self, branch_name: &str) -> Result<(), RefError> {
        oxidize_core::validate_branch_name(branch_name)?;
        let head_path = self.git_dir.join("HEAD");
        let mut lock = oxidize_core::LockFile::acquire(&head_path).map_err(RefError::Core)?;
        use std::io::Write;
        writeln!(lock, "ref: refs/heads/{}", branch_name)?;
        lock.commit()?;
        Ok(())
    }

    /// Sets `HEAD` to a detached commit OID.
    pub fn set_head_detached(&self, commit_oid: &ObjectId) -> Result<(), RefError> {
        let head_path = self.git_dir.join("HEAD");
        let mut lock = oxidize_core::LockFile::acquire(&head_path).map_err(RefError::Core)?;
        use std::io::Write;
        writeln!(lock, "{}", commit_oid)?;
        lock.commit()?;
        Ok(())
    }

    /// Finds the Lowest Common Ancestor (merge base) between two commits.
    pub fn find_merge_base(
        &self,
        store: &impl ObjectReader,
        commit_a: &ObjectId,
        commit_b: &ObjectId,
    ) -> Result<Option<ObjectId>, RefError> {
        if commit_a == commit_b {
            return Ok(Some(*commit_a));
        }

        // Collect all ancestors of commit_a
        let mut ancestors_a = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(*commit_a);
        ancestors_a.insert(*commit_a);

        while let Some(curr) = queue.pop_front() {
            if let Ok(Object::Commit(c)) = store.read_object(&curr) {
                for p in c.parents {
                    if ancestors_a.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
        }

        // BFS ancestors of commit_b until hitting an ancestor of commit_a
        let mut queue_b = std::collections::VecDeque::new();
        let mut visited_b = std::collections::HashSet::new();
        queue_b.push_back(*commit_b);
        visited_b.insert(*commit_b);

        while let Some(curr) = queue_b.pop_front() {
            if ancestors_a.contains(&curr) {
                return Ok(Some(curr));
            }

            if let Ok(Object::Commit(c)) = store.read_object(&curr) {
                for p in c.parents {
                    if visited_b.insert(p) {
                        queue_b.push_back(p);
                    }
                }
            }
        }

        Ok(None)
    }

    fn normalize_ref_name(&self, name: &str) -> String {
        if name.starts_with("refs/") || name == "HEAD" {
            name.to_string()
        } else {
            format!("refs/heads/{}", name)
        }
    }

    /// Renames a branch from `old_name` to `new_name`.
    /// Updates the branch ref, moves reflog, updates `HEAD` if currently on `old_name`,
    /// and deletes the old ref (both loose and packed).
    pub fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<(), RefError> {
        oxidize_core::validate_branch_name(new_name)?;
        let old_ref = format!("refs/heads/{}", old_name);
        let new_ref = format!("refs/heads/{}", new_name);

        let target_oid = self
            .read_ref(&old_ref)
            .map_err(|_| RefError::NotFound(format!("branch '{}' not found", old_name)))?;

        // Verify new branch does not already exist
        if self.read_ref(&new_ref).is_ok() {
            return Err(RefError::InvalidName(format!(
                "a branch named '{}' already exists",
                new_name
            )));
        }

        // 1. Create new branch ref pointing to target_oid
        self.create_branch(new_name, &target_oid)?;

        // 2. If HEAD currently points to old_name, point HEAD to new_name
        let (current_head_branch, _) = self.resolve_head()?;
        let is_current = current_head_branch == old_name;
        if is_current {
            self.set_head_symbolic(new_name)?;
        }

        // 3. Move reflog if it exists
        let old_log = self
            .common_dir
            .join("logs")
            .join("refs/heads")
            .join(old_name);
        let new_log = self
            .common_dir
            .join("logs")
            .join("refs/heads")
            .join(new_name);
        if old_log.exists() {
            if let Some(parent) = new_log.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&old_log, &new_log);
        }

        // 4. Delete old branch ref
        self.delete_branch(old_name)?;

        // 5. Append reflog entry
        let sig = get_default_signature(Some(&self.git_dir));
        let log_msg = format!(
            "Branch: renamed refs/heads/{} to refs/heads/{}",
            old_name, new_name
        );
        let _ = self.append_reflog(&new_ref, &ObjectId::ZERO, &target_oid, &sig, &log_msg);
        if is_current {
            let _ = self.append_reflog("HEAD", &target_oid, &target_oid, &sig, &log_msg);
        }

        Ok(())
    }

    /// Creates a lightweight tag pointing to `target_oid`.
    pub fn create_tag(&self, name: &str, target_oid: &ObjectId) -> Result<(), RefError> {
        let tag_ref = format!("refs/tags/{}", name);
        oxidize_core::validate_ref_name(&tag_ref)
            .map_err(|e| RefError::InvalidName(e.to_string()))?;

        let tag_path = self.common_dir.join("refs/tags").join(name);
        if tag_path.exists() {
            return Err(RefError::InvalidName(format!(
                "tag '{}' already exists",
                name
            )));
        }
        let packed = self.read_packed_refs()?;
        if packed.contains_key(&tag_ref) {
            return Err(RefError::InvalidName(format!(
                "tag '{}' already exists",
                name
            )));
        }

        if let Some(parent) = tag_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut lock = LockFile::acquire(&tag_path).map_err(RefError::Core)?;
        use std::io::Write;
        writeln!(lock, "{}", target_oid)?;
        lock.commit().map_err(RefError::Core)?;
        Ok(())
    }

    /// Deletes a tag by name, removing loose files and purging from packed-refs.
    pub fn delete_tag(&self, name: &str) -> Result<(), RefError> {
        let tag_ref = format!("refs/tags/{}", name);
        self.delete_ref(&tag_ref)
            .map_err(|_| RefError::NotFound(format!("tag '{}' not found", name)))
    }
}
