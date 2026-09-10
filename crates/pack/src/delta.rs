//! Git Packfile Delta compression and decompression engine.

use crate::PackError;

/// Maximum allowed delta target object size (512 MiB).
pub const MAX_DELTA_TARGET_SIZE: usize = 512 * 1024 * 1024;

/// Applies a Git packfile delta stream to a base object slice.
pub fn apply_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, PackError> {
    let mut cursor = 0;

    // 1. Read base object size (variable length integer)
    let (base_size, bytes_read) = read_varint(delta, cursor)?;
    cursor += bytes_read;
    if base.len() != base_size {
        return Err(PackError::DeltaError(format!(
            "base size mismatch: expected {}, got {}",
            base_size,
            base.len()
        )));
    }

    // 2. Read target object size (variable length integer)
    let (target_size, bytes_read) = read_varint(delta, cursor)?;
    cursor += bytes_read;
    if target_size > MAX_DELTA_TARGET_SIZE {
        return Err(PackError::DeltaError(format!(
            "target size {} exceeds maximum allowed limit {}",
            target_size, MAX_DELTA_TARGET_SIZE
        )));
    }

    // Bounded initial capacity allocation to prevent OOM on malicious target_size
    let mut out = Vec::with_capacity(target_size.min(1024 * 1024));

    // 3. Process delta instruction opcodes
    while cursor < delta.len() {
        let opcode = delta[cursor];
        cursor += 1;

        if (opcode & 0x80) != 0 {
            // Copy instruction
            let mut offset = 0usize;
            let mut size = 0usize;

            if (opcode & 0x01) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                offset |= delta[cursor] as usize;
                cursor += 1;
            }
            if (opcode & 0x02) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                offset |= (delta[cursor] as usize) << 8;
                cursor += 1;
            }
            if (opcode & 0x04) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                offset |= (delta[cursor] as usize) << 16;
                cursor += 1;
            }
            if (opcode & 0x08) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                offset |= (delta[cursor] as usize) << 24;
                cursor += 1;
            }

            if (opcode & 0x10) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                size |= delta[cursor] as usize;
                cursor += 1;
            }
            if (opcode & 0x20) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                size |= (delta[cursor] as usize) << 8;
                cursor += 1;
            }
            if (opcode & 0x40) != 0 {
                if cursor >= delta.len() {
                    return Err(PackError::DeltaError(
                        "truncated copy instruction in delta".to_string(),
                    ));
                }
                size |= (delta[cursor] as usize) << 16;
                cursor += 1;
            }

            if size == 0 {
                size = 0x10000; // 64 KiB
            }

            let end_offset = offset.checked_add(size).ok_or_else(|| {
                PackError::DeltaError("copy offset arithmetic overflow".to_string())
            })?;

            if end_offset > base.len() {
                return Err(PackError::DeltaError(format!(
                    "copy out of bounds: offset {} + size {} > base len {}",
                    offset,
                    size,
                    base.len()
                )));
            }

            if out.len().checked_add(size).is_none_or(|l| l > target_size) {
                return Err(PackError::DeltaError(format!(
                    "delta output exceeds declared target size {}",
                    target_size
                )));
            }

            out.extend_from_slice(&base[offset..end_offset]);
        } else if opcode != 0 {
            // Insert instruction
            let size = opcode as usize;
            let end = cursor
                .checked_add(size)
                .ok_or_else(|| PackError::DeltaError("insert size overflow".to_string()))?;
            if end > delta.len() {
                return Err(PackError::DeltaError(
                    "insert instruction exceeds delta slice".to_string(),
                ));
            }

            if out.len().checked_add(size).is_none_or(|l| l > target_size) {
                return Err(PackError::DeltaError(format!(
                    "delta output exceeds declared target size {}",
                    target_size
                )));
            }

            out.extend_from_slice(&delta[cursor..end]);
            cursor = end;
        } else {
            return Err(PackError::DeltaError("invalid delta opcode 0".to_string()));
        }
    }

    if out.len() != target_size {
        return Err(PackError::DeltaError(format!(
            "target size mismatch: expected {}, got {}",
            target_size,
            out.len()
        )));
    }

    Ok(out)
}

