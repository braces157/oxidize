//! Git Index Entry representation for DIRC format v2.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use oxidize_core::id::ObjectId;
use oxidize_core::object::FileMode;
use std::io::{Read, Write};
use std::time::SystemTime;

/// An individual file entry within the Git index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Creation time seconds.
    pub ctime_sec: u32,
    /// Creation time nanoseconds fraction.
    pub ctime_nsec: u32,
    /// Modification time seconds.
    pub mtime_sec: u32,
    /// Modification time nanoseconds fraction.
    pub mtime_nsec: u32,
    /// Device number.
    pub dev: u32,
    /// Inode number.
    pub ino: u32,
    /// Mode (e.g. 0o100644 or 0o100755).
    pub mode: u32,
    /// User ID.
    pub uid: u32,
    /// Group ID.
    pub gid: u32,
    /// File size in bytes.
    pub file_size: u32,
    /// SHA-1 Object ID of the blob.
    pub oid: ObjectId,
    /// Stage number (0: normal, 1: ancestor/base, 2: ours, 3: theirs).
    pub stage: u8,
    /// Assume-unchanged flag.
    pub assume_valid: bool,
    /// Repository-relative path using forward slashes (e.g. `src/main.rs`).
    pub path: String,
}

impl IndexEntry {
    /// Creates a new `IndexEntry` from filesystem metadata and blob `ObjectId`.
    pub fn from_fs_metadata(
        path: String,
        oid: ObjectId,
        metadata: &std::fs::Metadata,
        stage: u8,
    ) -> Self {
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let duration = mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let mtime_sec = duration.as_secs() as u32;
        let mtime_nsec = duration.subsec_nanos();

        let ctime = metadata.created().unwrap_or(mtime);
        let c_duration = ctime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(duration);
        let ctime_sec = c_duration.as_secs() as u32;
        let ctime_nsec = c_duration.subsec_nanos();

        let file_size = metadata.len() as u32;

        #[cfg(unix)]
        let (dev, ino, uid, gid, mode) = {
            use std::os::unix::fs::MetadataExt;
            let mode = if metadata.permissions().readonly() {
                0o100644
            } else if metadata.mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
            (
                metadata.dev() as u32,
                metadata.ino() as u32,
                metadata.uid(),
                metadata.gid(),
                mode,
            )
        };

        #[cfg(not(unix))]
        let (dev, ino, uid, gid, mode) = {
            // Windows fallback defaults: Git for Windows uses mode 100644 for regular files
            (0, 0, 0, 0, FileMode::REGULAR.0)
        };

        Self {
            ctime_sec,
            ctime_nsec,
            mtime_sec,
            mtime_nsec,
            dev,
            ino,
            mode,
            uid,
            gid,
            file_size,
            oid,
            stage,
            assume_valid: false,
            path,
        }
    }

    /// Reads an `IndexEntry` from a binary reader.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let ctime_sec = reader.read_u32::<BigEndian>()?;
        let ctime_nsec = reader.read_u32::<BigEndian>()?;
        let mtime_sec = reader.read_u32::<BigEndian>()?;
        let mtime_nsec = reader.read_u32::<BigEndian>()?;
        let dev = reader.read_u32::<BigEndian>()?;
        let ino = reader.read_u32::<BigEndian>()?;
        let mode = reader.read_u32::<BigEndian>()?;
        let uid = reader.read_u32::<BigEndian>()?;
        let gid = reader.read_u32::<BigEndian>()?;
        let file_size = reader.read_u32::<BigEndian>()?;

        let mut oid_bytes = [0u8; 20];
        reader.read_exact(&mut oid_bytes)?;
        let oid = ObjectId::from_bytes(oid_bytes);

        let flags = reader.read_u16::<BigEndian>()?;
        let assume_valid = (flags & 0x8000) != 0;
        let stage = ((flags >> 12) & 0x03) as u8;
        let name_len = (flags & 0x0FFF) as usize;

        // Read NUL-terminated path string
        let mut path_bytes = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            reader.read_exact(&mut byte)?;
            if byte[0] == 0 {
                break;
            }
            path_bytes.push(byte[0]);
        }

        let path = String::from_utf8_lossy(&path_bytes).to_string();

        // Total entry length so far = 62 + path_bytes.len() + 1 (the NUL)
        let bytes_read = 62 + path_bytes.len() + 1;
        let pad = (8 - (bytes_read % 8)) % 8;
        for _ in 0..pad {
            reader.read_exact(&mut byte)?;
        }

        let _ = name_len; // name_len in flags is capped at 0xFFF

        Ok(Self {
            ctime_sec,
            ctime_nsec,
            mtime_sec,
            mtime_nsec,
            dev,
            ino,
            mode,
            uid,
            gid,
            file_size,
            oid,
            stage,
            assume_valid,
            path,
        })
    }

    /// Serializes the index entry into a writer adhering to Git index v2 binary format.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<usize, std::io::Error> {
        writer.write_u32::<BigEndian>(self.ctime_sec)?;
        writer.write_u32::<BigEndian>(self.ctime_nsec)?;
        writer.write_u32::<BigEndian>(self.mtime_sec)?;
        writer.write_u32::<BigEndian>(self.mtime_nsec)?;
        writer.write_u32::<BigEndian>(self.dev)?;
        writer.write_u32::<BigEndian>(self.ino)?;
        writer.write_u32::<BigEndian>(self.mode)?;
        writer.write_u32::<BigEndian>(self.uid)?;
        writer.write_u32::<BigEndian>(self.gid)?;
        writer.write_u32::<BigEndian>(self.file_size)?;
        writer.write_all(self.oid.as_bytes())?;

        let mut flags: u16 = 0;
        if self.assume_valid {
            flags |= 0x8000;
        }
        flags |= ((self.stage as u16) & 0x03) << 12;
        let path_bytes = self.path.as_bytes();
        let len_field = path_bytes.len().min(0x0FFF) as u16;
        flags |= len_field;
        writer.write_u16::<BigEndian>(flags)?;

        writer.write_all(path_bytes)?;

        // Git requires 1-8 NUL bytes as necessary to pad the entry to a multiple of 8 bytes
        let pad = 8 - ((62 + path_bytes.len()) % 8);
        let padding = vec![0u8; pad];
        writer.write_all(&padding)?;

        Ok(62 + path_bytes.len() + pad)
    }
}
