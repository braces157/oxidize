# Changelog

All notable changes to the **Oxidize** project will be documented in this file.

## [Unreleased]

### Phase 1: Scaffolding (Completed)
- Set up Cargo workspace layout with 9 crates: `core`, `index`, `refs`, `diff`, `pack`, `transport`, `config`, `cli`, `tui`.
- Established `thiserror` typed error hierarchy per crate and `anyhow` CLI boundary.
- Implemented `ObjectId` (20-byte SHA-1), Git object data structures (`Blob`, `Tree`, `Commit`, `Tag`), and loose object store foundation in `oxidize-core`.
- Defined full CLI subcommand routing in `ox` binary with `clap`.
- Set up initial documentation (`README.md`, `DECISIONS.md`, `CHANGELOG.md`).

### Phase 2: Object Model & Content Store (Completed)
- Implemented full canonical serialization/deserialization for Blob, Tree, Commit, Tag with exact Git headers.
- Implemented directory upward search (`find_git_dir`) and prefix lookup with ambiguity detection in `LooseObjectStore`.
- Implemented and verified plumbing commands: `ox hash-object` (file & stdin, `-w`), `ox cat-file` (`-p`, `-t`, `-s`), `ox ls-tree` (recursive & long), `ox mktree`, `ox init`.
- Differential integration test suite (`tests/object_compatibility_test.rs`) passing byte-for-byte against official `git`.

### Phase 3: Index & Staging Area (Completed)
- Implemented binary Git index format v2 reader and atomic writer (`.git/index.lock` with SHA-1 checksum).
- Implemented `IndexEntry` metadata extraction from filesystem stat cache and padding rules.
- Implemented status calculation engine (`compute_status`) diffing HEAD tree vs index vs working tree.
- Implemented `write-tree` converting flat index entries into nested `Tree` objects.
- Added CLI commands: `ox add`, `ox status`, `ox ls-files` (`-s`), `ox update-index`, and `ox write-tree`.
- Differential integration test suite (`tests/index_compatibility_test.rs`) passing against official `git`.

### Phase 4: Core Porcelain Loop (Completed)
- Implemented Myers diff algorithm and unified diff formatter with hunk generation in `oxidize-diff`.
- Implemented reference storage (`RefStore`), atomic updates, reflog tracking, and revision resolver (`rev-parse`) in `oxidize-refs`.
- Implemented core porcelain commands: `ox commit` (with automated signature & tree generation), `ox log` (`--oneline`, `--graph`), `ox diff` (working tree and `--staged`), `ox rev-parse`, and `ox commit-tree`.
- Differential integration test suite (`tests/porcelain_compatibility_test.rs`) verifying two-way commit history and diff interoperability with official `git`.
