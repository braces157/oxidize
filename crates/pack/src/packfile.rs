//! Git Packfile v2 format encoder, decoder, and delta resolver.

use crate::delta::apply_delta;
use crate::index::IndexedObject;
use crate::PackError;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use oxidize_core::id::ObjectId;
use oxidize_core::object::ObjectType;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::Write;

/// Git Packfile object type constants.
pub const OBJ_COMMIT: u8 = 1;
pub const OBJ_TREE: u8 = 2;
pub const OBJ_BLOB: u8 = 3;
pub const OBJ_TAG: u8 = 4;
pub const OBJ_OFS_DELTA: u8 = 6;
pub const OBJ_REF_DELTA: u8 = 7;

/// A raw Git object to be packed or returned from unpacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPackObject {
    /// Object ID (SHA-1).
    pub oid: ObjectId,
    /// Object type.
    pub obj_type: ObjectType,
    /// Uncompressed object data.
    pub data: Vec<u8>,
}

impl RawPackObject {
    /// Creates a new `RawPackObject`.
    pub fn new(oid: ObjectId, obj_type: ObjectType, data: Vec<u8>) -> Self {
        Self {
            oid,
            obj_type,
            data,
        }
    }
}

/// Converts `ObjectType` to Git packfile type code.
pub fn type_to_code(obj_type: ObjectType) -> u8 {
    match obj_type {
        ObjectType::Commit => OBJ_COMMIT,
        ObjectType::Tree => OBJ_TREE,
        ObjectType::Blob => OBJ_BLOB,
        ObjectType::Tag => OBJ_TAG,
    }
}

/// Converts Git packfile type code to `ObjectType`.
pub fn code_to_type(code: u8) -> Result<ObjectType, PackError> {
    match code {
        OBJ_COMMIT => Ok(ObjectType::Commit),
        OBJ_TREE => Ok(ObjectType::Tree),
        OBJ_BLOB => Ok(ObjectType::Blob),
        OBJ_TAG => Ok(ObjectType::Tag),
        _ => Err(PackError::InvalidPackSignature),
    }
}

/// Encodes the variable-length Git object header (type + uncompressed size).
pub fn encode_object_header(obj_type: u8, mut size: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut byte0 = ((obj_type & 0x07) << 4) | ((size & 0x0F) as u8);
    size >>= 4;
    if size > 0 {
        byte0 |= 0x80;
    }
    buf.push(byte0);

    while size > 0 {
        let mut byte = (size & 0x7F) as u8;
        size >>= 7;
        if size > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
    }
    buf
}

/// Decodes the variable-length Git object header from a byte slice.
/// Returns `(obj_type, uncompressed_size, header_bytes_consumed)`.
pub fn decode_object_header(
    data: &[u8],
    mut cursor: usize,
) -> Result<(u8, usize, usize), PackError> {
    if cursor >= data.len() {
        return Err(PackError::InvalidPackSignature);
    }
    let start = cursor;
    let byte0 = data[cursor];
    cursor += 1;

    let obj_type = (byte0 >> 4) & 0x07;
    let mut size = (byte0 & 0x0F) as usize;
    let mut shift = 4;
    let mut c = byte0;

    while (c & 0x80) != 0 {
        if cursor >= data.len() {
            return Err(PackError::InvalidPackSignature);
        }
        c = data[cursor];
        cursor += 1;
        size |= ((c & 0x7F) as usize)
            .checked_shl(shift)
            .ok_or_else(|| PackError::DeltaError("object size overflow".to_string()))?;
        shift += 7;
    }

    Ok((obj_type, size, cursor - start))
}

/// Encodes an offset delta using Git's variable-length negative offset encoding.
pub fn encode_offset_delta(mut ofs: u64) -> Vec<u8> {
    let mut buf = [0u8; 16];
    let mut pos = buf.len() - 1;
    buf[pos] = (ofs & 127) as u8;
    ofs >>= 7;
    while ofs > 0 {
        ofs -= 1;
        pos -= 1;
        buf[pos] = (128 | (ofs & 127)) as u8;
        ofs >>= 7;
    }
    buf[pos..].to_vec()
}

