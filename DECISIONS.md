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

