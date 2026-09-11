# Lazygit parity implementation review — 2026-09-11

The implementation is **not ready to claim full parity**. Existing checks pass except formatting, but 12 additional behavioral regressions fail, including loss of uncommitted work. Fix mutation safety before expanding the feature surface.

Reviewed the dirty working tree based on `a60379b215af0c280eae7e9c7d14ea2d207cf9fe`, not just committed HEAD. Application source and pre-existing user changes were left intact. New artifacts are this review, the fix prompt, probe source, and evidence logs.

## Confirmed findings

All R01–R12 below have executable reproductions in [PARITY_REVIEW_PROBES.rs](PARITY_REVIEW_PROBES.rs). The tests assert required behavior; their failures are evidence of defects, not successful acceptance tests. See [captured output](PARITY_REVIEW_PROBES.log).

### R01 — P1: Rebase startup overwrites uncommitted changes

Location: `crates/tui/src/ops.rs:3109–3127`, `start_interactive_rebase`.

Startup calls `checkout_tree_and_update_index(..., true)` before saving sequencer state. The force flag bypasses dirty/collision preflight. A fixture with an ordinary tracked edit lost `UNCOMMITTED USER WORK`, ended with the previously committed bytes, and returned `Ok(Finished)`. The rebase modal submits directly to this function, so this is a reachable workflow, not an unused helper. Recovery metadata is also written after checkout and HEAD detachment, leaving a failure window before recovery exists.

Fix: validate the plan, operation state, dirty index/worktree, and untracked collisions before mutation; persist recoverable intent first. Use expected-original ref checks at finalization. Test failed startup and interrupted transitions, not only clean replay and abort.

### R02 — P1: Removing a dirty, unlocked worktree deletes user files

Location: `crates/tui/src/ops.rs:4236–4259`, `remove_worktree`.

The function checks the administrative `locked` file, but never checks staged, unstaged, untracked, ignored, or submodule changes. With `force=false`, a linked worktree containing a modified tracked file and a new untracked file was recursively removed and returned `Ok(())`. Deletion errors are discarded, and metadata removal proceeds independently.

Fix: resolve and validate worktree identity and reciprocal administrative paths; refuse dirty/current worktrees under ordinary removal. Separate dirty-force from unlock choices. Preserve metadata and return actionable failure if filesystem removal fails.

### R03 — P1: Applying a third-parent stash overwrites an untracked collision

Location: `crates/tui/src/ops.rs:1379–1398`, `apply_stash`.

Restoring the third-parent tree writes files without checking existing paths and ignores write errors. A Git-created `stash --include-untracked` containing `local.txt` overwrote a newly created `local.txt` with different contents and returned `Ok(true)`. Because `pop_stash` drops on clean apply, this false success also affects pop retention.

Fix: preflight all tracked and untracked restore paths before writes, preserve existing different bytes, represent partial/conflicted application explicitly, and only drop the exact stash identity after a verified clean apply. Add rejection/write-failure and pop-retention tests.

### R04 — P1: Stale short patches panic instead of rejecting

Location: `crates/diff/src/patch.rs:359–360` and `459–460`; also the analogous postimage scans.

The inclusive range `0..=len.saturating_sub(pre_image.len())` still executes at index zero when the preimage is longer than the target. Slicing `0..pre_image.len()` then panics. Both forward and reverse application panic when a three-line hunk is applied to an externally emptied file. This affects interactive discard/unstage and patch application.

Fix: bound every candidate slice by the actual target length and return a typed stale/preimage error. Test forward/reverse, empty/short targets, and short postimages. Do not fix this by catching panics in the UI.

### R05 — P1: Replay silently converts distinct binary bytes into the same text

Location: `crates/tui/src/ops.rs:3025–3050`; lossy helper at `69–75`.

The replay merge converts all three blobs through `String::from_utf8_lossy` before semantic decisions. Base `FF 0A`, ours `FE 0A`, and theirs `FD 0A` were treated as identical replacement-character text: replay reported no conflict and wrote `EF BF BD 0A`. The original bytes were lost.

Fix: merge entries by presence, mode, type, and raw OID/bytes first. Only use text merge on supported text; preserve raw objects in real conflict stages for binary/type conflicts. Object read failure must be an error, not an empty string. Cover modify/delete, empty-versus-absent, executable/symlink/gitlink modes as well.

### R06 — P1: Rebase preparation includes commits from another branch