/// Decodes an offset delta from a byte slice.
/// Returns `(negative_offset, bytes_consumed)`.
pub fn decode_offset_delta(data: &[u8], mut cursor: usize) -> Result<(u64, usize), PackError> {
    if cursor >= data.len() {
        return Err(PackError::InvalidPackSignature);
    }
    let start = cursor;
    let mut c = data[cursor];
    cursor += 1;
    let mut ofs = (c & 127) as u64;

    while (c & 128) != 0 {
        if cursor >= data.len() {
            return Err(PackError::InvalidPackSignature);
        }
        ofs = ofs.checked_add(1).ok_or(PackError::InvalidPackSignature)?;
        c = data[cursor];
        cursor += 1;
        ofs = ofs
            .checked_shl(7)
            .ok_or(PackError::InvalidPackSignature)?
            .checked_add((c & 127) as u64)
            .ok_or(PackError::InvalidPackSignature)?;
    }

    Ok((ofs, cursor - start))
}

use rayon::prelude::*;

enum PreparedPayload {
    Base {
        uncompressed_len: usize,
        compressed: Vec<u8>,
    },
    Delta {
        base_index: usize,
        delta_len: usize,
        compressed: Vec<u8>,
    },
}

/// Writes a list of raw objects into a complete Git Packfile v2.
/// Uses Rayon to parallelize delta compression and zlib encoding across threads.
/// Returns `(pack_bytes, indexed_objects, pack_checksum)`.
pub fn write_pack(
    objects: &[RawPackObject],
    enable_deltas: bool,
) -> Result<(Vec<u8>, Vec<IndexedObject>, ObjectId), PackError> {
    if objects.is_empty() {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"PACK");
        payload.write_u32::<BigEndian>(2)?;
        payload.write_u32::<BigEndian>(0)?;
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let pack_checksum = ObjectId::from_bytes(hasher.finalize().into());
        payload.extend_from_slice(pack_checksum.as_bytes());
        return Ok((payload, Vec::new(), pack_checksum));
    }

    // 1. Parallelize delta searching and zlib compression across threads using Rayon
    let prepared: Vec<Result<PreparedPayload, PackError>> = (0..objects.len())
        .into_par_iter()
        .map(|i| {
            let obj = &objects[i];
            if enable_deltas && i > 0 && !obj.data.is_empty() {
                let window_start = i.saturating_sub(10);
                let mut best_base: Option<(usize, Vec<u8>)> = None;

                for (b, base_obj) in objects.iter().enumerate().take(i).skip(window_start) {
                    if base_obj.obj_type == obj.obj_type && !base_obj.data.is_empty() {
                        let delta = crate::delta::create_delta(&base_obj.data, &obj.data);
                        if delta.len() < (obj.data.len() * 4 / 5) {
                            if let Some((_, ref best_delta)) = best_base {
                                if delta.len() < best_delta.len() {
                                    best_base = Some((b, delta));
                                }
                            } else {
                                best_base = Some((b, delta));
                            }
                        }
                    }
                }

                if let Some((best_idx, delta)) = best_base {
                    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                    encoder.write_all(&delta).map_err(PackError::Io)?;
                    let compressed = encoder.finish().map_err(PackError::Io)?;
                    return Ok(PreparedPayload::Delta {
                        base_index: best_idx,
                        delta_len: delta.len(),
                        compressed,
                    });
                }
            }

            // Base object
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&obj.data).map_err(PackError::Io)?;
            let compressed = encoder.finish().map_err(PackError::Io)?;
            Ok(PreparedPayload::Base {
                uncompressed_len: obj.data.len(),
                compressed,
            })
        })
        .collect();

    // 2. Assemble packfile sequentially
    let mut payload = Vec::new();
    payload.extend_from_slice(b"PACK");
    payload.write_u32::<BigEndian>(2)?;
    payload.write_u32::<BigEndian>(objects.len() as u32)?;

    let mut indexed_objects = Vec::with_capacity(objects.len());
    let mut offsets = Vec::with_capacity(objects.len());

    for (i, item_res) in prepared.into_iter().enumerate() {
        let item = item_res?;
        let start_offset = payload.len() as u64;
        offsets.push(start_offset);

        match item {
            PreparedPayload::Base {
                uncompressed_len,
                compressed,
            } => {
                let type_code = type_to_code(objects[i].obj_type);
                let header = encode_object_header(type_code, uncompressed_len);
                payload.extend_from_slice(&header);
                payload.extend_from_slice(&compressed);
            }
            PreparedPayload::Delta {
                base_index,
                delta_len,
                compressed,
            } => {
                let base_offset = offsets[base_index];
                let header = encode_object_header(OBJ_OFS_DELTA, delta_len);
                let ofs_bytes = encode_offset_delta(start_offset - base_offset);
                payload.extend_from_slice(&header);
                payload.extend_from_slice(&ofs_bytes);
                payload.extend_from_slice(&compressed);
            }
        }

        let end_offset = payload.len();
        let packed_slice = &payload[start_offset as usize..end_offset];
        let mut crc = flate2::Crc::new();
        crc.update(packed_slice);
        let crc32 = crc.sum();

        indexed_objects.push(IndexedObject {
            oid: objects[i].oid,
            offset: start_offset,
            crc32,
        });
    }

    // 3. Trailing 20-byte SHA-1 checksum of all preceding packfile bytes
    let mut hasher = Sha1::new();
    hasher.update(&payload);
    let pack_checksum_bytes: [u8; 20] = hasher.finalize().into();
    let pack_checksum = ObjectId::from_bytes(pack_checksum_bytes);
    payload.extend_from_slice(pack_checksum.as_bytes());

    Ok((payload, indexed_objects, pack_checksum))
}

