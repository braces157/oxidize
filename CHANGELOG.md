# Changelog

All notable changes to the **Oxidize** project will be documented in this file.

## [Unreleased]

### Phase 1: Scaffolding (In Progress)
- Set up Cargo workspace layout with 9 crates: `core`, `index`, `refs`, `diff`, `pack`, `transport`, `config`, `cli`, `tui`.
- Established `thiserror` typed error hierarchy per crate and `anyhow` CLI boundary.
- Implemented `ObjectId` (20-byte SHA-1), Git object data structures (`Blob`, `Tree`, `Commit`, `Tag`), and loose object store foundation in `oxidize-core`.
- Defined full CLI subcommand routing in `ox` binary with `clap`.
- Set up initial documentation (`README.md`, `DECISIONS.md`, `CHANGELOG.md`).
