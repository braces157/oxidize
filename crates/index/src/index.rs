//! Git Index (staging area) binary format v2 reader and writer.

use crate::entry::IndexEntry;
use crate::IndexError;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use sha1::{Digest, Sha1};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// Git index representation containing staged file entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Index {
    /// Index format version (typically 2).
    pub version: u32,
    /// Sorted list of staged entries.
    pub entries: Vec<IndexEntry>,
}

impl Index {
    /// Creates a new empty index with default format version 2.
    pub fn new() -> Self {
        Self {
            version: 2,
            entries: Vec::new(),
        }
    }

    /// Loads the index from `.git/index`. If the file does not exist, returns an empty index.
    pub fn load_from(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }

        let bytes = fs::read(path)?;
        if bytes.len() < 32 {
            // Header is 12 bytes + 20 bytes SHA-1 checksum = 32 bytes minimum
            return Err(IndexError::EntryParseError("index file too small".into()));
        }

        // Verify SHA-1 checksum of the entire index file
        let content_len = bytes.len() - 20;
        let mut hasher = Sha1::new();
        hasher.update(&bytes[..content_len]);
        let calculated_hash: [u8; 20] = hasher.finalize().into();

        if calculated_hash != bytes[content_len..] {
            return Err(IndexError::ChecksumMismatch);
        }

        let mut cursor = &bytes[..content_len];

        // 1. Header
        let mut signature = [0u8; 4];
        cursor.read_exact(&mut signature)?;
        if &signature != b"DIRC" {
            return Err(IndexError::InvalidSignature);
        }

        let version = cursor.read_u32::<BigEndian>()?;
        if version != 2 && version != 3 && version != 4 {
            return Err(IndexError::UnsupportedVersion(version));
        }

        let num_entries = cursor.read_u32::<BigEndian>()? as usize;
        let mut entries = Vec::with_capacity(num_entries);

        // 2. Entries
        if version == 4 {
            let mut prev_path = String::new();
            for _ in 0..num_entries {
                let entry = IndexEntry::read_v4_entry(&mut cursor, &prev_path)?;
                prev_path = entry.path.clone();
                entries.push(entry);
            }
        } else {
            for _ in 0..num_entries {
                let entry = IndexEntry::read_from(&mut cursor)?;
                entries.push(entry);
            }
        }

        // Optional extensions (we ignore them safely while advancing cursor)

