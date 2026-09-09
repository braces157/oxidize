//! Object storage combining Loose objects and Packfile archives.

use crate::index::PackIndex;
use crate::packfile::{read_pack_object_at, RawPackObject};
use crate::PackError;
use oxidize_core::error::CoreError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{Object, ObjectType};
use oxidize_core::store::{parse_object_from_content, LooseObjectStore};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// A memory-mapped packfile and its associated index.
pub struct PackHandle {
    pack_path: PathBuf,
    idx_path: PathBuf,
    index: PackIndex,
    mmap: memmap2::Mmap,
}

impl PackHandle {
    /// Opens and memory-maps a `.pack` and its `.idx` counterpart.
    pub fn open(idx_path: impl AsRef<Path>) -> Result<Self, PackError> {
        let idx_path = idx_path.as_ref().to_path_buf();
        let pack_path = idx_path.with_extension("pack");
        if !pack_path.exists() {
            return Err(PackError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("missing packfile for idx: {}", idx_path.display()),
            )));
        }

        let index = PackIndex::read_from(&idx_path)?;
        let file = File::open(&pack_path)?;
        // SAFETY: The packfile is opened read-only and is immutable while mapped.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        Ok(Self {
            pack_path,
            idx_path,
            index,
            mmap,
        })
    }

    /// Path to the `.pack` file.
    pub fn pack_path(&self) -> &Path {
        &self.pack_path
    }

    /// Path to the `.idx` file.
    pub fn idx_path(&self) -> &Path {
        &self.idx_path
    }

    /// Reference to the parsed index.
    pub fn index(&self) -> &PackIndex {
        &self.index
    }

    /// Slice of the memory-mapped packfile.
    pub fn data(&self) -> &[u8] {
        &self.mmap
    }

    /// Checks if this pack contains the given object ID.
    pub fn contains(&self, oid: &ObjectId) -> bool {
        self.index
            .objects
            .binary_search_by_key(oid, |o| o.oid)
            .is_ok()
    }

    /// Retrieves an object by its ID from this packfile.
    pub fn get_object(&self, oid: &ObjectId) -> Result<Option<(ObjectType, Vec<u8>)>, PackError> {
        if let Ok(idx) = self.index.objects.binary_search_by_key(oid, |o| o.oid) {
            let item = &self.index.objects[idx];
            let (obj_type, data, _, _) = read_pack_object_at(&self.mmap, item.offset, None)?;
            Ok(Some((obj_type, data)))
        } else {
            Ok(None)
        }
    }
}

/// Manages all packfiles within a Git repository's `.git/objects/pack/` directory.
pub struct PackStore {
    handles: Vec<PackHandle>,
}

impl PackStore {
    /// Opens all packfiles found in the repository's `.git/objects/pack/` directory.
    pub fn open(git_dir: impl AsRef<Path>) -> Result<Self, PackError> {
        let pack_dir = git_dir.as_ref().join("objects").join("pack");
        let mut handles = Vec::new();

        if pack_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(pack_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("idx") {
                        if let Ok(handle) = PackHandle::open(&path) {
                            handles.push(handle);
                        }
                    }
                }
            }
        }

        Ok(Self { handles })
    }

    /// Returns the opened pack handles.
    pub fn handles(&self) -> &[PackHandle] {
        &self.handles
    }

    /// Checks if any packfile contains the given object.
    pub fn contains(&self, oid: &ObjectId) -> bool {
        self.handles.iter().any(|h| h.contains(oid))
    }

    /// Retrieves an object from any loaded packfile.
    pub fn get_object(&self, oid: &ObjectId) -> Result<Option<(ObjectType, Vec<u8>)>, PackError> {
        for handle in &self.handles {
            if let Some(res) = handle.get_object(oid)? {
                return Ok(Some(res));
            }
        }
        Ok(None)
    }

    /// Lists all object IDs across all packfiles.
    pub fn list_objects(&self) -> Vec<ObjectId> {
        let mut oids = Vec::new();
        for handle in &self.handles {
            for obj in &handle.index.objects {
                oids.push(obj.oid);
            }
        }
        oids.sort();
        oids.dedup();
        oids
    }
}

/// Unified repository object store checking both loose objects and packfile archives.
pub struct RepoObjectStore {
    loose: LooseObjectStore,
    pack: PackStore,
}

