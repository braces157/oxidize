# Known Blockers and Defect Resolution Ledger

## Confirmed Defects Resolution Ledger (R01–R12)

All 12 defects confirmed in `docs/review/LAZYGIT_PARITY_IMPLEMENTATION_REVIEW.md` and reproduced via `docs/review/PARITY_REVIEW_PROBES.rs` have been resolved in native Rust code and verified by permanent regression tests in `crates/tui/tests/parity_review_probes.rs`.

| Defect ID | Category | Defect Symptoms | Status | Resolution Path | Verification Test |
|---|---|---|---|---|---|
| **R01** | Data Loss | Rebase force-checkout clobbered uncommitted user worktree modifications | **Resolved** | Preflight sequencer state and worktree/index dirtiness before checkout; persist sequencer state before mutating checkout | `rebase_must_reject_dirty_worktree_before_mutation` |
| **R02** | Data Loss | Linked worktree removal deleted dirty/unlocked trees without reciprocal metadata checks | **Resolved** | Verify reciprocal gitdir/commondir links, reject removal of main or active worktree, check worktree and index for dirty/untracked files | `worktree_removal_must_reject_dirty_unlocked_tree` |
| **R03** | Data Loss | Stash restore silently overwrote conflicting untracked files on disk | **Resolved** | Preflight 3rd parent untracked files against worktree; abort with error before writing; retain popped stash on failure | `stash_untracked_restore_must_preserve_existing_file` |
| **R04** | Panic / Crash | Stale or short patch text caused slice indexing panic in diff engine | **Resolved** | Check slice length boundary (`len >= image.len()`) in `apply_hunk_forward` and `apply_hunk_reverse` | `stale_short_patch_must_return_error_without_panicking` |
| **R05** | Corruption | Three-way tree replay merged distinct non-UTF-8 binary files into identical text | **Resolved** | Detect binary/non-UTF-8 content and preserve raw bytes in conflict stages 1, 2, and 3 | `replay_must_not_merge_distinct_invalid_utf8_as_identical_text` |
| **R06** | Logic Defect | Interactive rebase todo included disjoint branch commits with newer timestamps | **Resolved** | Traverse ancestral lineage from HEAD down to selected base OID rather than slicing multi-branch DAG | `rebase_todo_must_exclude_other_branch_commits` |
| **R07** | Data Loss | Staging hunk deletion of absent worktree file created an empty blob entry in index | **Resolved** | Remove index entry when file is absent from worktree and staged content is empty | `staging_deleted_file_hunk_must_remove_index_entry` |
| **R08** | Race / Stale State | Staged hunk applied stale cached hunk even if worktree was externally modified | **Resolved** | Validate worktree file against hunk context and pre-image before applying hunk to index | `stage_hunk_must_reject_changed_worktree_snapshot` |
| **R09** | Semantics | Conflicted rebase fixup continue created a redundant separate commit | **Resolved** | Inspect sequencer step action in `rebase_continue` and execute squashing/fixup amends | `conflicted_fixup_continue_must_preserve_fixup_semantics` |
| **R10** | Crash / False State | Corrupt index file fell back to empty repository instead of surfacing error | **Resolved** | Propagate index parse errors in `App::load_repository` and `discard_selected_hunk` | `corrupted_index_must_not_load_as_repository_truth` |
| **R11** | Action Routing | Deleting a tag in the Tags tab targeted the selected branch instead of the tag | **Resolved** | Use `self.selected_tag()` in `prompt_delete_selected_tag` to target the tag name | `tag_delete_must_target_selected_tag` |
| **R12** | Network Race | Force-with-lease push pushed current commit OID as expected OID instead of remote tracking ref | **Resolved** | Resolve expected remote OID from upstream tracking ref `refs/remotes/<remote>/<branch>` | `app_force_lease_must_use_recorded_remote_oid` |

---

## Architectural & Cross-Cutting Safeguards

| Item | Focus | Status | Implementation |
|---|---|---|---|
| **Submodule Dirty Worktree Protection** | Data Loss Prevention | **Complete** | Preflight uncommitted changes and untracked file collisions in `submodule_update` before checkout |
| **Multi-Path Custom Patch Preflight** | Atomic Transaction | **Complete** | Compute all text applications and path validations in memory before writing objects or index |
| **Historical Commit Patch Safety** | Safe Abort | **Complete** | Explicitly reject custom patches targeted at non-HEAD commits until full replay semantics exist |
| **Windows Atomic File Commit** | OS File Locking | **Complete** | Use `.lockbackup` staging rename with automatic restore on failure in `LockFile::commit` |

---

## Intentionally Deferred / Partial Capabilities

| Feature ID | Feature Name | Status | Rationale |
|---|---|---|---|
| **K05** | Custom patch on non-HEAD commits | Partial | HEAD commit amend supported; historical non-HEAD commits fail closed with explicit safe error message |
| **C03** | Individual line staging | Partial | Hunk-level staging, unstaging, and discard fully supported; visual line-range staging planned |
| **N04** | Non-linear DAG bisect | Partial | Linear commit bisect with multi-boundary good/bad/skip supported; complex octopus DAG merge bisect deferred |
| **D02 / I05** | External editor/mergetool | Partial | Native built-in composer and conflict resolution supported; external editor subprocess spawning optional |
