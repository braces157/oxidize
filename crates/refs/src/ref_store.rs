//! Git reference management, atomic ref updates, packed-refs, and rev-parse.

use crate::signature::get_default_signature;
use crate::RefError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{Object, Signature};
use oxidize_core::store::LooseObjectStore;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Access and manipulation of repository references.
#[derive(Debug, Clone)]
pub struct RefStore {
    git_dir: PathBuf,
}

impl RefStore {
    /// Creates a `RefStore` for the given `.git` directory.
    pub fn new(git_dir: impl Into<PathBuf>) -> Self {
        Self {
            git_dir: git_dir.into(),
        }
    }

    /// Returns the path to the `.git` directory.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
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

        // 1. Check loose ref file
        let loose_path = self.git_dir.join(&normalized);
        if loose_path.is_file() {
            let content = fs::read_to_string(loose_path)?;
            let content = content.trim();
            if let Ok(oid) = content.parse::<ObjectId>() {
                return Ok(oid);
            }
        }

        // 2. Check packed-refs
        let packed = self.read_packed_refs()?;
        if let Some(oid) = packed.get(&normalized) {
            return Ok(*oid);
        }

        Err(RefError::NotFound(ref_name.to_string()))
    }

    /// Lists all local branch names (`refs/heads/*`) and their target commit IDs.
    pub fn list_branches(&self) -> Result<BTreeMap<String, ObjectId>, RefError> {
        let mut branches = BTreeMap::new();

        // 1. Packed refs
        let packed = self.read_packed_refs()?;
        for (name, oid) in packed {
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                branches.insert(branch.to_string(), oid);
            }
        }

        // 2. Loose refs (override packed)
        let heads_dir = self.git_dir.join("refs/heads");
        if heads_dir.exists() {
            self.scan_refs_dir(&heads_dir, "refs/heads", &mut branches)?;
        }

        Ok(branches)
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
                        let branch_name = if let Some(stripped) = prefix.strip_prefix("refs/heads")
                        {
                            if stripped.is_empty() {
                                name
                            } else {
                                format!("{}/{}", stripped.trim_start_matches('/'), name)
                            }
                        } else {
                            name
                        };
                        out.insert(branch_name, oid);
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
        let current_oid = self.read_ref(&normalized).ok();

        if let Some(expected) = old_oid {
            if current_oid.as_ref() != Some(expected) {
                return Err(RefError::RevParseError(format!(
                    "ref {} does not match expected old value {}",
                    ref_name, expected
                )));
            }
        }

        let ref_path = self.git_dir.join(&normalized);
        let lock_path = PathBuf::from(format!("{}.lock", ref_path.display()));

        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent)?;
        }

        {
            let mut file = File::create(&lock_path)?;
            writeln!(file, "{}", new_oid)?;
            file.flush()?;
        }

        fs::rename(lock_path, &ref_path)?;

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
        let log_path = self.git_dir.join("logs").join(ref_name);
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

    /// Resolves a revision specifier (`HEAD`, `HEAD~2`, branch name, short/full SHA) to an `ObjectId`.
    pub fn resolve_rev(&self, rev: &str, store: &LooseObjectStore) -> Result<ObjectId, RefError> {
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

        // Try loose object prefix
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
        store: &LooseObjectStore,
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
        store: &LooseObjectStore,
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
        let packed_path = self.git_dir.join("packed-refs");
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
        let branch_path = self.git_dir.join("refs/heads").join(name);
        if branch_path.exists() {
            return Err(RefError::InvalidName(format!(
                "a branch named '{}' already exists",
                name
            )));
        }

        if let Some(parent) = branch_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&branch_path, format!("{}\n", target_oid))?;
        Ok(())
    }

    /// Deletes a branch reference.
    pub fn delete_branch(&self, name: &str) -> Result<(), RefError> {
        let branch_path = self.git_dir.join("refs/heads").join(name);
        if !branch_path.exists() {
            return Err(RefError::NotFound(format!("branch '{}' not found", name)));
        }

        fs::remove_file(branch_path)?;
        Ok(())
    }

    /// Sets `HEAD` to point symbolically to a branch.
    pub fn set_head_symbolic(&self, branch_name: &str) -> Result<(), RefError> {
        let head_path = self.git_dir.join("HEAD");
        fs::write(head_path, format!("ref: refs/heads/{}\n", branch_name))?;
        Ok(())
    }

    /// Sets `HEAD` to a detached commit OID.
    pub fn set_head_detached(&self, commit_oid: &ObjectId) -> Result<(), RefError> {
        let head_path = self.git_dir.join("HEAD");
        fs::write(head_path, format!("{}\n", commit_oid))?;
        Ok(())
    }

    /// Finds the Lowest Common Ancestor (merge base) between two commits.
    pub fn find_merge_base(
        &self,
        store: &LooseObjectStore,
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
}