impl RepoObjectStore {
    /// Opens the unified store for a given `.git` directory.
    pub fn open(git_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        let git_dir = git_dir.as_ref();
        let loose = LooseObjectStore::new(git_dir.join("objects"));
        let pack = PackStore::open(git_dir)
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))?;

        Ok(Self { loose, pack })
    }

    /// Reference to the loose object store.
    pub fn loose(&self) -> &LooseObjectStore {
        &self.loose
    }

    /// Reference to the pack object store.
    pub fn pack(&self) -> &PackStore {
        &self.pack
    }

    /// Checks if an object exists in either loose or pack storage.
    pub fn exists(&self, oid: &ObjectId) -> bool {
        self.loose.exists(oid) || self.pack.contains(oid)
    }

    /// Reads an object by its ID from loose storage or packfiles.
    pub fn read_object(&self, oid: &ObjectId) -> Result<Object, CoreError> {
        if self.loose.exists(oid) {
            return self.loose.read_object(oid);
        }

        if let Ok(Some((obj_type, data))) = self.pack.get_object(oid) {
            return parse_object_from_content(obj_type, &data);
        }

        Err(CoreError::ObjectNotFound(oid.to_string()))
    }

    /// Reads raw object payload and type.
    pub fn read_raw(&self, oid: &ObjectId) -> Result<(ObjectType, Vec<u8>), CoreError> {
        if self.loose.exists(oid) {
            let (obj_type, data) = self.loose.read_raw(oid)?;
            return Ok((obj_type, data));
        }

        if let Ok(Some((obj_type, data))) = self.pack.get_object(oid) {
            return Ok((obj_type, data));
        }

        Err(CoreError::ObjectNotFound(oid.to_string()))
    }

    /// Resolves an unambiguous hex prefix across both loose and packed objects.
    pub fn find_by_prefix(&self, prefix: &str) -> Result<ObjectId, CoreError> {
        let mut candidates = Vec::new();

        if let Ok(oid) = self.loose.find_by_prefix(prefix) {
            candidates.push(oid);
        }

        for handle in self.pack.handles() {
            for obj in &handle.index.objects {
                if obj.oid.starts_with(prefix) && !candidates.contains(&obj.oid) {
                    candidates.push(obj.oid);
                }
            }
        }

        match candidates.len() {
            0 => Err(CoreError::ObjectNotFound(prefix.to_string())),
            1 => Ok(candidates[0]),
            _ => Err(CoreError::AmbiguousPrefix(prefix.to_string())),
        }
    }

    /// Writes an object as a loose object into `.git/objects/`.
    pub fn write_object(&self, obj: &Object) -> Result<ObjectId, CoreError> {
        self.loose.write_object(obj)
    }

    /// Convenience helper to create and store a Blob from byte slice.
    pub fn write_blob(&self, data: &[u8]) -> Result<ObjectId, CoreError> {
        self.loose.write_blob(data)
    }
}

impl oxidize_core::ObjectReader for RepoObjectStore {
    fn read_object(&self, id: &ObjectId) -> Result<Object, CoreError> {
        self.read_object(id)
    }
}

impl RepoObjectStore {
    /// Collects all objects (loose and packed) in the repository.
    pub fn collect_all_objects(&self) -> Result<Vec<RawPackObject>, CoreError> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();

        // 1. Loose objects
        if self.loose.root().is_dir() {
            for entry in fs::read_dir(self.loose.root())? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    if dir_name.len() == 2 && dir_name != "in" && dir_name != "pa" {
                        for sub in fs::read_dir(&path)? {
                            let sub = sub?;
                            let file_name = sub.file_name().to_string_lossy().to_string();
                            let full_hex = format!("{}{}", dir_name, file_name);
                            if let Ok(oid) = full_hex.parse::<ObjectId>() {
                                if seen.insert(oid) {
                                    if let Ok((obj_type, data)) = self.loose.read_raw(&oid) {
                                        result.push(RawPackObject::new(oid, obj_type, data));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Packed objects
        for handle in self.pack.handles() {
            for obj in &handle.index.objects {
                if seen.insert(obj.oid) {
                    if let Ok(Some((obj_type, data))) = handle.get_object(&obj.oid) {
                        result.push(RawPackObject::new(obj.oid, obj_type, data));
                    }
                }
            }
        }

        Ok(result)
    }
}
