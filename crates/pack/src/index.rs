//! Git Pack Index v2 format (.idx) reader and writer.

use crate::PackError;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use oxidize_core::id::ObjectId;
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// An entry indexed in a packfile index v2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedObject {
    /// Object ID (SHA-1).
    pub oid: ObjectId,
    /// Offset in the .pack file.
    pub offset: u64,
    /// CRC32 checksum of the packed object data.
    pub crc32: u32,
}

/// A parsed packfile index v2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackIndex {
    /// Total number of objects in the pack.
    pub objects: Vec<IndexedObject>,
    /// Checksum of the associated .pack file.
    pub pack_checksum: ObjectId,
    /// Checksum of this .idx file.
    pub idx_checksum: ObjectId,
}

impl PackIndex {
    /// Parses an `.idx` file from disk.
    pub fn read_from(path: impl AsRef<Path>) -> Result<Self, PackError> {
        let bytes = std::fs::read(path)?;
        if bytes.len() < 4 + 4 + 256 * 4 + 20 + 20 {
            return Err(PackError::InvalidIndexSignature);
        }

        // Verify trailing SHA-1 checksum
        let content_len = bytes.len() - 20;
        let mut hasher = Sha1::new();
        hasher.update(&bytes[..content_len]);
        let calculated_idx_hash: [u8; 20] = hasher.finalize().into();
        let idx_checksum = ObjectId::from_bytes(calculated_idx_hash);

        let mut stored_idx_bytes = [0u8; 20];
        stored_idx_bytes.copy_from_slice(&bytes[content_len..]);
        if stored_idx_bytes != calculated_idx_hash {
            return Err(PackError::ChecksumMismatch);
        }

        let mut cursor = &bytes[..content_len];

        // 1. Magic \xFFtOc
        let mut magic = [0u8; 4];
        cursor.read_exact(&mut magic)?;
        if magic != [0xFF, b't', b'O', b'c'] {
            return Err(PackError::InvalidIndexSignature);
        }

        // 2. Version 2
        let version = cursor.read_u32::<BigEndian>()?;
        if version != 2 {
            return Err(PackError::UnsupportedVersion(version));
        }

        // 3. Fanout table (256 entries)
        let mut fanout = [0u32; 256];
        for entry in &mut fanout {
            *entry = cursor.read_u32::<BigEndian>()?;
        }
        let total_objects = fanout[255] as usize;

        // 4. SHA-1 table (N * 20 bytes)
        let mut oids = Vec::with_capacity(total_objects);
        for _ in 0..total_objects {
            let mut oid_bytes = [0u8; 20];
            cursor.read_exact(&mut oid_bytes)?;
            oids.push(ObjectId::from_bytes(oid_bytes));
        }

        // 5. CRC32 table (N * 4 bytes)
        let mut crc32s = Vec::with_capacity(total_objects);
        for _ in 0..total_objects {
            crc32s.push(cursor.read_u32::<BigEndian>()?);
        }

        // 6. Offset table (N * 4 bytes)
        let mut raw_offsets = Vec::with_capacity(total_objects);
        for _ in 0..total_objects {
            raw_offsets.push(cursor.read_u32::<BigEndian>()?);
        }

        // 7. Large 8-byte offset table (if any MSB set)
        let mut objects = Vec::with_capacity(total_objects);
        let mut large_offsets = Vec::new();

        // Check if any MSB set
        let has_large = raw_offsets.iter().any(|&off| (off & 0x8000_0000) != 0);
        if has_large {
            // Read remaining bytes before 20-byte pack checksum
            let num_large = (cursor.len() - 20) / 8;
            for _ in 0..num_large {
                large_offsets.push(cursor.read_u64::<BigEndian>()?);
            }
        }

        for i in 0..total_objects {
            let raw_off = raw_offsets[i];
            let offset = if (raw_off & 0x8000_0000) != 0 {
                let large_idx = (raw_off & 0x7FFF_FFFF) as usize;
                if large_idx >= large_offsets.len() {
                    return Err(PackError::InvalidIndexSignature);
                }
                large_offsets[large_idx]
            } else {
                raw_off as u64
            };

            objects.push(IndexedObject {
                oid: oids[i],
                offset,
                crc32: crc32s[i],
            });
        }

        // 8. 20-byte packfile checksum
        let mut pack_checksum_bytes = [0u8; 20];
        cursor.read_exact(&mut pack_checksum_bytes)?;
        let pack_checksum = ObjectId::from_bytes(pack_checksum_bytes);

        Ok(Self {
            objects,
            pack_checksum,
            idx_checksum,
        })
    }

