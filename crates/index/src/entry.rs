//! Git Index Entry representation for DIRC format v2.

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use oxidize_core::id::ObjectId;
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
    /// Creates a default `IndexEntry` for a given path, object ID, and mode.
    pub fn new(path: String, oid: ObjectId, mode: u32) -> Self {
        Self {
            ctime_sec: 0,
            ctime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size: 0,
            oid,
            stage: 0,
            assume_valid: false,
            path,
        }
    }

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
            let mode = if metadata.file_type().is_symlink() {
                0o120000
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
            let mode = if metadata.file_type().is_symlink() {
                0o120000
            } else {
                0o100644
            };
            (0, 0, 0, 0, mode)
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
        let extended = (flags & 0x4000) != 0;
        let stage = ((flags >> 12) & 0x03) as u8;
        let name_len = (flags & 0x0FFF) as usize;

        let mut fixed_len = 62;
        if extended {
            let _ext_flags = reader.read_u16::<BigEndian>()?;
            fixed_len += 2;
        }

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

        // Total entry length so far = fixed_len + path_bytes.len() + 1 (the NUL)
        let bytes_read = fixed_len + path_bytes.len() + 1;
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

    /// Reads an `IndexEntry` from an Index Version 4 binary reader using path prefix compression.
    pub fn read_v4_entry<R: Read>(reader: &mut R, prev_path: &str) -> Result<Self, std::io::Error> {
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
        let extended = (flags & 0x4000) != 0;
        let stage = ((flags >> 12) & 0x03) as u8;

        if extended {
            let _ext_flags = reader.read_u16::<BigEndian>()?;
        }

        // Decode varint: number of bytes to strip from the end of prev_path
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        let mut c = byte[0];
        let mut strip_count = (c & 127) as usize;
        while (c & 128) != 0 {
            strip_count = strip_count.checked_add(1).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "varint overflow")
            })?;
            reader.read_exact(&mut byte)?;
            c = byte[0];
            strip_count = strip_count
                .checked_shl(7)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "varint overflow")
                })?
                .checked_add((c & 127) as usize)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "varint overflow")
                })?;
        }

        // Read NUL-terminated suffix string
        let mut suffix_bytes = Vec::new();
        loop {
            reader.read_exact(&mut byte)?;
            if byte[0] == 0 {
                break;
            }
            suffix_bytes.push(byte[0]);
        }

        let suffix = String::from_utf8_lossy(&suffix_bytes);
        let prefix_len = prev_path.len().saturating_sub(strip_count);
        let path = format!("{}{}", &prev_path[..prefix_len], suffix);

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

    /// Serializes the index entry for Index Version 4 format using path prefix compression.
    pub fn write_v4_to<W: Write>(
        &self,
        writer: &mut W,
        prev_path: &str,
    ) -> Result<usize, std::io::Error> {
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

        // Find common prefix with prev_path
        let common_prefix_len = prev_path
            .as_bytes()
            .iter()
            .zip(path_bytes)
            .take_while(|(a, b)| a == b)
            .count();
        let strip_count = prev_path.len() - common_prefix_len;
        let suffix = &self.path[common_prefix_len..];

        let varint = encode_varint(strip_count);
        writer.write_all(&varint)?;
        writer.write_all(suffix.as_bytes())?;
        writer.write_all(&[0u8])?;

        Ok(62 + varint.len() + suffix.len() + 1)
    }
}

/// Encodes an unsigned integer using Git's variable-length integer encoding.
pub fn encode_varint(mut value: usize) -> Vec<u8> {
    let mut buf = [0u8; 16];
    let mut pos = buf.len() - 1;
    buf[pos] = (value & 127) as u8;
    while value >= 128 {
        value = (value >> 7) - 1;
        pos -= 1;
        buf[pos] = (128 | (value & 127)) as u8;
    }
    buf[pos..].to_vec()
}