Location: `crates/tui/src/app.rs:3743–3764`, `open_rebase_todo_modal`.

The graph contains commits from multiple refs. The rebase plan is created by reversing the visible vector prefix through the selected row, rather than computing the current branch's ancestry range. With `side` one commit ahead of `main`, selecting the `main` tip puts the side-only commit into main's rebase todo. Submitting this plan can import unrelated changes into rewritten history.

Fix: derive the plan from immutable HEAD/base ancestry, independent of graph ordering, timestamps, filters, and other refs. Validate merge/root policies and show exactly which branch and descendants will change.

### R07 — P1: Staging a deleted-file hunk creates an empty file

Location: `crates/tui/src/ops.rs:231–259`, `stage_hunk`; patch metadata in `crates/tui/src/model.rs:378–394`.

The stage path always writes a blob and adds an index entry. Removing all lines from a deleted tracked file staged `100644 e69de29... f.txt`, the empty blob, instead of removing the entry. A commit therefore retains an empty file. Conversely, empty content alone cannot determine deletion: a real empty file is a different operation.

Fix: carry old/new file existence, paths, modes, types, and object identity with the patch. Support deletion/creation semantics separately from zero-length content, including reverse application and basket operations.

### R08 — P1: Hunk staging ignores a changed worktree snapshot

Location: `crates/tui/src/app.rs:2032–2052`, `crates/tui/src/ops.rs:214–232`.

Staging validates the cached hunk against current index text, but never verifies the current worktree against the worktree used to build the hunk. After an external edit, the fixture successfully staged the old cached `selected` text while the worktree contained `EXTERNAL EDIT`. There is no repository/generation identity in `DiffView`. The patch engine also scans for the first matching preimage elsewhere, which cannot establish the intended selection's identity.

Fix: capture repository, source side, full preimage/postimage identity and generation; reject or explicitly recompute stale selections. Do not silently relocate to repeated context. Preserve selection by path plus staged/unstaged side, not path alone.

### R09 — P1: Conflict continuation changes fixup into a normal commit

Location: `crates/tui/src/ops.rs:3378–3395`, `rebase_continue`.

The clean loop distinguishes pick/reword/edit/squash/fixup. After a conflict, continuation always constructs a new commit with `parents: vec![current_head_oid]` and the replayed item's author/message. A pick followed by a conflicting fixup produced three total commits instead of two after resolution. Squash message/parent semantics and edit-stop behavior need the same action-aware continuation review.

Fix: route clean and conflict-resumed steps through one action-aware finalizer. Persist enough state to resume the original action after restart. Assert resulting parent OIDs, commit count, message, authorship, and pause state.

### R10 — P1: Corrupt index is accepted as an empty index

Location: `crates/tui/src/app.rs:240`, also `1535` and `2096`.

`Index::load_from(...).unwrap_or_default()` hides corruption and read errors. A repository whose index was replaced with invalid bytes loaded successfully. This presents fabricated index/status data to users deciding what to stage, discard, or commit. It is distinct from a legitimate missing index in an unborn repository.

Fix: propagate corruption/permission/object/state errors to a visible unavailable state and block dependent mutations. Default only for precisely supported missing-index cases. Do not turn status computation failure into an empty file list.

### R11 — P2: Tag deletion targets the local branch name

Location: `crates/tui/src/app.rs:3586–3601`, `prompt_delete_selected_tag`.

After checking that the Tags tab is active, the handler calls `selected_branch()` instead of `selected_tag()`. Selecting `release-v1` opened a confirmation to delete tag `main`. Usually deletion fails; if a tag matching that branch exists, confirmation targets the wrong tag.

Fix: capture the selected tag name and OID and validate them at execution. Exercise the actual key/mouse/palette route with distinct local branch/tag names and with a same-name decoy tag.

### R12 — P2: TUI force-with-lease uses the highlighted local commit

Location: `crates/tui/src/app.rs:4191–4196`, `push_force_lease`.

The expected remote OID comes from `selected_commit()`. With the remote still at the recorded tracking OID and a new local commit selected, the TUI rejected a valid push as stale. Existing packet-6 tests call `push_to_remote_ext` with a manually correct lease, so they miss this UI defect. Choosing another history row changes the lease expectation, which should have no connection to selection.

Fix: resolve remote/destination via upstream configuration and capture its recorded expectation, including an explicitly absent ref. Preview the source/destination and lease, preserve the expectation until transport CAS, and execute through the cancellable job path. Test valid leases, stale leases, differently named upstreams, and advancement between advertisement and update.