    /// Writes an index v2 file to disk.
    pub fn write_to(
        mut objects: Vec<IndexedObject>,
        pack_checksum: &ObjectId,
        path: impl AsRef<Path>,
    ) -> Result<ObjectId, PackError> {
        // Objects must be sorted lexicographically by ObjectId
        objects.sort_by_key(|o| o.oid);

        let mut payload = Vec::new();

        // 1. Magic and version
        payload.extend_from_slice(&[0xFF, b't', b'O', b'c']);
        payload.write_u32::<BigEndian>(2)?;

        // 2. Fanout table (256 entries)
        let mut fanout = [0u32; 256];
        for obj in &objects {
            let first_byte = obj.oid.as_bytes()[0] as usize;
            fanout[first_byte] += 1;
        }
        let mut running_sum = 0u32;
        for count in &mut fanout {
            running_sum += *count;
            *count = running_sum;
            payload.write_u32::<BigEndian>(*count)?;
        }

        // 3. SHA-1 table
        for obj in &objects {
            payload.extend_from_slice(obj.oid.as_bytes());
        }

        // 4. CRC32 table
        for obj in &objects {
            payload.write_u32::<BigEndian>(obj.crc32)?;
        }

        // 5. Offset table
        let mut large_offsets = Vec::new();
        for obj in &objects {
            if obj.offset > 0x7FFF_FFFF {
                let large_idx = large_offsets.len() as u32;
                payload.write_u32::<BigEndian>(0x8000_0000 | large_idx)?;
                large_offsets.push(obj.offset);
            } else {
                payload.write_u32::<BigEndian>(obj.offset as u32)?;
            }
        }

        // 6. Large offset table
        for offset in large_offsets {
            payload.write_u64::<BigEndian>(offset)?;
        }

        // 7. Packfile checksum
        payload.extend_from_slice(pack_checksum.as_bytes());

        // 8. Index checksum
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let idx_checksum: [u8; 20] = hasher.finalize().into();
        payload.extend_from_slice(&idx_checksum);

        let mut file = File::create(path)?;
        file.write_all(&payload)?;
        file.flush()?;

        Ok(ObjectId::from_bytes(idx_checksum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_pack_index_write_and_read_roundtrip() {
        let dir = tempdir().unwrap();
        let idx_path = dir.path().join("test.idx");

        let oid1: ObjectId = "1111111111111111111111111111111111111111".parse().unwrap();
        let oid2: ObjectId = "2222222222222222222222222222222222222222".parse().unwrap();
        let pack_checksum: ObjectId = "3333333333333333333333333333333333333333".parse().unwrap();

        let objects = vec![
            IndexedObject {
                oid: oid2,
                offset: 150,
                crc32: 0x12345678,
            },
            IndexedObject {
                oid: oid1,
                offset: 12,
                crc32: 0x87654321,
            },
        ];

        let idx_checksum = PackIndex::write_to(objects.clone(), &pack_checksum, &idx_path)
            .expect("index write succeeds");
        assert!(!idx_checksum.is_zero());

        let read_index = PackIndex::read_from(&idx_path).expect("index read succeeds");
        assert_eq!(read_index.pack_checksum, pack_checksum);
        assert_eq!(read_index.idx_checksum, idx_checksum);
        assert_eq!(read_index.objects.len(), 2);
        // Objects should be sorted by OID
        assert_eq!(read_index.objects[0].oid, oid1);
        assert_eq!(read_index.objects[0].offset, 12);
        assert_eq!(read_index.objects[0].crc32, 0x87654321);
        assert_eq!(read_index.objects[1].oid, oid2);
        assert_eq!(read_index.objects[1].offset, 150);
        assert_eq!(read_index.objects[1].crc32, 0x12345678);
    }
}