/// Creates a delta between a base slice and a target slice using copy and insert instructions.
pub fn create_delta(base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut delta = Vec::new();
    write_varint(&mut delta, base.len());
    write_varint(&mut delta, target.len());

    // 1. Find common prefix length
    let mut prefix_len = 0;
    while prefix_len < base.len()
        && prefix_len < target.len()
        && base[prefix_len] == target[prefix_len]
    {
        prefix_len += 1;
    }

    // 2. Find common suffix length (not overlapping prefix)
    let mut suffix_len = 0;
    while suffix_len < (base.len() - prefix_len)
        && suffix_len < (target.len() - prefix_len)
        && base[base.len() - 1 - suffix_len] == target[target.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    // Emit copy for prefix if >= 4 bytes (heuristic threshold)
    if prefix_len >= 4 {
        emit_copy(&mut delta, 0, prefix_len);
    } else {
        prefix_len = 0;
    }

    // Emit insert for middle target slice
    let middle_target = &target[prefix_len..target.len() - suffix_len];
    let mut cursor = 0;
    while cursor < middle_target.len() {
        let chunk_size = (middle_target.len() - cursor).min(127);
        delta.push(chunk_size as u8);
        delta.extend_from_slice(&middle_target[cursor..cursor + chunk_size]);
        cursor += chunk_size;
    }

    // Emit copy for suffix if >= 4 bytes
    if suffix_len >= 4 {
        let base_offset = base.len() - suffix_len;
        emit_copy(&mut delta, base_offset, suffix_len);
    } else if suffix_len > 0 {
        let suffix_target = &target[target.len() - suffix_len..];
        let mut cursor = 0;
        while cursor < suffix_target.len() {
            let chunk_size = (suffix_target.len() - cursor).min(127);
            delta.push(chunk_size as u8);
            delta.extend_from_slice(&suffix_target[cursor..cursor + chunk_size]);
            cursor += chunk_size;
        }
    }

    delta
}

fn emit_copy(delta: &mut Vec<u8>, mut offset: usize, mut size: usize) {
    while size > 0 {
        let chunk = size.min(0x10000);
        let mut opcode = 0x80u8;
        let mut args = Vec::with_capacity(7);

        if (offset & 0xFF) != 0 {
            opcode |= 0x01;
            args.push((offset & 0xFF) as u8);
        }
        if ((offset >> 8) & 0xFF) != 0 {
            opcode |= 0x02;
            args.push(((offset >> 8) & 0xFF) as u8);
        }
        if ((offset >> 16) & 0xFF) != 0 {
            opcode |= 0x04;
            args.push(((offset >> 16) & 0xFF) as u8);
        }
        if ((offset >> 24) & 0xFF) != 0 {
            opcode |= 0x08;
            args.push(((offset >> 24) & 0xFF) as u8);
        }

        if chunk != 0x10000 {
            if (chunk & 0xFF) != 0 {
                opcode |= 0x10;
                args.push((chunk & 0xFF) as u8);
            }
            if ((chunk >> 8) & 0xFF) != 0 {
                opcode |= 0x20;
                args.push(((chunk >> 8) & 0xFF) as u8);
            }
            if ((chunk >> 16) & 0xFF) != 0 {
                opcode |= 0x40;
                args.push(((chunk >> 16) & 0xFF) as u8);
            }
        }

        delta.push(opcode);
        delta.extend_from_slice(&args);

        offset += chunk;
        size -= chunk;
    }
}

fn read_varint(data: &[u8], mut cursor: usize) -> Result<(usize, usize), PackError> {
    let mut result = 0usize;
    let mut shift = 0;
    let start = cursor;

    while cursor < data.len() {
        let byte = data[cursor];
        cursor += 1;
        let val = (byte & 0x7F) as usize;
        if shift >= usize::BITS as usize {
            return Err(PackError::DeltaError("varint overflow".to_string()));
        }
        if val != 0
            && shift + 7 > usize::BITS as usize
            && (val >> (usize::BITS as usize - shift)) != 0
        {
            return Err(PackError::DeltaError("varint overflow".to_string()));
        }
        result |= val << shift;
        if (byte & 0x80) == 0 {
            return Ok((result, cursor - start));
        }
        shift += 7;
    }

    Err(PackError::DeltaError("truncated varint".to_string()))
}

fn write_varint(buf: &mut Vec<u8>, mut val: usize) {
    loop {
        let byte = (val & 0x7F) as u8;
        val >>= 7;
        if val == 0 {
            buf.push(byte);
            break;
        } else {
            buf.push(byte | 0x80);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_roundtrip() {
        let base = b"Hello, world! This is a long piece of base text that we will modify.";
        let target = b"Hello, brave new world! This is a long piece of base text that we will modify slightly.";

        let delta = create_delta(base, target);
        let reconstructed = apply_delta(base, &delta).expect("delta application succeeds");
        assert_eq!(reconstructed, target);
    }

    #[test]
    fn test_empty_and_large_copy() {
        let base = vec![b'x'; 70000];
        let mut target = base.clone();
        target.extend_from_slice(b"extra ending");

        let delta = create_delta(&base, &target);
        let reconstructed = apply_delta(&base, &delta).expect("delta application succeeds");
        assert_eq!(reconstructed, target);
    }
}
