# Architecture and Product Decisions (ADRs)

## ADR-01: Native Rust Engine Supremacy
- **Decision**: Oxidize retains its independent pure-Rust Git engine. Features in `ox ui` must be powered by Oxidize crates (`oxidize_core`, `oxidize_index`, `oxidize_refs`, `oxidize_diff`, `oxidize_pack`, `oxidize_transport`).
- **Rationale**: The core mission is a native Git engine plus a Lazygit-style TUI. Oxidize is not a subprocess wrapper around system `git` or `lazygit`.
- **Implication**: Any missing Git operations must be built in native Rust services shared across CLI and TUI.

## ADR-02: Strict Separation of Render and Mutation
- **Decision**: Rendering paths (`ui.rs`, `syntax.rs`) must never mutate repository state, write to disk, or spawn background processes.
- **Rationale**: Terminal rendering runs at high frequency during events; side effects cause race conditions and unexpected state corruption.

## ADR-03: Centralized Error and Result Routing
- **Decision**: User-initiated operations via keyboard, mouse, footer, and modals must never discard errors with `let _ = ...`.
- **Rationale**: Users must receive immediate, actionable feedback in the status area (`status_message`) when an operation fails or rejects a precondition. Recoverable modal inputs (such as commit messages or branch names) must be preserved on submission failure.

## ADR-04: Mutation Coordination and Concurrency Guard
- **Decision**: While an asynchronous job (such as push or pull) is actively mutating or interacting with the repository, all concurrent local mutations (staging, commits, checkout, stash) are locked and rejected with clear user notifications.
- **Rationale**: Concurrent access to the index, working tree, and refs without unified locks leads to lost updates or corrupt state.

## ADR-05: Byte-Truthful Diff and Patch Representation
- **Decision**: Never apply patches or stage changes reconstructed from lossy rendered text. All patch operations must use structured byte-preserving hunks with verified pre-images.
- **Rationale**: Line ending variations (CRLF vs LF), trailing spaces, and unicode normalization must not be mangled during partial staging.

## ADR-06: Unified Destructive Action Safeguards and Subtab Isolation
- **Decision**: All destructive operations (file discard, branch deletion, stash drop) must route through explicit user confirmation dialogs (`ActiveModal::Confirm`) displaying target identity and consequences, regardless of whether triggered via keyboard shortcut (`'d'`), footer button, or mouse action. Non-local tabs (Remotes, Tags) are isolated and must never trigger operations on stale local selections.
- **Rationale**: Prevents accidental unrecoverable data loss and guarantees parity between mouse and keyboard interaction models.

## ADR-07: Linear Ancestry for Interactive Rebase Plans
- **Decision**: Interactive rebase todo lists (`App::open_rebase_todo_modal`) must derive candidate commits strictly by walking first-parent/ancestral commit links from `HEAD` down to the selected `onto` base OID, rather than reversing a chronological slice of the global multi-branch DAG commit buffer.
- **Rationale**: Commits on disjoint branches created more recently than the rebase base can share timestamps and appear in chronological slices, causing foreign branch commits to be inappropriately rewritten into the current branch's rebase plan (reproducing defect R06).

## ADR-08: Transactional Atomic File Commits with Backup on Windows
- **Decision**: In `oxidize_core::lock::LockFile::commit`, replacing the target file on Windows must use transactional `.lockbackup` file rotation rather than deleting the target prior to renaming. If replacement fails, the backup is restored.
- **Rationale**: Deleting the destination file before renaming exposes a window where failure (e.g., antivirus locks, transient file system delays) leaves the target file permanently destroyed.

## ADR-09: Multi-Path Preflight Before Custom Patch and Submodule Checkout
- **Decision**: Before executing custom patch application (`apply_custom_patch_to_worktree`, `apply_custom_patch_to_index`) or submodule checkout (`submodule_update`), all target paths, hunk applications, unstaged/staged dirty states, and untracked file collisions must be preflighted in memory. Unsupported operations (such as custom patch applied to historical non-HEAD commits) must fail closed with explicit user-facing errors rather than mutating current HEAD.
- **Rationale**: Prevents partial disk writes leaving dangling objects or corrupt index states when a secondary file in a multi-file patch fails, and protects user worktree modifications in submodules from being overwritten by forced checkouts.

## ADR-10: Binary-Safe Tree Merge and Replay Preservation
- **Decision**: Tree replay merging (`merge_trees_into_index_and_worktree`) must inspect file contents for binary/invalid UTF-8 bytes and avoid lossy UTF-8 string conversions. When binary files differ between branches, both variants must be preserved in conflict stages 1, 2, and 3 without merging corruption.
- **Rationale**: Replay must never merge distinct binary files or mangle non-UTF-8 payloads (defect R05).