/// Maximum allowed delta depth to prevent stack overflow on deep or recursive delta chains.
pub const MAX_DELTA_DEPTH: usize = 64;

/// Maximum allowed decompressed object size (512 MiB).
pub const MAX_DECOMPRESSED_OBJECT_SIZE: usize = 512 * 1024 * 1024;

/// Decompresses a zlib stream of known uncompressed size.
/// Returns `(decompressed_data, compressed_bytes_consumed)`.
fn decompress_zlib(input: &[u8], expected_size: usize) -> Result<(Vec<u8>, usize), PackError> {
    if expected_size > MAX_DECOMPRESSED_OBJECT_SIZE {
        return Err(PackError::DeltaError(format!(
            "declared object size {} exceeds maximum allowed limit {}",
            expected_size, MAX_DECOMPRESSED_OBJECT_SIZE
        )));
    }
    let mut decompress = flate2::Decompress::new(true);
    let mut out = vec![0u8; expected_size];
    let mut out_pos = 0;

    loop {
        let in_before = decompress.total_in() as usize;
        let out_before = decompress.total_out() as usize;

        let status = decompress
            .decompress(
                &input[in_before..],
                &mut out[out_pos..],
                flate2::FlushDecompress::Finish,
            )
            .map_err(|e| PackError::DeltaError(format!("zlib decompress error: {}", e)))?;

        let out_produced = (decompress.total_out() as usize) - out_before;
        out_pos += out_produced;

        if status == flate2::Status::StreamEnd {
            break;
        }

        if in_before == decompress.total_in() as usize && out_produced == 0 {
            return Err(PackError::DeltaError("zlib decompress stalled".to_string()));
        }

        if out_pos >= expected_size && status != flate2::Status::StreamEnd {
            return Err(PackError::DeltaError(format!(
                "zlib stream produced more bytes than declared size {}",
                expected_size
            )));
        }
    }

    if out_pos != expected_size {
        return Err(PackError::DeltaError(format!(
            "decompressed size mismatch: expected {}, got {}",
            expected_size, out_pos
        )));
    }

    out.truncate(out_pos);
    Ok((out, decompress.total_in() as usize))
}

