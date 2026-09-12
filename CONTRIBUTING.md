# Contributing to Oxidize (`ox`)

Thank you for your interest in contributing to **Oxidize**! We welcome bug reports, feature suggestions, architectural improvements, and code contributions.

Oxidize is built from scratch in pure Rust to provide a high-performance Git implementation with verified bidirectional compatibility against official Git.

---

## Code of Conduct

We are committed to providing a welcoming, inclusive, and harassment-free environment for all contributors. Please review our [Code of Conduct](CODE_OF_CONDUCT.md).

---

## Architectural Principles

Before contributing code, please keep these core tenets in mind:

1. **Byte-for-Byte Interoperability**: Any data structure written by `ox` (blobs, trees, commits, tags, index v2/v4, packfiles v2, pack indices v2, pkt-lines) must be readable by official C Git, and vice-versa.
2. **Zero C/C++ Dependencies**: The core codebase relies solely on safe, pure Rust (with audited, minimal dependencies like `sha1`, `flate2`, `memmap2`, `rayon`, `ratatui`, and `clap`).
3. **Strict Error Handling**:
   - Production crate libraries (`oxidize-core`, `oxidize-index`, `oxidize-pack`, etc.) use typed errors powered by `thiserror`.
   - Never use `unwrap()` or `expect()` in library code. Propagate errors using `?`.
   - The CLI crate (`crates/cli`) uses `anyhow::Result` for user-friendly error boundaries and contextual reporting.
4. **Zero Warnings**:
   - `cargo clippy --all --all-targets -- -D warnings` must pass with zero warnings.
   - `cargo fmt --all -- --check` must pass.
   - `cargo doc --workspace --no-deps` must compile cleanly with zero documentation warnings.

---

## Getting Started

### Prerequisites

- **Rust toolchain** (1.88 or later): [rustup.rs](https://rustup.rs)
- **Git** (version 2.30+): Used as the ground-truth oracle for differential integration tests.

### Clone & Build

```bash
git clone https://github.com/braces157/oxidize.git
cd oxidize

# Build debug binaries
cargo build

# Build optimized release binary
cargo build --release
```

The compiled CLI binary will be located at `target/release/ox` (or `target/release/ox.exe` on Windows).

---

## Running the Test Suite

Oxidize features a rigorous differential integration test suite that executes operations in parallel against both `ox` and official `git`, verifying that SHA-1 hashes, index entries, packfiles, and commit trees match byte-for-byte.

```bash
# Run all workspace unit and differential integration tests
cargo test --all

# Run differential tests with live output
cargo test --test porcelain_compatibility_test -- --nocapture
cargo test --test pack_compatibility_test -- --nocapture
cargo test --test transport_compatibility_test -- --nocapture

# Run the performance benchmark suite
cargo test --test performance_benchmark_test -- --nocapture
```

---

## Workspace Structure

The project is divided into 9 focused crates:

| Crate | Directory | Purpose |
|---|---|---|
| `oxidize-core` | `crates/core` | Git object model (`Blob`, `Tree`, `Commit`, `Tag`), `ObjectId`, `LooseObjectStore` |
| `oxidize-index` | `crates/index` | Binary Index v2 & v4 parser/writer (`DIRC`), stat cache, status engine |
| `oxidize-refs` | `crates/refs` | References (`HEAD`, branches, tags), reflog, revision resolver (`rev-parse`) |
| `oxidize-diff` | `crates/diff` | Myers diff algorithm, unified hunk generation, 3-way line merge |
| `oxidize-pack` | `crates/pack` | Packfile v2, base-128 OFS_DELTA, `.idx` v2 reader/writer, memory-mapped repo store |
| `oxidize-transport` | `crates/transport` | Git `pkt-line` framing, sideband demuxer, Smart HTTP client, native SSH transport |
| `oxidize-config` | `crates/config` | Git INI configuration parser (`.git/config`), `.gitignore` glob matcher |
| `oxidize-tui` | `crates/tui` | Interactive Terminal UI dashboard powered by Ratatui & Crossterm |
| `ox` | `crates/cli` | Unified CLI dispatcher, subcommand handlers, and command aliases |

---

## Submitting Pull Requests

1. **Fork and Branch**: Create a feature branch off `master`:
   ```bash
   git checkout -b feat/my-new-feature
   ```
2. **Write Tests**: Add differential integration tests in `tests/` verifying interoperability with official `git`.
3. **Verify Linting & Formatting**:
   ```bash
   cargo fmt --all
   cargo clippy --all --all-targets -- -D warnings
   cargo doc --workspace --no-deps
   ```
4. **Commit**: Use clear, conventional commit messages (e.g. `feat(index): ...`, `fix(transport): ...`, `perf(pack): ...`, `docs: ...`).
5. **Open a PR**: Submit a pull request on GitHub. Our CI suite will automatically validate tests across Linux, macOS, and Windows.
