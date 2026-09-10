# Architectural & Engineering Decisions Log (DECISIONS.md)

This document records the architectural, design, and protocol decisions made during the implementation of Oxidize (`ox`).

## Phase 1: Workspace Scaffolding & Initial Layout

### DECISION 001: Multi-Crate Workspace Architecture
- **Date**: 2026-09-10
- **Context**: Git is comprised of distinct layered subsystems (content-addressable object store, index file management, reference tracking, diffing/merging algorithms, packfiles, network transport protocols, configuration, and porcelain CLI).
- **Decision**: Structuring Oxidize into 9 dedicated workspace crates:
  - `oxidize-core`: Object identities (`ObjectId`), data models (`Blob`, `Tree`, `Commit`, `Tag`), loose storage.
  - `oxidize-index`: Git index v2/v3 DIRC format, stat cache, staging operations.
  - `oxidize-refs`: Symbolic refs, HEAD, branch/tag tracking, packed-refs, reflog.
  - `oxidize-diff`: Myers diff algorithm, unified diff output, three-way merge with conflict markers.
  - `oxidize-pack`: Packfile format, OFS/REF delta compression/decompression, `.idx` v2 fanout tables.
  - `oxidize-transport`: Git pkt-line protocol, smart HTTP client, Git protocol v2.
  - `oxidize-config`: INI parser (.git/config, ~/.gitconfig), .gitignore glob pattern matcher.
  - `oxidize-tui`: Terminal dashboard for commit graph, status, and diff visualization.
  - `ox` (`crates/cli`): Command-line binary interface dispatching commands.
- **Rationale**: Clean boundaries, testability in isolation, fast incremental builds, modular reuse.

### DECISION 002: Error Handling Strategy
- **Date**: 2026-09-10
- **Context**: Ground rules require `thiserror` for typed errors per-crate and `anyhow` at the CLI boundary, with zero `.unwrap()` or `.expect()` in non-test code.
- **Decision**: Each library crate defines a typed error enum with `#[derive(thiserror::Error)]`. Conversions between crates use explicit `#[from]` mappings where appropriate. The CLI binary uses `anyhow::Result` to print human-friendly error messages and map errors to appropriate exit codes.