        Ok(Self { version, entries })
    }

    /// Writes the index atomically to disk using an exclusive `.lock` file.
    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<(), IndexError> {
        let path = path.as_ref();
        let mut lock = oxidize_core::LockFile::acquire(path)?;

        let mut payload = Vec::new();

        // 1. Header
        payload.extend_from_slice(b"DIRC");
        payload.write_u32::<BigEndian>(self.version)?;
        payload.write_u32::<BigEndian>(self.entries.len() as u32)?;

        // 2. Entries (must be sorted by path, then stage)
        let mut sorted_entries = self.entries.clone();
        sorted_entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.stage.cmp(&b.stage)));

        if self.version == 4 {
            let mut prev_path = String::new();
            for entry in &sorted_entries {
                entry.write_v4_to(&mut payload, &prev_path)?;
                prev_path = entry.path.clone();
            }
        } else {
            for entry in &sorted_entries {
                entry.write_to(&mut payload)?;
            }
        }

        // 3. Trailing SHA-1 checksum
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let checksum: [u8; 20] = hasher.finalize().into();
        payload.extend_from_slice(&checksum);

        lock.write_all(&payload).map_err(IndexError::Io)?;
        lock.commit()?;

        Ok(())
    }

    /// Adds or updates an entry in the index, maintaining canonical ordering.
    /// Adding a stage 0 entry removes any unmerged stage entries (1, 2, 3) for that path.
    pub fn add_entry(&mut self, entry: IndexEntry) {
        if entry.stage == 0 {
            let start = self
                .entries
                .partition_point(|e| e.path.as_str() < entry.path.as_str());
            let count = self.entries[start..]
                .iter()
                .take_while(|e| e.path == entry.path)
                .count();
            if count > 0 {
                self.entries.drain(start..start + count);
            }
            self.entries.insert(start, entry);
        } else {
            let key = (entry.path.as_str(), entry.stage);
            match self
                .entries
                .binary_search_by(|e| (e.path.as_str(), e.stage).cmp(&key))
            {
                Ok(pos) => self.entries[pos] = entry,
                Err(pos) => self.entries.insert(pos, entry),
            }
        }
    }

    /// Adds or updates multiple entries efficiently in a single bulk sort/merge pass.
    pub fn add_entries(&mut self, mut new_entries: Vec<IndexEntry>) {
        if new_entries.is_empty() {
            return;
        }

        // Sort new entries canonically
        new_entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.stage.cmp(&b.stage)));

        // Dedup new_entries: if multiple for same (path, stage), keep last
        new_entries.dedup_by(|a, b| {
            if a.path == b.path && (a.stage == 0 || a.stage == b.stage) {
                *b = a.clone();
                true
            } else {
                false
            }
        });

        // Collect paths that have stage 0 in new_entries (these clear any stage 1..3 in old)
        let stage0_paths: std::collections::HashSet<String> = new_entries
            .iter()
            .filter(|e| e.stage == 0)
            .map(|e| e.path.clone())
            .collect();

        // Drain old entries that are replaced or superseded by stage 0
        let old_entries = std::mem::take(&mut self.entries);
        let mut merged = Vec::with_capacity(old_entries.len() + new_entries.len());

        let mut old_iter = old_entries.into_iter().peekable();
        let mut new_iter = new_entries.into_iter().peekable();

        while let (Some(old_e), Some(new_e)) = (old_iter.peek(), new_iter.peek()) {
            let old_key = (old_e.path.as_str(), old_e.stage);
            let new_key = (new_e.path.as_str(), new_e.stage);

            if stage0_paths.contains(old_e.path.as_str()) && old_e.stage != 0 {
                // Superseded unmerged entry
                old_iter.next();
                continue;
            }

            match old_key.cmp(&new_key) {
                std::cmp::Ordering::Less => {
                    merged.push(old_iter.next().unwrap());
                }
                std::cmp::Ordering::Greater => {
                    merged.push(new_iter.next().unwrap());
                }
                std::cmp::Ordering::Equal => {
                    // New entry overwrites old entry
                    merged.push(new_iter.next().unwrap());
                    old_iter.next();
                }
            }
        }

        for old_e in old_iter {
            if !(stage0_paths.contains(old_e.path.as_str()) && old_e.stage != 0) {
                merged.push(old_e);
            }
        }
        merged.extend(new_iter);

        self.entries = merged;
    }

    /// Removes an entry by repository-relative path.
    pub fn remove_entry(&mut self, path: &str) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|e| e.path != path);
        self.entries.len() != initial_len
    }

    /// Finds an entry by repository-relative path and stage 0.
    pub fn find_entry(&self, path: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.path == path && e.stage == 0)
    }

    /// Alias for `find_entry`.
    pub fn get_entry(&self, path: &str) -> Option<&IndexEntry> {
        self.find_entry(path)
    }

    /// Returns a slice of all staged entries.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidize_core::object::Object;

    #[test]
    fn test_index_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("index");

        let mut index = Index::new();
        index.add_entry(IndexEntry {
            ctime_sec: 100,
            ctime_nsec: 200,
            mtime_sec: 300,
            mtime_nsec: 400,
            dev: 1,
            ino: 2,
            mode: 0o100644,
            uid: 1000,
            gid: 1000,
            file_size: 12,
            oid: Object::Blob(oxidize_core::object::Blob::new(b"hello world\n".to_vec())).id(),
            stage: 0,
            assume_valid: false,
            path: "hello.txt".to_string(),
        });

        index.write_to(&index_path).unwrap();
        let loaded = Index::load_from(&index_path).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].path, "hello.txt");
    }

    #[test]
    fn test_index_v4_loading() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        let status = std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .status();
        if status.is_err() || !status.unwrap().success() {
            return;
        }

        std::fs::write(repo_path.join("file1.txt"), "first").unwrap();
        std::fs::write(repo_path.join("file2.txt"), "second").unwrap();

        let _ = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .status();

        let _ = std::process::Command::new("git")
            .args(["update-index", "--index-version", "4"])
            .current_dir(repo_path)
            .status();

        let index_path = repo_path.join(".git").join("index");
        let loaded = Index::load_from(&index_path).unwrap();
        assert_eq!(loaded.version, 4);
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].path, "file1.txt");
        assert_eq!(loaded.entries[1].path, "file2.txt");
    }

    #[test]
    fn test_bulk_add_entries_and_ordering() {
        use oxidize_core::ObjectId;
        let mut index = Index::new();
        let e1 = IndexEntry {
            ctime_sec: 0,
            ctime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode: 0o100644,
            uid: 0,
            gid: 0,
            file_size: 10,
            oid: ObjectId::from_bytes([1u8; 20]),
            assume_valid: false,
            path: "z_file.txt".to_string(),
            stage: 0,
        };
        let e2 = IndexEntry {
            path: "a_file.txt".to_string(),
            ..e1.clone()
        };
        let e3 = IndexEntry {
            path: "m_file.txt".to_string(),
            ..e1.clone()
        };

        index.add_entries(vec![e1, e2, e3]);
        assert_eq!(index.entries.len(), 3);
        assert_eq!(index.entries[0].path, "a_file.txt");
        assert_eq!(index.entries[1].path, "m_file.txt");
        assert_eq!(index.entries[2].path, "z_file.txt");

        // Adding an unmerged stage entry then stage 0 replaces unmerged
        let mut stage2 = index.entries[1].clone();
        stage2.stage = 2;
        let mut stage3 = index.entries[1].clone();
        stage3.stage = 3;
        index.add_entry(stage2);
        index.add_entry(stage3);
        assert_eq!(index.entries.len(), 5);

        let resolved = index.entries[0].clone();
        let mut resolved_m = index.entries[0].clone();
        resolved_m.path = "m_file.txt".to_string();
        index.add_entries(vec![resolved, resolved_m]);
        // All stage 1..3 for m_file.txt should be cleared
        let m_stages: Vec<u8> = index
            .entries
            .iter()
            .filter(|e| e.path == "m_file.txt")
            .map(|e| e.stage)
            .collect();
        assert_eq!(m_stages, vec![0]);
    }
}
