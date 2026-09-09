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



