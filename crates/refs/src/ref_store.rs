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

fn snapshot_file(path: &Path) -> Result<Option<Vec<u8>>, RefError> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(fs::read(path)?)),
        Ok(_) => Err(RefError::Io(std::io::Error::other(format!(
            "expected '{}' to be a regular file",
            path.display()
        )))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RefError::Io(e)),
    }
}

fn write_file_atomic(path: &Path, data: &[u8]) -> Result<(), RefError> {
    let mut lock = LockFile::acquire(path).map_err(RefError::Core)?;
    lock.write_all(data)?;
    lock.commit().map_err(RefError::Core)
}

fn remove_regular_file_if_exists(path: &Path) -> Result<(), RefError> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            fs::remove_file(path)?;
            Ok(())
        }
        Ok(_) => Err(RefError::Io(std::io::Error::other(format!(
            "expected '{}' to be a regular file",
            path.display()
        )))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RefError::Io(e)),
    }
}

fn restore_file(path: &Path, snapshot: &Option<Vec<u8>>) -> Result<(), RefError> {
    match snapshot {
        Some(data) => write_file_atomic(path, data),
        None => remove_regular_file_if_exists(path),
    }
}

fn rollback_files(snapshots: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), RefError> {
    let mut first_error = None;
    for (path, snapshot) in snapshots.iter().rev() {
        if let Err(error) = restore_file(path, snapshot) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn with_rollback<T>(
    snapshots: &[(PathBuf, Option<Vec<u8>>)],
    operation: impl FnOnce() -> Result<T, RefError>,
) -> Result<T, RefError> {
    match operation() {
        Ok(value) => Ok(value),
        Err(original) => match rollback_files(snapshots) {
            Ok(()) => Err(original),
            Err(rollback) => Err(RefError::RevParseError(format!(
                "{}; rollback also failed: {}",
                original, rollback
            ))),
        },
    }
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

        let resolve_symbolic = |name: &str| -> Result<Option<ObjectId>, RefError> {
            match self.read_ref(name) {
                Ok(oid) => Ok(Some(oid)),
                Err(RefError::NotFound(_)) => Ok(None),
                Err(error) => Err(error),
            }
        };

        if let Some(rest) = content.strip_prefix("ref: refs/heads/") {
            if rest.is_empty() {
                return Err(RefError::RevParseError(
                    "malformed HEAD: empty branch name".to_string(),
                ));
            }
            let branch = rest.to_string();
            let oid = resolve_symbolic(&format!("refs/heads/{}", branch))?;
            Ok((branch, oid))
        } else if let Some(rest) = content.strip_prefix("ref: ") {
            if rest.is_empty() {
                return Err(RefError::RevParseError(
                    "malformed HEAD: empty symbolic ref".to_string(),
                ));
            }
            let full_ref = rest.to_string();
            let oid = resolve_symbolic(&full_ref)?;
            Ok((full_ref, oid))
        } else {
            let oid = content.parse::<ObjectId>().map_err(|_| {
                RefError::RevParseError(format!("malformed HEAD contents: {content:?}"))
            })?;
            Ok(("HEAD".to_string(), Some(oid)))
        }
    }

    /// Reads a reference by full or short name, checking loose refs then `packed-refs`.
    pub fn read_ref(&self, ref_name: &str) -> Result<ObjectId, RefError> {
        self.read_ref_inner(ref_name, 0)
    }

    fn read_ref_inner(&self, ref_name: &str, depth: usize) -> Result<ObjectId, RefError> {
        if depth > 16 {
            return Err(RefError::RevParseError(format!(
                "symbolic reference cycle while resolving '{}'",
                ref_name
            )));
        }
        let normalized = self.normalize_ref_name(ref_name);

        let parse_loose = |content: &str| -> Result<ObjectId, RefError> {
            let content = content.trim();
            if let Some(target) = content.strip_prefix("ref: ") {
                if target.is_empty() {
                    return Err(RefError::RevParseError(format!(
                        "malformed symbolic reference '{}': empty target",
                        normalized
                    )));
                }
                return self.read_ref_inner(target, depth + 1);
            }
            content.parse::<ObjectId>().map_err(|_| {
                RefError::RevParseError(format!(
                    "malformed reference '{}': invalid object id",
                    normalized
                ))
            })
        };

        // 1. Check per-worktree loose ref file (e.g. for HEAD, or worktree-local refs)
        let loose_path = self.git_dir.join(&normalized);
        if loose_path.is_file() {
            let content = fs::read_to_string(loose_path)?;
            return parse_loose(&content);
        }

        // 2. Check common_dir loose ref file (for shared refs)
        if self.common_dir != self.git_dir {
            let common_path = self.common_dir.join(&normalized);
            if common_path.is_file() {
                let content = fs::read_to_string(common_path)?;
                return parse_loose(&content);
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
                let full_ref = format!("{}/{}", prefix, name);
                let oid = self.read_ref(&full_ref)?;
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

        let current_oid = match self.read_ref(&normalized) {
            Ok(oid) => Some(oid),
            Err(RefError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };

        if let Some(expected) = old_oid {
            if current_oid.as_ref() != Some(expected) {
                return Err(RefError::RevParseError(format!(
                    "ref {} does not match expected old value {}",
                    ref_name, expected
                )));
            }
        }

        let (head_name, _) = self.resolve_head()?;
        let is_active = normalized != "HEAD"
            && (head_name == ref_name || format!("refs/heads/{}", head_name) == normalized);
        let ref_log_path = self.common_dir.join("logs").join(&normalized);
        let head_log_path = self.git_dir.join("logs").join("HEAD");
        let mut snapshot_paths = vec![ref_path.clone(), ref_log_path];
        if is_active && !snapshot_paths.contains(&head_log_path) {
            snapshot_paths.push(head_log_path);
        }
        let snapshots = snapshot_paths
            .into_iter()
            .map(|path| snapshot_file(&path).map(|snapshot| (path, snapshot)))
            .collect::<Result<Vec<_>, _>>()?;

        let from_oid = current_oid.unwrap_or(ObjectId::ZERO);
        let sig = get_default_signature(Some(&self.git_dir));
        with_rollback(&snapshots, || {
            write_file_atomic(&ref_path, format!("{}\n", new_oid).as_bytes())?;
            self.append_reflog(&normalized, &from_oid, new_oid, &sig, message)?;
            if is_active {
                self.append_reflog("HEAD", &from_oid, new_oid, &sig, message)?;
            }
            Ok(())
        })
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
        let snapshots = vec![
            (stash_ref.clone(), snapshot_file(&stash_ref)?),
            (stash_log.clone(), snapshot_file(&stash_log)?),
        ];

        with_rollback(&snapshots, || {
            if entries.is_empty() {
                remove_regular_file_if_exists(&stash_ref)?;
                remove_regular_file_if_exists(&stash_log)?;
            } else {
                let mut log_data = Vec::new();
                let mut prev_oid = ObjectId::ZERO;
                for entry in &entries {
                    writeln!(
                        log_data,
                        "{} {} {}\t{}",
                        prev_oid, entry.new_oid, entry.signature, entry.message
                    )?;
                    prev_oid = entry.new_oid;
                }
                write_file_atomic(&stash_log, &log_data)?;
                let newest = entries.last().ok_or_else(|| {
                    RefError::RevParseError("stash reflog unexpectedly became empty".to_string())
                })?;
                write_file_atomic(&stash_ref, format!("{}\n", newest.new_oid).as_bytes())?;
            }
            Ok(())
        })?;

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

            let (sha, name) = line.split_once(' ').ok_or_else(|| {
                RefError::RevParseError(format!("malformed packed-ref line: '{}'", line))
            })?;
            let name = name.trim();
            if name.is_empty() {
                return Err(RefError::RevParseError(format!(
                    "malformed packed-ref line: '{}'",
                    line
                )));
            }
            let oid = sha.parse::<ObjectId>().map_err(|_| {
                RefError::RevParseError(format!("malformed packed-ref object id: '{}'", sha))
            })?;
            map.insert(name.to_string(), oid);
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
        let ref_path = self.common_dir.join(ref_name);
        let wt_ref_path = (self.git_dir != self.common_dir).then(|| self.git_dir.join(ref_name));
        let packed_path = self.common_dir.join("packed-refs");

        let mut snapshots = vec![(ref_path.clone(), snapshot_file(&ref_path)?)];
        if let Some(path) = &wt_ref_path {
            snapshots.push((path.clone(), snapshot_file(path)?));
        }
        snapshots.push((packed_path.clone(), snapshot_file(&packed_path)?));

        let loose_exists = snapshots
            .iter()
            .any(|(path, snapshot)| path != &packed_path && snapshot.is_some());
        let packed_content = snapshots
            .iter()
            .find(|(path, _)| path == &packed_path)
            .and_then(|(_, snapshot)| snapshot.as_ref())
            .map(|bytes| {
                std::str::from_utf8(bytes).map_err(|e| {
                    RefError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                })
            })
            .transpose()?;

        let mut packed_rewrite: Option<Vec<u8>> = None;
        let mut packed_contains_ref = false;
        if let Some(content) = packed_content {
            let mut new_lines = Vec::new();
            let mut skip_next_peeled = false;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('^') {
                    if skip_next_peeled {
                        skip_next_peeled = false;
                    } else {
                        new_lines.push(line.to_string());
                    }
                    continue;
                }
                skip_next_peeled = false;

                if trimmed.is_empty() || trimmed.starts_with('#') {
                    new_lines.push(line.to_string());
                    continue;
                }

                let (_sha, rname) = trimmed.split_once(' ').ok_or_else(|| {
                    RefError::RevParseError(format!("malformed packed-ref line: '{}'", line))
                })?;
                if rname.trim() == ref_name {
                    packed_contains_ref = true;
                    skip_next_peeled = true;
                    continue;
                }
                new_lines.push(line.to_string());
            }

            if packed_contains_ref {
                let mut rewritten = new_lines.join("\n").into_bytes();
                if !rewritten.is_empty() {
                    rewritten.push(b'\n');
                }
                packed_rewrite = Some(rewritten);
            }
        }

        if !loose_exists && !packed_contains_ref {
            return Err(RefError::NotFound(format!("ref '{}' not found", ref_name)));
        }

        with_rollback(&snapshots, || {
            remove_regular_file_if_exists(&ref_path)?;
            if let Some(path) = &wt_ref_path {
                remove_regular_file_if_exists(path)?;
            }
            if let Some(rewritten) = &packed_rewrite {
                write_file_atomic(&packed_path, rewritten)?;
            }
            Ok(())
        })
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
        oxidize_core::validate_branch_name(old_name)?;
        oxidize_core::validate_branch_name(new_name)?;
        if old_name == new_name {
            return Ok(());
        }
        let old_ref = format!("refs/heads/{}", old_name);
        let new_ref = format!("refs/heads/{}", new_name);
        let target_oid = self.read_ref(&old_ref).map_err(|error| match error {
            RefError::NotFound(_) => RefError::NotFound(format!("branch '{}' not found", old_name)),
            other => other,
        })?;
        match self.read_ref(&new_ref) {
            Ok(_) => {
                return Err(RefError::InvalidName(format!(
                    "a branch named '{}' already exists",
                    new_name
                )))
            }
            Err(RefError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }

        let (current_head_branch, _) = self.resolve_head()?;
        let is_current = current_head_branch == old_name;
        let old_ref_path = self.common_dir.join(&old_ref);
        let new_ref_path = self.common_dir.join(&new_ref);
        let head_path = self.git_dir.join("HEAD");
        let old_log = self.common_dir.join("logs").join(&old_ref);
        let new_log = self.common_dir.join("logs").join(&new_ref);
        let head_log = self.git_dir.join("logs").join("HEAD");
        let packed_refs = self.common_dir.join("packed-refs");

        let mut paths = vec![
            old_ref_path,
            new_ref_path,
            old_log.clone(),
            new_log.clone(),
            packed_refs,
        ];
        if is_current {
            paths.push(head_path);
            paths.push(head_log);
        }
        let snapshots = paths
            .into_iter()
            .map(|path| snapshot_file(&path).map(|snapshot| (path, snapshot)))
            .collect::<Result<Vec<_>, _>>()?;
        let sig = get_default_signature(Some(&self.git_dir));
        let log_msg = format!(
            "Branch: renamed refs/heads/{} to refs/heads/{}",
            old_name, new_name
        );

        with_rollback(&snapshots, || {
            self.create_branch(new_name, &target_oid)?;
            if is_current {
                self.set_head_symbolic(new_name)?;
            }
            if old_log.exists() {
                if new_log.exists() {
                    return Err(RefError::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("reflog '{}' already exists", new_log.display()),
                    )));
                }
                if let Some(parent) = new_log.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&old_log, &new_log)?;
            }
            self.delete_branch(old_name)?;
            self.append_reflog(&new_ref, &target_oid, &target_oid, &sig, &log_msg)?;
            if is_current {
                self.append_reflog("HEAD", &target_oid, &target_oid, &sig, &log_msg)?;
            }
            Ok(())
        })
    }

    /// Atomically renames all tracking refs under `refs/remotes/<old>/`.
    pub fn rename_remote_refs(&self, old_name: &str, new_name: &str) -> Result<(), RefError> {
        if old_name.is_empty() || new_name.is_empty() {
            return Err(RefError::InvalidName(
                "remote name cannot be empty".to_string(),
            ));
        }
        oxidize_core::validate_ref_name(&format!("refs/remotes/{old_name}/probe"))
            .map_err(|e| RefError::InvalidName(e.to_string()))?;
        oxidize_core::validate_ref_name(&format!("refs/remotes/{new_name}/probe"))
            .map_err(|e| RefError::InvalidName(e.to_string()))?;
        if old_name == new_name {
            return Ok(());
        }

        let remotes = self.list_remotes()?;
        let old_prefix = format!("{old_name}/");
        let new_prefix = format!("{new_name}/");
        let moving: Vec<(String, String, ObjectId)> = remotes
            .iter()
            .filter_map(|(name, oid)| {
                name.strip_prefix(&old_prefix).map(|rest| {
                    (
                        format!("refs/remotes/{name}"),
                        format!("refs/remotes/{new_prefix}{rest}"),
                        *oid,
                    )
                })
            })
            .collect();
        for (_, new_ref, _) in &moving {
            match self.read_ref(new_ref) {
                Ok(_) => {
                    return Err(RefError::InvalidName(format!(
                        "tracking reference '{}' already exists",
                        new_ref
                    )))
                }
                Err(RefError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }

        let mut paths = vec![self.common_dir.join("packed-refs")];
        for (old_ref, new_ref, _) in &moving {
            paths.push(self.common_dir.join(old_ref));
            paths.push(self.common_dir.join(new_ref));
            paths.push(self.common_dir.join("logs").join(old_ref));
            paths.push(self.common_dir.join("logs").join(new_ref));
        }
        paths.sort();
        paths.dedup();
        let snapshots = paths
            .into_iter()
            .map(|path| snapshot_file(&path).map(|snapshot| (path, snapshot)))
            .collect::<Result<Vec<_>, _>>()?;

        with_rollback(&snapshots, || {
            for (_, new_ref, oid) in &moving {
                self.update_ref(new_ref, oid, None, "remote: rename tracking ref")?;
            }
            for (old_ref, _, _) in &moving {
                self.delete_ref(old_ref)?;
                remove_regular_file_if_exists(&self.common_dir.join("logs").join(old_ref))?;
            }
            Ok(())
        })
    }

    /// Atomically removes all tracking refs under `refs/remotes/<name>/`.
    pub fn remove_remote_refs(&self, name: &str) -> Result<(), RefError> {
        if name.is_empty() {
            return Err(RefError::InvalidName(
                "remote name cannot be empty".to_string(),
            ));
        }
        oxidize_core::validate_ref_name(&format!("refs/remotes/{name}/probe"))
            .map_err(|e| RefError::InvalidName(e.to_string()))?;
        let remotes = self.list_remotes()?;
        let prefix = format!("{name}/");
        let refs: Vec<String> = remotes
            .keys()
            .filter(|remote| remote.starts_with(&prefix))
            .map(|remote| format!("refs/remotes/{remote}"))
            .collect();
        let mut paths = vec![self.common_dir.join("packed-refs")];
        for ref_name in &refs {
            paths.push(self.common_dir.join(ref_name));
            paths.push(self.common_dir.join("logs").join(ref_name));
        }
        paths.sort();
        paths.dedup();
        let snapshots = paths
            .into_iter()
            .map(|path| snapshot_file(&path).map(|snapshot| (path, snapshot)))
            .collect::<Result<Vec<_>, _>>()?;
        with_rollback(&snapshots, || {
            for ref_name in &refs {
                self.delete_ref(ref_name)?;
                remove_regular_file_if_exists(&self.common_dir.join("logs").join(ref_name))?;
            }
            Ok(())
        })
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