/// A callback to resolve external base objects for `OBJ_REF_DELTA`.
pub type BaseResolver<'a> = &'a dyn Fn(&ObjectId) -> Result<(ObjectType, Vec<u8>), PackError>;

/// Reads and resolves a single object from a packfile at a specific offset.
/// Returns `(obj_type, data, packed_len, crc32)`.
pub fn read_pack_object_at(
    pack_data: &[u8],
    offset: u64,
    base_resolver: Option<BaseResolver<'_>>,
) -> Result<(ObjectType, Vec<u8>, usize, u32), PackError> {
    let mut visited_offsets = std::collections::HashSet::new();
    read_pack_object_at_depth(pack_data, offset, base_resolver, 0, &mut visited_offsets)
}

/// Reads and resolves a single object from a packfile with delta recursion limits and cycle detection.
pub fn read_pack_object_at_depth(
    pack_data: &[u8],
    offset: u64,
    base_resolver: Option<BaseResolver<'_>>,
    depth: usize,
    visited_offsets: &mut std::collections::HashSet<u64>,
) -> Result<(ObjectType, Vec<u8>, usize, u32), PackError> {
    if depth > MAX_DELTA_DEPTH {
        return Err(PackError::DeltaError(format!(
            "delta depth exceeded maximum limit ({})",
            MAX_DELTA_DEPTH
        )));
    }
    if !visited_offsets.insert(offset) {
        return Err(PackError::DeltaError(format!(
            "cyclical delta chain detected at offset {}",
            offset
        )));
    }

    let start = offset as usize;
    if start >= pack_data.len() {
        return Err(PackError::InvalidPackSignature);
    }

    let (obj_type_raw, uncompressed_size, header_len) = decode_object_header(pack_data, start)?;
    let mut cursor = start + header_len;

    match obj_type_raw {
        OBJ_OFS_DELTA => {
            let (ofs, delta_ofs_len) = decode_offset_delta(pack_data, cursor)?;
            cursor += delta_ofs_len;
            if ofs == 0 {
                return Err(PackError::DeltaError(
                    "offset delta ofs cannot be 0".to_string(),
                ));
            }
            let base_offset = offset
                .checked_sub(ofs)
                .ok_or(PackError::InvalidPackSignature)?;
            if base_offset >= offset {
                return Err(PackError::DeltaError(
                    "offset delta base offset must precede current offset".to_string(),
                ));
            }

            let (delta_data, consumed) = decompress_zlib(&pack_data[cursor..], uncompressed_size)?;
            cursor += consumed;

            let packed_len = cursor - start;
            let mut crc = flate2::Crc::new();
            crc.update(&pack_data[start..cursor]);
            let crc32 = crc.sum();

            let (base_type, base_data, _, _) = read_pack_object_at_depth(
                pack_data,
                base_offset,
                base_resolver,
                depth + 1,
                visited_offsets,
            )?;
            let reconstructed = apply_delta(&base_data, &delta_data)?;

            Ok((base_type, reconstructed, packed_len, crc32))
        }
        OBJ_REF_DELTA => {
            if cursor + 20 > pack_data.len() {
                return Err(PackError::InvalidPackSignature);
            }
            let mut base_oid_bytes = [0u8; 20];
            base_oid_bytes.copy_from_slice(&pack_data[cursor..cursor + 20]);
            let base_oid = ObjectId::from_bytes(base_oid_bytes);
            cursor += 20;

            let (delta_data, consumed) = decompress_zlib(&pack_data[cursor..], uncompressed_size)?;
            cursor += consumed;

            let packed_len = cursor - start;
            let mut crc = flate2::Crc::new();
            crc.update(&pack_data[start..cursor]);
            let crc32 = crc.sum();

            let resolver = base_resolver.ok_or_else(|| {
                PackError::DeltaError(format!("missing base resolver for REF_DELTA {}", base_oid))
            })?;
            let (base_type, base_data) = resolver(&base_oid)?;
            let reconstructed = apply_delta(&base_data, &delta_data)?;

            Ok((base_type, reconstructed, packed_len, crc32))
        }
        base_code => {
            let obj_type = code_to_type(base_code)?;
            let (data, consumed) = decompress_zlib(&pack_data[cursor..], uncompressed_size)?;
            cursor += consumed;

            let packed_len = cursor - start;
            let mut crc = flate2::Crc::new();
            crc.update(&pack_data[start..cursor]);
            let crc32 = crc.sum();

            Ok((obj_type, data, packed_len, crc32))
        }
    }
}

