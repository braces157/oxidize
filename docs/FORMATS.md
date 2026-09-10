# Git Binary Formats Specification in Oxidize

Oxidize guarantees 100% byte-compatibility with official Git storage layouts and binary serialization standards. This document specifies each format implemented across the Oxidize codebase.

---

## 1. Loose Object Format

Every loose object in `.git/objects/xx/yyyy...` is a zlib-compressed stream structured as:

```
[type_string] [space] [size_in_decimal] [\0] [raw_object_content]
```

### Type Identifiers
- `commit`: Commit metadata (tree, parent, author, committer, message)
- `tree`: Ordered binary directory entries
- `blob`: File content bytes
- `tag`: Annotated tag data

### SHA-1 Calculation
The 20-byte object hash is computed over the uncompressed byte sequence:
```rust
let header = format!("{} {}\0", obj_type, content.len());
let mut hasher = Sha1::new();
hasher.update(header.as_bytes());
hasher.update(content);
let hash = hasher.finalize();
```

---

## 2. Tree Binary Object Format

A Git tree is a binary concatenation of tree entries sorted canonically:

```
[mode_octal_ascii] [space] [filename_bytes] [\0] [20_byte_binary_sha1]
```

### Canonical Sort Order
Entries in a tree are sorted by path name, with directory entries treated as if a trailing slash `/` was appended to their name.

---

## 3. Git Index Format (DIRC v2 and v4)

Stored in `.git/index`.

```
┌─────────────────────────────────────────────────────────────────┐
│                          12-byte Header                         │
│  4 bytes: "DIRC"  │  4 bytes: version (2 or 4)  │ 4 bytes: count│
├─────────────────────────────────────────────────────────────────┤
│                          Index Entries                          │
│  62 bytes fixed metadata + path + NUL padding (v2)              │
│  or prefix compressed (v4)                                      │
├─────────────────────────────────────────────────────────────────┤
│                       Optional Extension Blocks                 │
│  e.g. TREE (Cached Tree), UNTR (Untracked cache), etc.          │
├─────────────────────────────────────────────────────────────────┤
│                         20-byte SHA-1                           │
│  Checksum covering all bytes from start of header               │
└─────────────────────────────────────────────────────────────────┘
```

### Fixed Metadata per Entry (62 bytes)
```
Offset  Size  Field
0       4     ctime seconds
4       4     ctime nanosecond fraction
8       4     mtime seconds
12      4     mtime nanosecond fraction
16      4     dev (device ID)
20      4     ino (inode number)
24      4     mode (file permissions / object type)
28      4     uid (user ID)
32      4     gid (group ID)
36      4     file size (truncated to 32 bits)
40      20    20-byte SHA-1 hash of blob
60      2     flags (stage, skip-worktree, name length)
```

---

## 4. Packfile Format (v2)

Stored in `.git/objects/pack/pack-*.pack`.

```
┌─────────────────────────────────────────────────────────────────┐
│                          12-byte Header                         │
│  4 bytes: "PACK"  │  4 bytes: version (2)  │  4 bytes: # objects│
├─────────────────────────────────────────────────────────────────┤
│                          Packed Objects                         │
│  Variable-length header (type + uncompressed size)              │
│  Optional delta header (OFS offset or REF 20-byte hash)         │
│  Zlib-compressed data stream                                    │
├─────────────────────────────────────────────────────────────────┤
│                         20-byte SHA-1                           │
│  Checksum of all preceding bytes in packfile                    │
└─────────────────────────────────────────────────────────────────┘
```

### Object Types in Packfile
| Code | Constant | Meaning |
|---|---|---|
| `1` | `OBJ_COMMIT` | Commit object |
| `2` | `OBJ_TREE` | Tree object |
| `3` | `OBJ_BLOB` | Blob object |
| `4` | `OBJ_TAG` | Tag object |
| `6` | `OBJ_OFS_DELTA` | Offset delta pointing back $N$ bytes in current pack |
| `7` | `OBJ_REF_DELTA` | Reference delta pointing to 20-byte base object ID |

### Variable-Length Header Encoding
- **Byte 1**: MSB indicates more header bytes (`1xxx_xxxx`). Bits 4-6 encode the 3-bit object type. Bits 0-3 encode the least significant 4 bits of the uncompressed size.
- **Subsequent Bytes**: MSB indicates more bytes (`1xxx_xxxx`). Remaining 7 bits provide the next chunk of the uncompressed size (little-endian order).

---

## 5. Pack Index Format (`.idx` v2)

Stored in `.git/objects/pack/pack-*.idx`.

```
┌─────────────────────────────────────────────────────────────────┐
│                     Magic & Version (8 bytes)                   │
│  4 bytes: \xFF t O c  │  4 bytes: version 2                     │
├─────────────────────────────────────────────────────────────────┤
│                   Fan-out Table (256 * 4 bytes)                 │
│  Cumulative object counts up to each first byte (0x00 to 0xFF)  │
├─────────────────────────────────────────────────────────────────┤
│                  Table of Object Names (N * 20 bytes)           │
│  Sorted lexicographically by 20-byte SHA-1 hash                 │
├─────────────────────────────────────────────────────────────────┤
│                  Table of CRC32 Checksums (N * 4 bytes)         │
│  CRC32 of compressed object data in packfile                    │
├─────────────────────────────────────────────────────────────────┤
│                  Table of Offsets (N * 4 bytes)                 │
│  Packfile byte offsets. MSB set indicates 8-byte offset table.  │
├─────────────────────────────────────────────────────────────────┤
│              Optional 8-byte Large Offset Table (variable)      │
│  Used for packfiles exceeding 2 GiB in size                     │
├─────────────────────────────────────────────────────────────────┤
│                  Packfile Checksum (20 bytes)                   │
│  SHA-1 checksum of the corresponding .pack file                 │
├─────────────────────────────────────────────────────────────────┤
│                  Index Checksum (20 bytes)                      │
│  SHA-1 checksum covering all preceding bytes of this .idx       │
└─────────────────────────────────────────────────────────────────┘
```

---

## 6. Smart HTTP & Git Pkt-Line Format

Communication over HTTP/HTTPS and SSH uses length-prefixed packet lines:

- **Format**: `HHHH[payload]` where `HHHH` is 4 hexadecimal ASCII characters indicating total packet length including the 4-byte header.
- **Flush Packet**: `0000` (length 0) signals the end of a section or stream.
- **Delimiter Packet**: `0001` signals boundary separation in protocol v2.
- **Response End**: `0002` signals end of response.