## Remaining implementation gaps and improvement priorities

The matrix currently contains **128 rows: 58 marked verified, 16 partial, 53 missing, and 1 in-progress**. These are existing document labels, not a newly verified coverage percentage. Some missing rows are stale because helpers were subsequently added; conversely, several verified rows are contradicted by the regressions above. The checkpoint's “100% complete” claim is unsupported in either direction.

Source inspection also identifies work beyond the 12 reproduced defects:

- **Shared services and action dispatch:** CLI still depends on TUI; semantics remain concentrated in a roughly 4,800-line `ops.rs`. Keyboard, mouse, footer, and palette are separate routes. Extract complete service slices and use one capability/confirmation/dispatch contract; avoid merely moving large files.
- **Jobs and scale:** `load_repository` eagerly walks multi-root history on refresh; `commit_diff_cache` is an unbounded map. Only push/pull spawn workers in `app.rs`; `fetch_all` and force-with-lease call network operations synchronously. Inspector slicing is useful, but does not establish asynchronous loading, bounded memory, or cancellation.
- **Patch workspace:** line/range selection, file identity/mode/binary metadata, persistent basket identity, and cross-repository invalidation are incomplete. `apply_custom_patch_to_commit` ignores a non-HEAD target and makes a commit on current HEAD instead of rewriting the selected target. Basket worktree writes happen path by path before full preflight.
- **Replay durability:** save-after-mutation windows remain; final branch updates omit an expected-original OID. `rebase-merge`-shaped files alone do not prove Git-compatible recovery. Root/merge handling, restart, skip/edit semantics, and abort preserving later user work need explicit journeys.
- **Worktree/submodule isolation:** `submodule_update` still calls forced checkout at `ops.rs:4459`. Review preservation of dirty submodules before marking M06/M08 complete. Shared-context discovery does not automatically fix every config/read/write call site.
- **Transactions:** Windows `LockFile::commit` still removes the old target before rename (`core/src/lock.rs:79`), and index locking occurs at write time rather than spanning read/modify/write. Add controlled replacement failure and competing-edit tests before claiming transactional safety.
- **Product coverage:** file trees/range selection, diff search/comparisons, old-commit patch movement, full conflict views/undo, recovery, custom keybindings/configuration, editor integration, and measured performance remain incomplete or unverified. A modal or helper is insufficient evidence for its full catalog item.

These observations are source-grounded follow-up work, not additional fixture-confirmed findings. In particular, this review does not certify every catalog row or every transport/platform capability.

## Verification and limits

On Windows, Rust `1.95.0 (59807616e 2026-04-14)`:

| Check | Result |
|---|---|
| `cargo build --bin ox --locked` | Passed; binary rebuilt before baseline integration tests |
| `cargo build --workspace --locked` | Passed |
| `cargo test --workspace --all-targets --locked` | Passed existing suite |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all -- --check` | Failed; formatting diffs across implementation and packet tests |
| `cargo doc --workspace --no-deps --locked` | Passed; [log](PARITY_REVIEW_DOC_BUILD.log) |
| Additional review regressions | **12 failed / 12**, for the behaviors above |

The extra tests used disposable repositories and local bare remotes only. Git prepared fixtures and inspected refs/index/blob content; native Oxidize helpers and App handlers performed the reviewed operations. The harness sets fixture identities and disables system/global Git config for its Git subprocesses. It is Windows-oriented (`NUL`); adapt that to `/dev/null` and further isolate HOME/environment for cross-platform CI.

To reproduce, copy `PARITY_REVIEW_PROBES.rs` to an unused `crates/tui/tests/parity_review_probes.rs`, then run:

```text
cargo test -p oxidize-tui --test parity_review_probes --locked -- --test-threads=1
```

The temporary Cargo test copy was removed after this review, so the requested review does not leave intentionally failing tests in the normal suite. The preserved source and log remain available for conversion to permanent regressions during the fix task.

No interactive ConPTY session, Unix/MSRV matrix, authenticated HTTP/SSH service, pinned Lazygit checkout validation, fault-injected transaction suite, or full 12-journey acceptance campaign was completed here. Passing existing tests does not establish those properties.

Next action: execute [LAZYGIT_PARITY_FIX_PROMPT.md](LAZYGIT_PARITY_FIX_PROMPT.md), starting with data preservation and the reproduced regressions.
