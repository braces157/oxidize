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
