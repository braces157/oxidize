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

### Phase 5: Branching & Merging (Completed)
- Implemented branch reference manipulation (`list_branches`, `create_branch`, `delete_branch`), symbolic and detached HEAD switching.
- Implemented safe working-tree and index checkout engine (`checkout_tree_and_update_index`).
- Implemented LCA merge base discovery (`find_merge_base`) on the commit DAG.
- Implemented 3-way line merge with conflict markers synthesis and automated merge commits.
- Implemented commands: `ox branch`, `ox checkout`, `ox switch`, `ox merge`, and `ox reset` (`--soft`, `--mixed`, `--hard`).
- Differential integration test suite (`tests/branch_merge_compatibility_test.rs`) passing against official `git`.

### Phase 6: Packfiles & Maintenance (Completed)
- Implemented packfile v2 writer (`write_pack`), object reader (`read_pack_object_at`), and unpacker (`unpack_packfile`).
- Implemented delta compression and decompression (`apply_delta`, `create_delta` with copy/insert opcodes).
- Implemented Git bijective base-128 variable-length offset delta encoding (`OBJ_OFS_DELTA`) and `OBJ_REF_DELTA` handling.
- Implemented pack index v2 reader and writer (`PackIndex`, `\xFFtOc`, 256 fanout table, CRC32, 4-byte/8-byte offsets).
- Implemented zero-copy memory-mapped object store (`PackStore`, `RepoObjectStore`) with automatic packfile fallback.
- Added plumbing & porcelain commands: `ox pack-objects`, `ox unpack-objects`, `ox index-pack`, `ox verify-pack`, `ox gc`, `ox fsck`.
- Differential integration test suite (`tests/pack_compatibility_test.rs`) passing against official `git` (verifying pack integrity with `git verify-pack`, `git fsck --full`, and bidirectional commit log inspection).

### Phase 7: Networking & Smart HTTP Transport (Completed)
- Implemented Git pkt-line protocol framing (`encode_pkt_line`, `parse_pkt_line`, flush/delim/response-end) and sideband demultiplexing (`SidebandDemuxer`).
- Implemented smart HTTP negotiation client (`SmartHttpClient` with `ureq`) supporting `git-upload-pack` and `git-receive-pack`.
- Implemented local repository filesystem transport (`resolve_local_path`, `discover_local_refs`, `fetch_local_pack`) supporting both bare and non-bare repos with packed-refs.
- Implemented full Git INI configuration parser and serializer (`GitConfig`) with URL path normalization.
- Introduced `ObjectReader` trait in `oxidize-core` allowing zero-copy tree and index checkout straight from packfiles.
- Added porcelain commands: `ox clone`, `ox fetch`, `ox pull`, `ox push`, `ox remote` (`add`, `remove`).
- Differential integration test suite (`tests/transport_compatibility_test.rs`) passing against official `git` verifying bidirectional clone, push, pull, and remote tracking.

### Phase 8: Advanced Porcelain & UX (Completed)
- Implemented `.gitignore` pattern parser and path matcher (`GitIgnore`, `IgnorePattern`) supporting wildcards, directory-only patterns, negation (`!`), and recursive globs (`**`).
- Integrated `.gitignore` filtering into `ox status` and `ox add .`.
- Added porcelain file management commands: `ox rm` (`--cached`, `-r`, `-f`), `ox mv`, `ox restore` (`--staged`).
- Added tag manipulation command: `ox tag` (list, create lightweight or annotated tags with `-a`/`-m`, delete with `-d`).
- Added working state stashing: `ox stash` (`push`, `pop`, `list`, `drop`) with dual-parent stash commit topology and reflog tracking.
- Added history rewriting commands: `ox rebase` (linear commit replay with 3-way line merge), `ox cherry-pick`, `ox revert`.
- Added history inspection commands: `ox blame` (line attribution via reverse topological commit walk) and `ox reflog` (`.git/logs/HEAD`).
- Added binary search debugging: `ox bisect` (`start`, `bad`, `good`, `reset`) with logarithmic midpoint computation.
- Differential integration test suite (`tests/advanced_porcelain_test.rs`) verifying all Phase 8 commands against official `git`.


