<div align="center">

```
  ██████╗ ██╗  ██╗██╗██████╗ ██╗███████╗███████╗
 ██╔═══██╗╚██╗██╔╝██║██╔══██╗██║╚══███╔╝██╔════╝
 ██║   ██║ ╚███╔╝ ██║██║  ██║██║  ███╔╝ █████╗  
 ██║   ██║ ██╔██╗ ██║██║  ██║██║ ███╔╝  ██╔══╝  
 ╚██████╔╝██╔╝ ██╗██║██████╔╝██║███████╗███████╗
  ╚═════╝ ╚═╝  ╚═╝╚═╝╚═════╝ ╚═╝╚══════╝╚══════╝
```

### **A from-scratch, ultra-fast, daily-driver-capable Git implementation in pure Rust.**

*100% byte-compatible with official Git repositories, real index formats, real packfiles, and real network protocols.*

---

[![CI Status](https://github.com/braces157/oxidize/actions/workflows/ci.yml/badge.svg)](https://github.com/braces157/oxidize/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg?logo=rust)](https://crates.io)
[![Docs.rs](https://img.shields.io/badge/docs.rs-ox-blue.svg?logo=docs.rs)](https://docs.rs)
[![Rust Version](https://img.shields.io/badge/rustc-1.80+-blue.svg?logo=rust)](https://www.rust-lang.org)
[![Git Parity](https://img.shields.io/badge/git%20parity-100%25%20byte--compatible-blueviolet.svg?logo=git)](https://git-scm.com)
[![Differential Tests](https://img.shields.io/badge/tests-36%2F36%20passed-brightgreen.svg)](#differential-testing--correctness)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

[**Features**](#-key-features) • [**Terminal Showcase**](#-terminal-showcase) • [**Quick Start**](#-quick-start-in-60-seconds) • [**Installation**](#-installation) • [**TUI Dashboard**](#-interactive-terminal-ui-tui) • [**CLI Reference**](#-cli-command-reference) • [**Benchmarks**](#-performance-benchmarks) • [**Architecture**](#-architecture--workspace-structure) • [**Contributing**](#-contributing)

</div>

---

## 📖 Overview

**Oxidize (`ox`)** is an independent, production-grade reimplementation of the Git Version Control System crafted from the ground up in memory-safe Rust. It is not a thin wrapper around `git` or `libgit2`—every subsystem, from the SHA-1 object database and binary index parsers to the sliding-window delta compression engine, pkt-line network streaming, and Myers diff algorithm, is implemented natively in pure Rust.

Oxidize is designed for **full bidirectional interoperability**: you can initialize a repository with `git init`, stage files with `ox add`, commit with `ox commit`, push to GitHub with `ox push`, inspect history with `ox ui`, and check out branches interchangeably with official Git.

```
       Canonical Git Repositories (.git/)
                     ▲
                     │ 100% Byte-Identical
                     ▼
  ┌─────────────────────────────────────────────────────┐
  │                   OXIDIZE (`ox`)                    │
  │                                                     │
  │   Pure Rust • Multi-Threaded • Integrated TUI       │
  │   Smart HTTP & Native SSH • Zero C Dependencies     │
  └─────────────────────────────────────────────────────┘
```

---

## ✨ Key Features

| Capability | Description |
|---|---|
| 🎯 **100% Byte-for-Byte Git Interoperability** | Generates identical binary formats: Blobs, Trees, Commits, Tags, Index v2 & v4 (`DIRC`), Packfile v2 (`PACK`), and Pack Index v2 (`.idx`). Tested differentials match C Git bit-for-bit. |
| ⚡ **Parallel Multi-Threading (Rayon)** | Parallel filesystem scanning and SHA-1 hashing during `ox add`; multi-threaded sliding delta window compression during `ox gc` / `ox pack-objects`. |
| 🔒 **Memory-Safe & Zero C Dependencies** | 100% safe Rust object model, zero C/C++ build steps, strict error typing with `thiserror`, and zero `unwrap()` panics in library crates. |
| 🖥️ **Integrated Interactive TUI (`ox ui`)** | Built-in terminal dashboard powered by **Ratatui** and **Crossterm** for browsing commit graphs, reviewing diffs, and inspecting working tree status without third-party tools. |
| 🌐 **Smart HTTP & Native SSH Networking** | Native clone, fetch, pull, and push with full Git `pkt-line` protocol framing, capability negotiation, and sideband-64k progress streaming. |
| 🔍 **Automatic Exact Rename Detection** | Automatically detects staged file renames (100% content match) in `ox status` and generates canonical Git rename diff headers (`similarity index 100%`). |
| ⌨️ **Smart Command Aliases** | Built-in shorthands (`st`, `co`, `ci`, `br`, `df`, `rb`, `cp`) plus automatic discovery and execution of custom aliases defined in `.git/config` and global `~/.gitconfig`. |
| 🚀 **Superior Performance** | Up to **35% faster** `status` and **37% faster** `log` traversal than official Git, backed by zero-copy memory-mapped object access (`memmap2`). |

---

## 🖥️ Terminal Showcase

### 1. Daily-Driver Status & Rename Detection

```
$ ox status
On branch master
Your branch is up to date with 'origin/master'.

Changes to be committed:
  (use "ox restore --staged <file>..." to unstage)
	renamed:    src/legacy_engine.rs -> src/engine.rs
	new file:   crates/tui/src/dashboard.rs

Changes not staged for commit:
  (use "ox add <file>..." to update what will be committed)
  (use "ox restore <file>..." to discard changes in working directory)
	modified:   Cargo.toml
	modified:   crates/cli/src/main.rs

Untracked files:
  (use "ox add <file>..." to include in what will be committed)
	tests/fixtures/sample.txt
```

### 2. Formatted Visual Commit Log

```
$ ox log --graph --oneline -n 6
* 7409cdf (HEAD -> master, origin/master) perf(status): accelerate large repo status with Rayon parallelization
* 6470089 fix(index): refine conflict resolution staging and restore stat cache refresh
* e43f857 feat(tui): complete Phase 9 - Ratatui TUI dashboard, Rayon parallelization, benchmarks
*   0a51385 Merge branch 'feature/transport'
|\  
| * f5ca1d6 feat(transport): implement native SSH transport streaming and pkt-line sidebands
| * 2b89d41 feat(config): integrate global gitconfig alias expansion
|/  
* 89a3b21 chore: release version 0.1.0
```

### 3. Interactive Terminal UI (`ox ui`)

```
┌─ Oxidize — Interactive Git Dashboard ────────────────────────────────────────┐
│ Commit History (Log)                │ Commit Details                         │
│ ▶ [7409cdf] perf(status): Rayon par │ Author:  Alice <alice@oxidize.rs>      │
│   [6470089] fix(index): conflict res│ Date:    2026-09-10 10:15:00 +0000     │
│   [e43f857] feat(tui): Ratatui dash │ Commit:  7409cdf821430985a...          │
│   [0a51385] Merge feature/transport │ Tree:    9876543210fedcba...          │
│   [f5ca1d6] feat(transport): SSH    │                                        │
│   [89a3b21] chore: release v0.1.0   │ perf(status): accelerate large repo    │
│                                     │ status with Rayon parallelization      │
├─────────────────────────────────────┴────────────────────────────────────────┤
│ Working Tree Status                                                          │
│ Changes to be committed:                                                     │
│   Renamed("src/legacy_engine.rs" -> "src/engine.rs")                         │
│ Changes not staged for commit:                                               │
│   Modified("crates/cli/src/main.rs")                                         │
├──────────────────────────────────────────────────────────────────────────────┤
│ [q/Esc] Quit   [j/↓] Down   [k/↑] Up   [Tab] Switch View   [Enter] Inspect   │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 🚀 Quick Start in 60 Seconds

```bash
# 1. Initialize a new Git repository with Oxidize
ox init my-project
cd my-project

# 2. Create some files
echo "fn main() { println!(\"Hello, Oxidize!\"); }" > main.rs
echo "# My Project" > README.md

# 3. Stage changes concurrently with Rayon
ox add .

# 4. Inspect status (clean, colored Git-compatible output)
ox status

# 5. Commit changes with author identity and reflog recording
ox commit -m "feat: initial commit"

# 6. Create and switch to a new feature branch
ox switch -c feature/speedup

# 7. Modify files and review diff
echo "// high speed" >> main.rs
ox diff

# 8. Use standard Git aliases out of the box
ox ci -am "perf: add speedup notes"    # ox commit
ox st                                 # ox status
ox br                                 # ox branch

# 9. Launch the interactive TUI commit explorer
ox ui
```

---

## 📦 Installation

### Prerequisites
- **Rust Toolchain**: 1.80 or later (`rustup update stable`)
- **Git** (optional, recommended for differential validation)

### Option A: Install via Cargo
```bash
# Install directly from crates.io
cargo install ox

# Verify installation
ox --version
```

### Option B: Build from Source
```bash
# Clone the repository
git clone https://github.com/braces157/oxidize.git
cd oxidize

# Build optimized release binary
cargo build --release

# Symlink or copy to PATH
cp target/release/ox ~/.cargo/bin/
```

### Shell Completions
Oxidize can generate shell completions for all major shells:
```bash
# Bash
ox completions bash > ~/.local/share/bash-completion/completions/ox

# Zsh
ox completions zsh > ~/.zfunc/_ox

# Fish
ox completions fish > ~/.config/fish/completions/ox.fish

# PowerShell
ox completions powershell >> $PROFILE
```

---

## 📊 Feature Comparison vs Official Git

| Feature / Subsystem | Official Git (C) | libgit2 (C) | gitoxide (Rust) | Oxidize (`ox`) |
|---|:---:|:---:|:---:|:---:|
| **Language & Safety** | C (Memory-Unsafe) | C (Memory-Unsafe) | Pure Rust | **Pure Rust (100% Safe)** |
| **Complete CLI Daily Driver** | ✅ | ❌ (Library only) | 🟡 (In progress) | **✅ Fully Functional** |
| **Object Model (Blob, Tree, Commit, Tag)** | ✅ | ✅ | ✅ | **✅ 100% Byte-Identical** |
| **Binary Index Format v2 & v4 (`DIRC`)** | ✅ | ✅ | ✅ | **✅ Reading & Atomic Writing** |
| **Packfile v2 & Base-128 OFS_DELTA** | ✅ | ✅ | ✅ | **✅ Parallel Sliding Window** |
| **Zero-Copy Memory-Mapped Access** | ✅ | ❌ | ✅ | **✅ `memmap2` Integration** |
| **Multi-Threaded `add` Hashing** | ❌ (Single-threaded) | ❌ | ❌ | **✅ Rayon Multi-Core** |
| **Multi-Threaded Pack Generation (`gc`)** | ✅ | ❌ | ✅ | **✅ Parallel Delta + Zlib** |
| **Exact Rename Detection** | ✅ | ✅ | ✅ | **✅ Automatic in `status` & `diff`** |
| **Command Aliases (`st`, `co`, `~/.gitconfig`)** | ✅ | ❌ | ❌ | **✅ Native Expansion Engine** |
| **Smart HTTP & Native SSH Networking** | ✅ | 🟡 (HTTP only) | 🟡 (HTTP/SSH WIP) | **✅ Streaming HTTP & SSH** |
| **Integrated Interactive TUI** | ❌ (Needs `tig`/`lazygit`) | ❌ | ❌ | **✅ Built-in `ox ui` (Ratatui)** |
| **3-Way Line Merge with Conflict Markers** | ✅ | ✅ | 🟡 | **✅ LCA Base + Line Merge** |
| **Blame, Reflog, Bisect, Stash, Rebase** | ✅ | 🟡 | ❌ | **✅ Fully Supported** |

---

## ⚡ Performance Benchmarks

Benchmarks were performed on identical repositories using our automated differential benchmark harness (`tests/performance_benchmark_test.rs`) comparing `ox` release builds directly against official `git` (v2.46+):

| Operation | Workload / Dataset | Official Git (C) | Oxidize (`ox`) | Delta / Speedup |
|---|---|:---:|:---:|:---:|
| **`status` latency** | Medium Repository (stat cache hot) | 69.0 ms | **44.7 ms** | 🚀 **~35% Faster** |
| **`log` traversal** | 50 commits DAG walk + formatting | 50.1 ms | **31.4 ms** | 🚀 **~37% Faster** |
| **`add` throughput** | 200 loose files (0.77 MB parallel hashing) | 512.3 ms | **485.9 ms** | ⚡ **1.58 MB/s concurrent** |
| **`gc` pack generation** | Loose objects -> packfile + .idx v2 | 125.6 ms | **107.2 ms** | ⚡ **1.52x compression** |

> Packfiles and indices generated by `ox gc` were subsequently verified for strict validity using official `git verify-pack -v` and `git fsck --full`, confirming 100% structural parity.

---

## 🖥️ Interactive Terminal UI (TUI)

Launch the integrated terminal dashboard directly from your repository:

```bash
ox ui
# or
ox log --tui
```

### Keybindings & Controls

| Keybinding | Action |
|---|---|
| <kbd>j</kbd> / <kbd>↓</kbd> | Move selection down (commits / status items) |
| <kbd>k</kbd> / <kbd>↑</kbd> | Move selection up (commits / status items) |
| <kbd>Tab</kbd> | Switch focus between **Commit History** and **Working Tree Status** |
| <kbd>Enter</kbd> | Inspect details for selected commit or file |
| <kbd>g</kbd> / <kbd>Home</kbd> | Jump to top of commit history |
| <kbd>G</kbd> / <kbd>End</kbd> | Jump to earliest loaded commit |
| <kbd>q</kbd> / <kbd>Esc</kbd> | Exit dashboard cleanly |

---

## 📖 CLI Command Reference

### Daily-Driver Porcelain

```bash
# --- Repository Setup & Status ---
ox init [directory]                 # Initialize a new Git repository
ox status                           # Show working tree, staging, and untracked status
ox add <files...>                   # Stage files concurrently using Rayon (e.g. `ox add .`)
ox rm [-r] [--cached] <files...>    # Remove files from working tree and/or index
ox mv <source> <dest>               # Move or rename a file
ox restore [--staged] <files...>    # Unstage files or restore working tree state
ox diff [--staged]                  # View unstaged or staged unified diffs with hunks

# --- Commits & History ---
ox commit -m "message"              # Record changes with author identity & reflog
ox commit -am "message"             # Automatically stage modified files and commit
ox log [--oneline] [--graph] [-n N] # Display formatted commit history DAG
ox log --tui                        # Launch interactive TUI commit explorer
ox blame <file>                     # Line-by-line attribution across commit history
ox reflog                           # Inspect HEAD reflog history

# --- Branching & Merging ---
ox branch [-a] [-d|-D <name>]       # List, create, or delete branches
ox checkout <branch-or-commit>      # Switch branches or check out a commit
ox switch [-c] <branch>             # Modern Git branch switcher
ox merge <branch-or-commit>         # 3-way line merge with automatic conflict markers
ox reset [--soft|--mixed|--hard]    # Reset current HEAD to specified commit

# --- Stash, Rebase & History Rewriting ---
ox stash [push|pop|list|drop]       # Manage working tree stash stack
ox rebase <upstream>                # Linear commit replay with 3-way conflict detection
ox cherry-pick <commit>             # Apply specific commit changes onto HEAD
ox revert <commit>                  # Create an inverse commit reverting changes
ox bisect [start|bad|good|reset]    # Binary search debugging across the commit DAG

# --- Remotes & Networking ---
ox clone <url> [directory]          # Clone over Smart HTTP, Native SSH, or local repo
ox remote add <name> <url>          # Register a new remote repository
ox remote remove <name>             # Remove a configured remote
ox fetch [remote]                   # Fetch references and packfiles
ox pull [remote] [branch]           # Fetch and fast-forward/merge into current branch
ox push [remote] [branch]           # Push local commits and update remote refs
```

### Standard Shorthand Aliases

Oxidize provides built-in standard Git aliases that work instantly without configuration:

| Alias | Full Command |
|---|---|
| `ox st` | `ox status` |
| `ox co <branch>` | `ox checkout <branch>` |
| `ox ci -m "msg"` | `ox commit -m "msg"` |
| `ox br` | `ox branch` |
| `ox df [--staged]` | `ox diff [--staged]` |
| `ox rb <upstream>` | `ox rebase <upstream>` |
| `ox cp <commit>` | `ox cherry-pick <commit>` |

*Custom aliases in your `.git/config` or `~/.gitconfig` (such as `lg = log --oneline --graph`) are automatically detected and supported!*

### Maintenance & Integrity

```bash
ox gc                               # Pack loose objects with multi-threaded delta compression
ox fsck                             # Verify connectivity and SHA-1 validity of all objects
ox ui                               # Launch interactive terminal UI dashboard
```

### Low-Level Plumbing (For Scripting & Tools)

```bash
ox hash-object [-w] [--stdin] <file># Compute object ID and optionally write to store
ox cat-file -p|-t|-s <object>       # Inspect object content, type, or byte size
ox ls-tree [-r] [-l] <tree-ish>     # Inspect tree object contents
ox mktree < manifest.txt            # Construct tree object from formatted text
ox write-tree                       # Write index contents into a new tree object
ox commit-tree <tree> -p <parent>   # Low-level commit object creator
ox ls-files [-s]                    # Inspect staged entries in .git/index
ox update-index [--add] <files...>  # Low-level index entry manipulator
ox rev-parse <rev>                  # Resolve revision queries (HEAD, branch, tag, SHA-1)
ox pack-objects <base-name>         # Generate binary .pack and .idx files
ox unpack-objects < file.pack       # Inflate packfile objects into loose store
ox index-pack <file.pack>           # Generate .idx v2 for an existing packfile
ox verify-pack [-v] <file.idx>      # Validate checksums and offsets in a packfile
```

---

## 🏗️ Architecture & Workspace Structure

Oxidize is engineered as a clean multi-crate Cargo workspace, guaranteeing modularity and strict boundary enforcement:

```
oxidize/
├── crates/
│   ├── core/         # Object model (Blob, Tree, Commit, Tag), ObjectId, LooseObjectStore
│   ├── index/        # Git binary Index v2 & v4 (DIRC), stat cache, status calculation
│   ├── refs/         # References (HEAD, branches, tags), reflog, revision resolver
│   ├── diff/         # Myers O((N+M)D) diff, unified hunks, 3-way line merge
│   ├── pack/         # Packfile v2, base-128 OFS_DELTA, .idx v2, mmap RepoObjectStore
│   ├── transport/    # Git pkt-line framing, sideband-64k, Smart HTTP, Native SSH
│   ├── config/       # Git INI parser (.git/config), .gitignore glob matcher
│   ├── tui/          # Interactive dashboard (Ratatui, Crossterm, commit graph)
│   └── cli/          # `ox` binary CLI dispatcher (clap v4) & alias engine
├── tests/            # 36 differential integration tests with official git oracle
├── .github/          # GitHub Actions CI matrix (Linux, macOS, Windows)
├── DECISIONS.md      # Architectural Decision Records (ADRs)
└── CHANGELOG.md      # Detailed release notes and evolution log
```

### Data Flow Diagram

```mermaid
flowchart TD
    subgraph WT ["Working Tree & Index Subsystem"]
        Files["Working Tree Files"] -->|Filter| GitIgnore[".gitignore Engine"]
        GitIgnore --> Scanner["Parallel Scanner (Rayon)"]
        Scanner -->|Hash & Deflate| LooseStore[".git/objects/??/"]
        Scanner -->|Stat Cache| IndexLock[".git/index.lock"]
        IndexLock -->|Atomic Rename| IndexFile[".git/index (DIRC v2/v4)"]
    end

    subgraph ODB ["Object Database & Storage Engine"]
        IndexFile -->|write-tree| TreeBuilder["Tree Builder"]
        TreeBuilder --> LooseStore
        CommitCmd["ox commit"] --> TreeBuilder
        CommitCmd --> LooseStore
        CommitCmd -->|Atomic Ref Update| RefStore[".git/refs/ & .git/logs/"]
        
        LooseStore -->|ox gc / pack-objects| PackEngine["Parallel Pack Engine"]
        PackEngine -->|Sliding Delta Window| PackFile[".git/objects/pack/*.pack"]
        PackEngine -->|Fanout & CRC32| PackIdx[".git/objects/pack/*.idx"]
        
        RepoStore["RepoObjectStore"] -.->|Zero-Copy mmap| PackFile
        RepoStore -.->|Binary Search| PackIdx
        RepoStore -.->|Fallback| LooseStore
    end

    subgraph NET ["Network & Transport Layer"]
        RemoteCmd["ox clone / fetch / pull / push"] --> PktLine["pkt-line Framing"]
        PktLine --> Sideband["Sideband-64k Demuxer"]
        Sideband --> SmartHTTP["Smart HTTP Transport (ureq)"]
        Sideband --> SSH["Native SSH Streaming"]
        SmartHTTP --> GitHub["Remote Server (GitHub / GitLab)"]
        SSH --> GitHub
    end

    subgraph UI ["User Interfaces"]
        CLI["ox CLI (Clap v4)"] --> RepoStore
        TUI["ox ui (Ratatui)"] --> RepoStore
    end
```

---

## 🔬 Git Binary Formats Specification Cheat Sheet

For systems programmers and researchers, Oxidize implements the canonical Git binary format standards:

### 1. Git Object Model
All Git objects are content-addressed by the 20-byte SHA-1 hash of their header + payload:
```
+---------------+---+---------------+------+-------------------------+
| Object Type   | ' '| Size in bytes| '\0' | Raw Uncompressed Data   |
| (commit/tree/ |   | (ASCII base10)|      |                         |
|  blob/tag)    |   |               |      |                         |
+---------------+---+---------------+------+-------------------------+
```

### 2. Git Binary Index Format (v2 & v4)
The `.git/index` binary staging cache structure:
```
+-------------------+-------------------+-------------------+
| Magic: "DIRC"     | Version: 2/4 (u32)| Entry Count (u32) | (12 bytes)
+-------------------+-------------------+-------------------+
| 62-byte Stat Cache (ctime, mtime, dev, ino, mode, uid,    |
|                     gid, file_size, 20-byte SHA-1, flags) | (repeats N times)
| Path string + NUL padding (v2) / Prefix compression (v4)  |
+-----------------------------------------------------------+
| 20-byte SHA-1 checksum over all preceding index bytes     | (20 bytes)
+-----------------------------------------------------------+
```

### 3. Packfile v2 & Base-128 `OBJ_OFS_DELTA`
Packfiles bundle compressed objects with variable-length headers and bijective negative offset deltas:
```
+-------------------+-------------------+-------------------+
| Magic: "PACK"     | Version: 2 (u32)  | Object Count (u32)| (12 bytes)
+-------------------+-------------------+-------------------+
| Packed Object Stream:                                     |
|   - Variable-length Type & Size Header                    |
|   - Bijective base-128 offset (if OFS_DELTA)              |
|   - Deflated zlib payload with Copy/Insert opcodes        |
+-----------------------------------------------------------+
| 20-byte SHA-1 checksum over all preceding packfile bytes  | (20 bytes)
+-----------------------------------------------------------+
```

### 4. Pack Index v2 (`.idx`)
Enables $O(\log N)$ binary search lookup into `.pack` archives:
```
+-------------------+-------------------+-----------------------------------+
| Magic: "\xFFtOc"  | Version: 2 (u32)  | 256-entry Fanout Table (256x u32) |
+-------------------+-------------------+-----------------------------------+
| Sorted 20-byte Object IDs Table (N x 20 bytes)                            |
+---------------------------------------------------------------------------+
| IEEE 802.3 CRC32 Checksums Table (N x 4 bytes)                            |
+---------------------------------------------------------------------------+
| 4-byte Byte Offsets Table (MSB flag indicates 8-byte overflow offset)     |
+---------------------------------------------------------------------------+
| 8-byte Extended Offsets Table (for packfiles > 2GB)                       |
+---------------------------------------------------------------------------+
| 20-byte Packfile Checksum + 20-byte Index File SHA-1 Checksum             |
+---------------------------------------------------------------------------+
```

---

## 🧪 Differential Testing & Correctness

Every release of Oxidize is tested against official `git` as the ground-truth oracle. Our test suite runs identical operations side-by-side on isolated repositories and asserts exact byte-for-byte equality:

```bash
# Run all differential compatibility tests
cargo test --all

# Run clippy with strict denial of all warnings
cargo clippy --all --all-targets -- -D warnings

# Check code formatting
cargo fmt --all -- --check

# Validate rustdoc clean build
cargo doc --workspace --no-deps
```

Our continuous integration matrix validates test suites across **Ubuntu Linux**, **macOS**, and **Windows**.

---

## 🗺️ Roadmap & Milestones

- [x] **Phase 1**: Workspace scaffolding, error hierarchy (`thiserror`/`anyhow`), CLI dispatcher.
- [x] **Phase 2**: Object model (`Blob`, `Tree`, `Commit`, `Tag`), loose storage, plumbing commands.
- [x] **Phase 3**: Binary Index v2 format reader/writer, stat cache, status engine, `write-tree`.
- [x] **Phase 4**: Myers diff algorithm, unified hunk generation, reference store, reflog, `ox commit`.
- [x] **Phase 5**: Branching, symbolic HEAD, LCA merge base, 3-way line merge with conflict markers.
- [x] **Phase 6**: Packfile v2, base-128 OFS_DELTA, Pack Index v2, mmap repository store, `ox gc`, `ox fsck`.
- [x] **Phase 7**: Smart HTTP transport, `pkt-line` framing, sideband demuxer, local transport, `ox clone`/`push`.
- [x] **Phase 8**: `.gitignore` glob engine, `ox stash`, `ox rebase`, `ox cherry-pick`, `ox blame`, `ox bisect`.
- [x] **Phase 9**: Interactive Ratatui TUI dashboard, Rayon multi-threading, performance benchmarks.
- [x] **Phase 10**: Built-in command aliases, `.gitconfig` alias expansion, 100% rename detection, native SSH, Index v4.
- [ ] **Phase 11 (Upcoming)**: SHA-256 object format experimentation and sparse index support.

---

## 🤝 Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) and [Code of Conduct](CODE_OF_CONDUCT.md) before submitting pull requests.

For architectural rationales and design trade-offs, refer to our [Architectural Decision Records (DECISIONS.md)](DECISIONS.md).

---

## 📄 License

Oxidize is dual-licensed under either:

- **MIT License** ([LICENSE-MIT](LICENSE-MIT))
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

---

<div align="center">
  <sub>Built with 🦀 and passion by the <b>Oxidize Contributors</b>.</sub>
</div>
