# Oxidize Architecture Guide

Oxidize is an engineered, byte-compatible, multi-threaded reimplementation of Git written in memory-safe, idiomatic Rust. It is organized as a modular workspace consisting of 9 specialized crates.

```
┌──────────────────────────────────────────────────────────┐
│                     ox (CLI Entrypoint)                  │
└────────┬──────────────────────┬──────────────────────────┘
         │                      │
         ▼                      ▼
┌──────────────────┐   ┌──────────────────┐
│   oxidize-tui    │   │  oxidize-config  │
└────────┬─────────┘   └────────┬─────────┘
         │                      │
         ▼                      ▼
┌──────────────────────────────────────────────────────────┐
│                   oxidize-transport                      │
└────────┬──────────────────────┬──────────────────────────┘
         │                      │
         ▼                      ▼
┌──────────────────┐   ┌──────────────────┐
│   oxidize-refs   │   │   oxidize-diff   │
└────────┬─────────┘   └────────┬─────────┘
         │                      │
         ▼                      ▼
┌──────────────────┐   ┌──────────────────┐
│   oxidize-index  │   │   oxidize-pack   │
└────────┬─────────┘   └────────┬─────────┘
         │                      │
         └──────────┬───────────┘
                    ▼
┌──────────────────────────────────────────────────────────┐
│                    oxidize-core                          │
└──────────────────────────────────────────────────────────┘
```

---

## Workspace Crates Overview