### DECISION 003: Path Normalization on Windows
- **Date**: 2026-09-10
- **Context**: Git repositories require paths in index entries and tree objects to use forward slashes (`/`) without leading slashes. On Windows, file paths use backslashes (`\`).
- **Decision**: Enforce normalization of all relative working paths to forward-slash UTF-8 strings when interacting with Git index and tree objects.

## Phase 2: Object Model & Content Store

### DECISION 004: Canonical Tree Entry Ordering
- **Date**: 2026-09-10
- **Context**: Git trees require strict canonical sorting. If directory trees are not sorted as if their names have an appended trailing `/`, tree SHA-1 hashes diverge from Git's hashes whenever files and directories share a common prefix (e.g. `foo` directory vs `foo.txt` file).
- **Decision**: Implement the canonical Git comparison rule in `Tree::new`: directory names are sorted with an implicit trailing slash `/`.

### DECISION 005: Short SHA-1 Prefix Resolution & Ambiguity Detection
- **Date**: 2026-09-10
- **Context**: Commands like `git cat-file` and `git ls-tree` allow abbreviated object prefixes down to 4 characters.
- **Decision**: Implement `find_by_prefix` in `LooseObjectStore`. If a prefix is shorter than 4 characters, return an error. If multiple matching objects are found in the 2-character hex directory, return `CoreError::AmbiguousPrefix`. If exactly one matches, resolve the full `ObjectId`.

## Phase 3: Index & Staging Area

### DECISION 006: Index v2 Binary Padding and Lockfile Semantics
- **Date**: 2026-09-10
- **Context**: The Git binary index format requires 1-8 NUL bytes after the path name to pad the total entry size (62 bytes header + path length + padding) to a multiple of 8 bytes. Additionally, index writes must never leave the repository in a corrupted state during interruptions.
- **Decision**: Implemented strict 1-8 byte NUL padding formula `8 - ((62 + path_len) % 8)` followed by trailing SHA-1 checksum verification. Writing uses `.git/index.lock` with atomic rename.

### DECISION 007: Hierarchical write-tree from Flat Index
- **Date**: 2026-09-10
- **Context**: The index stores paths as a flat sorted list of slash-delimited paths (`a/b/c.txt`), whereas Git stores them as nested `Tree` objects.
- **Decision**: Build a recursive `TreeNode` Trie structure that aggregates files into intermediate directory nodes, writes subtrees bottom-up to the object store, and returns the root `ObjectId`.

## Phase 4: Core Porcelain Loop

### DECISION 008: Myers Diff Algorithm and Unified Diff Hunking
- **Date**: 2026-09-10
- **Context**: `git diff` produces canonical unified diff format with hunks (`@@ -old_start,old_count +new_start,new_count @@`) and 3 lines of context.
- **Decision**: Implemented the classic Myers O((N+M)D) shortest edit script algorithm with trace backtracking in `oxidize-diff`. Edits are grouped into unified hunks adhering to Git's 3-line context boundary rules.

### DECISION 009: Atomic Reference Updates with Reflog
- **Date**: 2026-09-10
- **Context**: Commits must update branch tips and HEAD atomically, recording an entry in `.git/logs/HEAD` and `.git/logs/refs/heads/<branch>` with old OID, new OID, signature, and message.
- **Decision**: Implemented `RefStore::update_ref` with lockfiles (`<ref>.lock`), validation of expected previous commit, and simultaneous appending to both branch and HEAD reflogs.

## Phase 5: Branching & Merging

### DECISION 010: BFS Lowest Common Ancestor (LCA) Merge Base Resolution
- **Date**: 2026-09-10
- **Context**: Merging requires accurately discovering common commit ancestors between divergent branch tips across complex commit graphs.
- **Decision**: Implemented `RefStore::find_merge_base` traversing all ancestors of the first commit into a visited set, followed by BFS on the second commit to return the first intersected ancestor.

### DECISION 011: Three-Way Line Merge Engine with Conflict Markers
- **Date**: 2026-09-10
- **Context**: When both branches touch the same file since their common ancestor, Git performs a 3-way merge. Non-overlapping edits must merge cleanly without conflict, while overlapping conflicting edits must output standard conflict markers (`<<<<<<< HEAD`, `=======`, `>>>>>>>`).
- **Decision**: Implemented `three_way_merge` in `oxidize-diff` aligning diff chunks relative to base lines. When conflicts occur, files are written with conflict markers and staged at stage 1 in the index, pausing the merge for user resolution.

## Phase 6: Packfiles & Maintenance

### DECISION 012: Git Packfile v2 & Offset Delta (OFS_DELTA) Encoding
- **Date**: 2026-09-10
- **Context**: Packfile v2 is the canonical storage format for Git repositories, bundling hundreds or thousands of objects into `.pack` archives with delta compression.
- **Decision**: Implemented `write_pack`, `read_pack_object_at`, and `unpack_packfile` in `oxidize-pack`:
  - Pack header: `PACK`, version 2, big-endian object count.
  - Object header: variable-length integer encoding 3-bit type (commit, tree, blob, tag, ofs_delta, ref_delta) and uncompressed size.
  - OFS_DELTA: Bijective base-128 negative relative offset encoding where each continuation byte adds 1 to the accumulator before shifting.
  - Delta engine: `apply_delta` and `create_delta` supporting copy (opcode 0x80 | flags) and insert (1..=127) instructions.
  - Trailing 20-byte SHA-1 covering all preceding bytes of the packfile.

### DECISION 013: Pack Index v2 (.idx) and Memory-Mapped Storage Architecture
- **Date**: 2026-09-10
- **Context**: Accessing objects within a packfile requires rapid O(log N) lookup without scanning the `.pack` sequentially. Repositories can have multiple packs.
- **Decision**: Implemented `PackIndex` v2 reader/writer and `PackStore`:
  - Header: `\xFFtOc`, version 2, 256-entry fanout table.
  - Tables: Lexicographically sorted OIDs, 4-byte IEEE 802.3 CRC32 checksums covering the entire packed slice (header + delta header + compressed bytes), 4-byte offsets and dynamic 8-byte large offset tables.
  - Trailing checksums: 20-byte pack checksum followed by 20-byte SHA-1 of the index itself.
  - `RepoObjectStore`: Transparent zero-copy memory-mapped access using `memmap2`, searching loose objects first and falling back to packfile indexes.

## Phase 7: Networking & Smart HTTP Transport

### DECISION 014: Git pkt-line Framing & Smart HTTP Client
- **Date**: 2026-09-10
- **Context**: Git network transport requires precise 4-hex-digit length-prefixed packet framing (`pkt-line`) with special markers (`0000` flush, `0001` delimiter, `0002` response end) and sideband multiplexing (channel 1: packfile data, channel 2: progress, channel 3: error).
- **Decision**: Implemented `PktLine` parser/encoder and `SidebandDemuxer` in `oxidize-transport`:
  - `SmartHttpClient` wraps `ureq` to communicate over Git Smart HTTP (`/info/refs?service=git-upload-pack`, `/git-upload-pack`, `/git-receive-pack`).
  - Supports capability negotiation (`multi_ack_detailed`, `side-band-64k`, `ofs-delta`, `agent=ox/0.1.0`).

### DECISION 015: Git Config INI Parser & Cross-Platform Path Normalization
- **Date**: 2026-09-10
- **Context**: Storing remotes and tracking branches requires reading and writing `.git/config`. On Windows, standard filesystem paths contain backslashes (`\`) which official Git parses as escape characters, causing syntax errors if written unescaped.
- **Decision**: Implemented `GitConfig` INI parser/serializer in `oxidize-config`. When adding remotes or writing URLs, backslashes are systematically normalized to forward slashes (`/`), guaranteeing 100% compatibility with official `git` on all platforms.

### DECISION 016: ObjectReader Trait & Local Transport Re-use
- **Date**: 2026-09-10
- **Context**: Cloning and checking out repositories where objects are bundled directly into packfiles (rather than loose objects) requires tree traversal and blob extraction without unpacking every object to disk.
- **Decision**: Added `ObjectReader` trait in `oxidize-core` and implemented it for both `LooseObjectStore` and `RepoObjectStore`. `checkout_tree_and_update_index` accepts `&impl ObjectReader`, allowing `ox clone` to populate the working tree directly from freshly indexed packfiles with zero loose object unpacking. For local filesystem remotes, `resolve_local_path` discovers references across loose refs and `packed-refs`, and generates thin packfiles directly.

## Phase 8: Advanced Porcelain & UX

### DECISION 017: .gitignore Pattern Matcher & Decoupled Filtering Closure
- **Date**: 2026-09-10
- **Context**: Daily-driver operations like `status` and `add .` require ignoring compiler artifacts, temporary files, and directory patterns defined in `.gitignore` (with wildcards, directory markers, negations `!`, and recursive `**`).
- **Decision**: Implemented `GitIgnore` and `IgnorePattern` in `oxidize-config`. Decoupled status scanning via `IgnoreFilter` closure in `oxidize-index`, allowing `compute_status_with_ignore` and recursive `add` to filter untracked files without circular crate dependencies.

### DECISION 018: Multi-Parent Stash Commits & Reflog Stack
- **Date**: 2026-09-10
- **Context**: `git stash` produces a special commit structure where the index state is committed as parent 2 and the working tree state as the root commit with parent 1 = HEAD, and logs the history to `logs/refs/stash`.
- **Decision**: Implemented `cmd_stash` (push, pop, list, drop) matching Git's exact dual-commit DAG topology, maintaining atomic stack state in `refs/stash` and `.git/logs/refs/stash`.

### DECISION 019: Three-Way History Manipulation (Rebase, Cherry-Pick, Revert, Blame, Bisect)
- **Date**: 2026-09-10
- **Context**: Advanced workflows demand rewriting and inspecting history: linear replay (`rebase`), selective porting (`cherry-pick`), inverse commit application (`revert`), line-by-line attribution (`blame`), and binary search debugging (`bisect`).
- **Decision**:
  - `rebase`, `cherry-pick`, and `revert` utilize 3-way line merges with Myers diff against their respective base/parent commits, automatically staging clean merges or presenting conflict markers on divergence.
  - `blame` traverses commit ancestry graphs via reverse topological BFS, mapping line survivals across diff hunk operations (`DiffOp::Keep`).
  - `bisect` computes reachability frontiers using DAG sets and selects logarithmic bisect midpoints, recording state in `.git/BISECT_*`.

## Phase 9: Polish, Performance & Deliverables

### DECISION 020: Ratatui Interactive Terminal UI (TUI) Dashboard & Event Loop
- **Date**: 2026-09-10
- **Context**: Providing an intuitive, fast, interactive visual interface for exploring commit history, inspect commits, viewing unstaged/staged working tree status, and navigating repositories (`ox ui` or `ox log --tui`).
- **Decision**: Implemented an interactive dashboard in `oxidize-tui` using `ratatui` (v0.29) and `crossterm` (v0.28):
  - Architecture: State-machine based `App` maintaining selected commit, view modes (Log vs Status), commit detail caches, and status summaries.
  - Raw Mode & Alternate Screen: Clean terminal lifecycle management entering alternate screen on startup and restoring normal terminal mode on exit (with panic hook safety).
  - Event Loop: Non-blocking 50ms polling event loop processing keyboard navigation (`j`/`k`/arrows, `Tab` switching, `q`/`Esc` exit).
  - Dual Command Integration: Accessible via dedicated `ox ui` subcommand or standard flag `ox log --tui`.

### DECISION 021: Rayon Parallelized Packfile Delta Window Compression
- **Date**: 2026-09-10
- **Context**: Packfile generation (`ox pack-objects`, `ox gc`) is compute-intensive: sliding window delta matching and zlib compression become bottlenecks for large repositories when run sequentially.
- **Decision**:
  - Leveraged `rayon`'s parallel iterators (`par_iter()`) in `write_pack` across all available CPU threads.
  - Objects are chunked across threads to evaluate candidate base objects in parallel within a sliding delta window (window size 10), selecting optimal bases that yield >20% reduction.
  - Zlib deflate compression for both delta and base objects is executed concurrently across threads.
  - The packfile is assembled sequentially in memory by resolving negative base-128 offsets against precomputed byte positions, preserving strict Git Packfile v2 conformance.

### DECISION 022: Thread-Safe Atomic Loose Object Storage and Parallel `ox add`
- **Date**: 2026-09-10
- **Context**: Staging hundreds or thousands of files with `ox add .` involves filesystem traversal, reading file contents, computing SHA-1 hashes, and writing loose objects to `.git/objects/`. Running this sequentially underutilizes multi-core CPUs.
- **Decision**:
  - Hardened `LooseObjectStore::write_object` for thread safety: unique temporary files using nanosecond timestamps (`<oid>.<nanos>.tmp`) prevent race conditions during concurrent object creation, and benign rename collisions (identical content hashed simultaneously) are handled safely.
  - In `cmd_add`, candidate files are discovered during directory traversal, sorted and deduplicated, then read, hashed, written to the loose object store, and converted to `IndexEntry` structs in parallel via Rayon. The resulting entries are sequentially inserted into the index and written to disk atomically.

## Phase 10: Scope Boundary Extensions & Git Parity

### DECISION 023: Command Alias Expansion & INI Config Subcommands
- **Date**: 2026-09-10
- **Context**: Real-world developers rely heavily on built-in Git shorthand aliases (`st`, `co`, `ci`, `br`, `df`, `rb`, `cp`) and custom user-defined shortcuts defined in `.git/config` and `~/.gitconfig` under the `[alias]` section (e.g. `lg = log --oneline --graph`).
- **Decision**:
  - Implemented `expand_aliases` in `crates/cli/src/main.rs` before Clap CLI argument parsing.
  - Checks local `.git/config` and global `~/.gitconfig` `[alias]` sections via `GitConfig::get_alias`.
  - Falls back to built-in shorthand defaults (`st` -> `status`, `co` -> `checkout`, `ci` -> `commit`, `br` -> `branch`, `df` -> `diff`, `rb` -> `rebase`, `cp` -> `cherry-pick`).
  - Added quotes-aware command tokenizer (`tokenize_command`) to split multi-parameter command strings accurately.

### DECISION 024: 100% Exact Rename Detection in Status and Diff
- **Date**: 2026-09-10
- **Context**: When a tracked file is moved or renamed, Git groups the staged deletion and addition together as `renamed: <from> -> <to>` in `git status` and outputs unified diff rename headers (`similarity index 100%`, `rename from <from>`, `rename to <to>`) in `git diff --staged`.
- **Decision**:
  - Extended `StagedChange` enum in `oxidize-index` with `Renamed { from: String, to: String }`.
  - In `compute_status_with_ignore`, compare newly staged index entries against deleted HEAD tree entries. When `head_oid == new_oid`, pair them into a `Renamed` change rather than independent addition and deletion.
  - In `cmd_status`, formatted as `\trenamed:    <from> -> <to>`.
  - In `cmd_diff(staged = true)`, detect matching OIDs across files to synthesize Git-compatible rename headers without extraneous body diffs.

### DECISION 025: Native SSH Transport Client via System `ssh`
- **Date**: 2026-09-10
- **Context**: Daily-driver usage requires cloning, fetching, and pushing over SSH (`git@github.com:org/repo.git` and `ssh://user@host:port/path`).
- **Decision**:
  - Created `oxidize_transport::ssh` module with `is_ssh_url`, `parse_ssh_url`, and `SshClient`.
  - Automatically resolves SSH binary from `GIT_SSH_COMMAND`, `GIT_SSH`, system `PATH`, or standard Windows Git fallback paths (`C:\Program Files\Git\usr\bin\ssh.exe`).
  - Executes remote `git-upload-pack` and `git-receive-pack` commands over SSH stdin/stdout using Git's native `pkt-line` framing and sideband demultiplexing.
  - Wired into `ox clone`, `ox fetch`, `ox pull`, and `ox push`.

### DECISION 026: Git Index Version 4 Format Reader Support
- **Date**: 2026-09-10
- **Context**: Git version 4 index files (`DIRC` version 4) use run-length path prefix compression (varint byte-strip count from previous path + suffix string) and omit 8-byte alignment padding to save 30-50% disk space.
- **Decision**:
  - Added `read_v4_entry` to `IndexEntry` in `oxidize-index` to decode varint strip counts and reconstruct paths from preceding entries without padding.
  - Updated `Index::load_from` to accept version 4 index files alongside version 2 and 3, ensuring full interoperability with Git repositories configured with `index.version = 4`.