/// Unpacks all objects from a packfile byte slice.
/// Returns a list of `(oid, obj_type, data)`.
pub fn unpack_packfile(
    pack_data: &[u8],
) -> Result<Vec<(ObjectId, ObjectType, Vec<u8>)>, PackError> {
    if pack_data.len() < 12 + 20 {
        return Err(PackError::InvalidPackSignature);
    }

    // Check magic
    if &pack_data[0..4] != b"PACK" {
        return Err(PackError::InvalidPackSignature);
    }

    let mut cursor = 4;
    let mut header_reader = &pack_data[4..12];
    let version = header_reader.read_u32::<BigEndian>()?;
    if version != 2 {
        return Err(PackError::UnsupportedVersion(version));
    }
    let count = header_reader.read_u32::<BigEndian>()? as usize;
    cursor += 8;

    // Verify pack checksum
    let payload_len = pack_data.len() - 20;
    let mut hasher = Sha1::new();
    hasher.update(&pack_data[..payload_len]);
    let calculated_hash: [u8; 20] = hasher.finalize().into();
    if calculated_hash != pack_data[payload_len..] {
        return Err(PackError::ChecksumMismatch);
    }

    let mut resolved_objects = Vec::with_capacity(count);
    let mut objects_by_offset = HashMap::new();
    let mut objects_by_oid: HashMap<ObjectId, (ObjectType, Vec<u8>)> = HashMap::new();

    for _ in 0..count {
        let obj_offset = cursor as u64;

        let resolver = |oid: &ObjectId| -> Result<(ObjectType, Vec<u8>), PackError> {
            if let Some((t, d)) = objects_by_oid.get(oid) {
                Ok((*t, d.clone()))
            } else {
                Err(PackError::DeltaError(format!(
                    "base object not found: {}",
                    oid
                )))
            }
        };

        let (obj_type, data, packed_len, _crc32) =
            read_pack_object_at(pack_data, obj_offset, Some(&resolver))?;

        // Calculate Git SHA-1: "<type> <size>\0<content>"
        let mut obj_hasher = Sha1::new();
        obj_hasher.update(format!("{} {}\0", obj_type.as_str(), data.len()).as_bytes());
        obj_hasher.update(&data);
        let oid = ObjectId::from_bytes(obj_hasher.finalize().into());

        objects_by_offset.insert(obj_offset, (obj_type, data.clone()));
        objects_by_oid.insert(oid, (obj_type, data.clone()));
        resolved_objects.push((oid, obj_type, data));

        cursor += packed_len;
    }

    Ok(resolved_objects)
}

