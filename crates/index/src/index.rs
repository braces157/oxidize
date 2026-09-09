//! Git Index (staging area) binary format v2 reader and writer.

use crate::entry::IndexEntry;
use crate::IndexError;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use sha1::{Digest, Sha1};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
        if version != 2 && version != 3 {
            return Err(IndexError::UnsupportedVersion(version));
        }

        let num_entries = cursor.read_u32::<BigEndian>()? as usize;
        let mut entries = Vec::with_capacity(num_entries);

        // 2. Entries
        for _ in 0..num_entries {
            let entry = IndexEntry::read_from(&mut cursor)?;
            entries.push(entry);
        }

        // Optional extensions (we ignore them safely while advancing cursor)

        Ok(Self { version, entries })
    }

    /// Writes the index atomically to disk using a `.lock` file.
    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<(), IndexError> {
        let path = path.as_ref();
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));

        let mut payload = Vec::new();

        // 1. Header
        payload.extend_from_slice(b"DIRC");
        payload.write_u32::<BigEndian>(self.version)?;
        payload.write_u32::<BigEndian>(self.entries.len() as u32)?;

        // 2. Entries (must be sorted by path, then stage)
        let mut sorted_entries = self.entries.clone();
        sorted_entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.stage.cmp(&b.stage)));

        for entry in &sorted_entries {
            entry.write_to(&mut payload)?;
        }

        // 3. Trailing SHA-1 checksum
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let checksum: [u8; 20] = hasher.finalize().into();
        payload.extend_from_slice(&checksum);

        // Write to lock file and rename atomically
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        {
            let mut file = File::create(&lock_path)?;
            file.write_all(&payload)?;
            file.flush()?;
        }

        fs::rename(lock_path, path)?;

        Ok(())
    }

    /// Adds or updates an entry in the index, maintaining canonical ordering.
    pub fn add_entry(&mut self, entry: IndexEntry) {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.path == entry.path && e.stage == entry.stage)
        {
            self.entries[pos] = entry;
        } else {
            self.entries.push(entry);
            self.entries
                .sort_by(|a, b| a.path.cmp(&b.path).then(a.stage.cmp(&b.stage)));
        }
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

    /// Returns a slice of all staged entries.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}
