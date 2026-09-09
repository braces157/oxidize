# 🦀 Oxidize (`ox`)

> A from-scratch, production-quality reimplementation of Git in Rust. Daily-driver capable, byte-compatible with official Git repositories, real index formats, real packfiles, and real Git network protocols.

## Architecture

```
oxidize/
├── crates/
│   ├── core/         # Object model: Blob, Tree, Commit, Tag; LooseObjectStore (SHA-1)
│   ├── index/        # Staging area: binary index format v2/v3, working tree scanner
│   ├── refs/         # HEAD, branches, tags, packed-refs, reflog, rev-parse
│   ├── diff/         # Myers diff, unified diff formatter, 3-way merge + conflict markers
│   ├── pack/         # Packfile v2, OFS/REF delta compression/decompression, .idx v2
│   ├── transport/    # Git pkt-line, smart HTTP transport, Git protocol v2
│   ├── config/       # INI parser (.git/config, ~/.gitconfig), .gitignore pattern engine
│   ├── cli/          # `ox` binary with full porcelain and plumbing CLI commands
│   └── tui/          # Ratatui terminal dashboard (commit graph, diff viewer, status)
├── tests/            # Cross-crate integration & real `git` differential tests
├── fuzz/             # Cargo-fuzz harnesses for index and pack parsers
└── benches/          # Criterion performance benchmarks vs official Git
```

## Quickstart

```bash
# Build the workspace
cargo build

# Run the `ox` CLI
cargo run -p ox -- --version
cargo run -p ox -- --help
```