| Crate | Purpose | Key Primitives |
|---|---|---|
| [`oxidize-core`](file:///c:/Users/PC/Documents/OXIDIZE/crates/core) | Fundamental Git object models, SHA-1 IDs, zlib compression, and loose storage | `ObjectId`, `Object`, `ObjectType`, `Commit`, `Tree`, `Blob`, `Tag`, `ObjectStore` |
| [`oxidize-index`](file:///c:/Users/PC/Documents/OXIDIZE/crates/index) | DIRC binary format parser and serializer, working tree status detection | `Index`, `IndexEntry`, `IndexVersion`, `FileMode`, `WorkingTreeStatus` |
| [`oxidize-refs`](file:///c:/Users/PC/Documents/OXIDIZE/crates/refs) | Reference management, packed-refs parsing, atomic ref transactions, reflog | `RefStore`, `Reference`, `Reflog`, `ReflogEntry`, `SymbolicRef` |
| [`oxidize-diff`](file:///c:/Users/PC/Documents/OXIDIZE/crates/diff) | Myers difference algorithm, unified diff formatter, line-level hunk generation | `myers_diff`, `UnifiedDiff`, `Hunk`, `DiffLine`, `FileDiff` |
| [`oxidize-pack`](file:///c:/Users/PC/Documents/OXIDIZE/crates/pack) | Packfile v2 reader & writer, .idx fanout search, OFS/REF delta decompression | `PackFile`, `PackIndex`, `DeltaResolver`, `PackWriter`, `MmapReader` |
| [`oxidize-transport`](file:///c:/Users/PC/Documents/OXIDIZE/crates/transport) | Git smart HTTP & native SSH protocols, pkt-line encoding, push/fetch negotiation | `SmartHttp`, `SshTransport`, `PktLineReader`, `PktLineWriter`, `Discovery` |
| [`oxidize-config`](file:///c:/Users/PC/Documents/OXIDIZE/crates/config) | INI-style `.git/config` parser and writer with section/subsection support | `GitConfig`, `ConfigSection`, `ConfigValue` |
| [`oxidize-tui`](file:///c:/Users/PC/Documents/OXIDIZE/crates/tui) | Terminal user interface for interactive branch browsing, log graph, and staging | `App`, `TreeWidget`, `LogWidget`, `StatusWidget` (via Ratatui) |
| [`ox` (cli)](file:///c:/Users/PC/Documents/OXIDIZE/crates/cli) | Command-line interface orchestration, argument parsing, output formatting | `Cli`, `Commands`, `OutputFormat`, `ShellCompletions` |

---

## 1. Core Object Engine (`oxidize-core`)

### Object Identification (`ObjectId`)
Git objects are addressed by the 20-byte SHA-1 digest of their formatted content:
$$\text{SHA1}(\text{"<type> <size>\0<data>"})$$
In Oxidize, `ObjectId` is represented as an unaligned 20-byte array `[u8; 20]` that implements `Copy`, `Eq`, `Hash`, `Ord`, and `Display` (hex formatting).

### Object Serialization
All object formats adhere byte-for-byte to C Git specifications:
- **Commit**: Includes `tree <sha1>`, optional `parent <sha1>` references, `author <ident>`, `committer <ident>`, and a free-form message.
- **Tree**: Sorted sequence of `"<mode> <name>\0<20-byte-hash>"`. Sorting enforces Git's canonical order where directory entries are sorted as if ending in `/`.
- **Blob**: Raw byte array of file contents.
- **Tag**: Annotated tag pointing to an object target with tagger identity and PGP payload support.

### Loose Object Storage
Loose objects are stored in `.git/objects/xx/yyyy...` compressed with `flate2` DEFLATE/zlib (level 6). Oxidize handles file creation atomically via temporary files renamed into place, preventing race conditions.

---

## 2. Fast Index Engine (`oxidize-index`)

### DIRC Binary Layout
The index file (`.git/index`) implements standard `DIRC` (Directory Cache) version 2 and version 4 formats:
1. **12-byte Header**:
   - `4 bytes`: Magic bytes `DIRC`
   - `4 bytes`: Version number (`2` or `4`)
   - `4 bytes`: Number of index entries (big-endian `u32`)
2. **Entries**:
   - Stat cache: `ctime` (8B), `mtime` (8B), `dev` (4B), `ino` (4B), `mode` (4B), `uid` (4B), `gid` (4B), `file_size` (4B).
   - 20-byte SHA-1 hash of the staged blob.
   - Flags: stage (bits 12-13), skip-worktree, assume-unchanged, path length.
   - Variable-length path string, NUL-padded to 8-byte alignment in v2 or prefix-compressed in v4.
3. **Checksum**:
   - Trailing 20-byte SHA-1 hash covering the entire file up to that point.

### Working Tree Diff Detection
To determine dirty files efficiently:
1. Stat the working tree file using filesystem metadata (`std::fs::symlink_metadata`).
2. Compare `mtime`, `size`, and `mode` against the stored index entry.
3. If stat cache matches, the file is assumed clean without reading contents.
4. If stat cache differs, read and compute SHA-1 of the working tree file to verify actual content changes.

---

## 3. High-Performance Packfile Engine (`oxidize-pack`)

Packfiles store Git objects with high compression using delta encoding. Oxidize achieves high throughput through:

### Memory-Mapped I/O (`memmap2`)
Instead of reading gigabyte-sized packfiles into heap allocations, `oxidize-pack` memory-maps both `.pack` and `.idx` files using `memmap2::Mmap`. The OS manages paging directly from disk into memory.

### Binary Fan-Out Search (`.idx` v2)
1. The 256-entry primary fan-out table allows locating the candidate hash range in $O(1)$ time.
2. Binary search within that range ($O(\log N)$) locates the 20-byte object name.
3. The corresponding offset in the 4-byte offset table (or 8-byte large offset table) provides the exact byte position within the `.pack` file.

### Delta Chain Resolution
Oxidize handles both `OBJ_OFS_DELTA` (offset-based) and `OBJ_REF_DELTA` (hash-based) deltas:
- Deltas represent byte-level copy/insert instructions from a base object.
- Oxidize resolves deep delta chains iteratively, preventing stack overflow on long commit histories.
- Resolved objects are cached in an LRU memory buffer to accelerate consecutive lookups.

---

## 4. Myers Difference Algorithm (`oxidize-diff`)

Oxidize implements the classic Eugene Myers $O(ND)$ difference algorithm:
- Generates the minimal edit script (SES) between two sequences of lines.
- Groups contiguous additions and deletions into unified diff hunks with configurable context lines (default: 3).
- Generates diff headers (`diff --git a/file b/file`, `index ...`, `--- a/file`, `+++ b/file`, `@@ -l,s +l,s @@`).

---

## 5. Parallel Execution Engine (`rayon`)

Multi-threading is integrated across computationally heavy paths:
- **Batch Hashing**: In `ox add .`, files are scanned, read, and SHA-1 hashed concurrently across CPU cores using `rayon::par_iter()`.
- **Pack Indexing**: Object verification and checksum validation run in parallel batches.
- **Diff Calculation**: Multi-file diffs compute independent file diffs concurrently.