/// Generates an index (`Vec<IndexedObject>` and pack checksum) for a `.pack` file slice.
pub fn index_packfile(pack_data: &[u8]) -> Result<(Vec<IndexedObject>, ObjectId), PackError> {
    if pack_data.len() < 12 + 20 {
        return Err(PackError::InvalidPackSignature);
    }

    if &pack_data[0..4] != b"PACK" {
        return Err(PackError::InvalidPackSignature);
    }

    let mut cursor = 4;
    let mut header_reader = &pack_data[4..12];
    let version = header_reader.read_u32::<BigEndian>()?;
    if version != 2 {
        return Err(PackError::UnsupportedVersion(version));
    }
    let count = header_reader.read_u32::<BigEndian>()? as usize;
    cursor += 8;

    let payload_len = pack_data.len() - 20;
    let mut hasher = Sha1::new();
    hasher.update(&pack_data[..payload_len]);
    let calculated_hash: [u8; 20] = hasher.finalize().into();
    if calculated_hash != pack_data[payload_len..] {
        return Err(PackError::ChecksumMismatch);
    }
    let pack_checksum = ObjectId::from_bytes(calculated_hash);

    let mut indexed = Vec::with_capacity(count);
    let mut objects_by_oid: HashMap<ObjectId, (ObjectType, Vec<u8>)> = HashMap::new();

    for _ in 0..count {
        let obj_offset = cursor as u64;

        let resolver = |oid: &ObjectId| -> Result<(ObjectType, Vec<u8>), PackError> {
            if let Some((t, d)) = objects_by_oid.get(oid) {
                Ok((*t, d.clone()))
            } else {
                Err(PackError::DeltaError(format!(
                    "base object not found: {}",
                    oid
                )))
            }
        };

        let (obj_type, data, packed_len, crc32) =
            read_pack_object_at(pack_data, obj_offset, Some(&resolver))?;

        let mut obj_hasher = Sha1::new();
        obj_hasher.update(format!("{} {}\0", obj_type.as_str(), data.len()).as_bytes());
        obj_hasher.update(&data);
        let oid = ObjectId::from_bytes(obj_hasher.finalize().into());

        objects_by_oid.insert(oid, (obj_type, data));

        indexed.push(IndexedObject {
            oid,
            offset: obj_offset,
            crc32,
        });

        cursor += packed_len;
    }

    Ok((indexed, pack_checksum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packfile_write_and_unpack_roundtrip() {
        let blob1 = b"Hello, this is blob number 1 for packfile testing!";
        let blob2 = b"Hello, this is blob number 2 for packfile testing with some extra!";
        let commit_text = b"tree 0000000000000000000000000000000000000000\nauthor A <a@a.com> 1234567890 +0000\ncommitter A <a@a.com> 1234567890 +0000\n\nInitial commit";

        let mut h1 = Sha1::new();
        h1.update(format!("blob {}\0", blob1.len()).as_bytes());
        h1.update(blob1);
        let oid1 = ObjectId::from_bytes(h1.finalize().into());

        let mut h2 = Sha1::new();
        h2.update(format!("blob {}\0", blob2.len()).as_bytes());
        h2.update(blob2);
        let oid2 = ObjectId::from_bytes(h2.finalize().into());

        let mut h3 = Sha1::new();
        h3.update(format!("commit {}\0", commit_text.len()).as_bytes());
        h3.update(commit_text);
        let oid3 = ObjectId::from_bytes(h3.finalize().into());

        let objects = vec![
            RawPackObject::new(oid1, ObjectType::Blob, blob1.to_vec()),
            RawPackObject::new(oid2, ObjectType::Blob, blob2.to_vec()),
            RawPackObject::new(oid3, ObjectType::Commit, commit_text.to_vec()),
        ];

        let (pack_bytes, indexed, checksum) =
            write_pack(&objects, true).expect("pack writing succeeded");

        assert_eq!(indexed.len(), 3);
        assert!(!checksum.is_zero());

        let unpacked = unpack_packfile(&pack_bytes).expect("pack unpacking succeeded");
        assert_eq!(unpacked.len(), 3);
        assert_eq!(unpacked[0].0, oid1);
        assert_eq!(unpacked[0].2, blob1);
        assert_eq!(unpacked[1].0, oid2);
        assert_eq!(unpacked[1].2, blob2);
        assert_eq!(unpacked[2].0, oid3);
        assert_eq!(unpacked[2].2, commit_text);
    }
}
