# Prompt: repair the reviewed Oxidize implementation and finish verified parity

Work in `C:\Users\PC\Documents\OXIDIZE`. Improve the existing native Rust Git engine and Ratatui/Crossterm TUI. Preserve the user's dirty working tree. Do not replace core operations with system Git or Lazygit subprocesses. Git is allowed as a fixture builder and independent test oracle in disposable repositories.

Read applicable repository instructions, `docs/review/LAZYGIT_PARITY_IMPLEMENTATION_REVIEW.md`, `docs/review/PARITY_REVIEW_PROBES.rs`, `docs/review/PARITY_REVIEW_PROBES.log`, the original `docs/review/LAZYGIT_PARITY_PROMPT.md`, and `docs/parity/`.

The reviewed snapshot is based on HEAD `a60379b215af0c280eae7e9c7d14ea2d207cf9fe` plus substantial uncommitted implementation. Revalidate named functions against the live checkout. The checkpoint's “100% complete” claim is not acceptance evidence. Existing tests passed, but 12 additional safety/workflow regressions failed and formatting failed.

## First deliverable: make mutation paths safe

Import the supplied review regressions into normal test targets, improving fixture isolation and assertions where appropriate. Keep the assertions about required behavior; do not invert them to approve existing bugs. Run them once, then fix related groups with targeted tests. Preserve the existing suite.

1. **R01/R02/R03 — prevent loss of user files.** Rebase must preflight before force checkout and save recoverable intent before mutation. Ordinary worktree removal must reject dirty/current worktrees, validate reciprocal metadata and resolved paths, and propagate deletion failures. Stash third-parent restoration must refuse conflicting untracked paths, preserve bytes, report failures truthfully, and retain a popped stash unless application is verified clean. Test staged, unstaged, untracked, ignored, and nested-submodule cases; inject failures before and after initial writes.
2. **R04/R07/R08 — repair patch semantics.** Eliminate all out-of-bounds forward/reverse preimage and postimage scans. Introduce a patch identity carrying repository, source side, old/new paths, existence, modes/types, blob/preimage identities, exact bytes/newlines, and snapshot generation. Distinguish deletion from an empty file. Reject externally changed worktrees/indexes and ambiguous repeated context instead of guessing a new location. Preserve unrelated staged and unstaged changes. Add line/range selection only over this verified representation.
3. **R05/R06/R09 — unify correct replay.** Stop lossy decoding in mutation paths. Handle binary/type/add/delete conflicts with original base/ours/theirs objects and true index stages. Derive todo from the current branch ancestry, not the multi-ref display vector. Share action finalization between clean and resumed pick/reword/edit/squash/fixup steps. Preserve parent/message/author semantics and edit stops after conflicts and restarts. Protect final ref updates with the original expected OID; journal transitions before mutations and implement recoverable failures.
4. **R10/R11/R12 — restore repository truth and correct targets.** Propagate corrupt index/object/sequencer/status errors; only default for explicitly supported absent-index states. Tag actions must use the selected tag's name/OID and revalidate at execution. Force-with-lease must derive its expectation from the chosen remote tracking state, never the highlighted local history row. Resolve differing upstream names and retain CAS semantics through actual remote update.

For every fix, exercise the service and the actual App/action route. Add missing key/mouse/footer/palette dispatch tests where routes differ. A manually supplied correct helper argument does not validate that the UI constructs it correctly.

## Second deliverable: repair the architecture where workflows require it

Extract native operations into shared services below both CLI and TUI, incrementally by complete workflow. Carry explicit repository context and typed outcomes. Keep presentation and ambient cwd out of services. Do not build a broad framework before fixing the reproduced failures.

Use one action registry for contexts, labels, capabilities, enabled reasons, shortcuts, help, confirmation and execution. Capture immutable target identity before confirmation and revalidate on submission. Keep completed-mutation/failed-refresh outcomes separate so users are not encouraged to repeat successful commits.

Extend the existing worker design into cancellable jobs with bounded queues, progress, generation-based stale-result rejection, and shared per-repository mutation coordination. Route fetch, force-with-lease, replay and costly scans/diffs through it. Navigation must remain responsive. Add an explicitly paused fake worker test; App-local flags alone do not protect independent services or external processes.

Replace Windows delete-before-rename and write-only index locking with verified replacement and transaction semantics. Test competing index edits, stale refs, replacement failures, and interrupted multi-file operations. Preflight full selections before the first write and preserve recovery information across partial failure.

Make history and diff loading incremental and deterministic. Bound immutable caches; invalidate mutable views by source generation and repository. Measure first useful frame and navigation latency on controlled fixtures independently from total history load. Viewport rendering alone is not sufficient.

Audit `apply_custom_patch_to_commit`: a non-HEAD target must not silently become a new commit on current HEAD. Implement verified replay of the chosen target and affected descendants, with preview, conflict recovery and abort; safely disable that exact unsupported operation until the semantics exist. Apply the same rule to unsafe submodule force checkout and unsupported root/merge replay.

## Third deliverable: close the remaining product gaps honestly

Reconcile all 128 original feature IDs against live services, visible actions and acceptance evidence. Preserve the denominator. The existing matrix has 58 verified labels, 16 partial, 53 missing and 1 in-progress; these labels are stale and are not a measured parity percentage. Downgrade unsupported verified claims and recognize genuinely completed work with current evidence.

Continue in dependency order through file tree/range selection and partial staging; comparisons/file history; full conflict resolution and replay recovery; stash variants/custom history patch movement; safe worktrees/submodules/networking; configuration/keybindings/editor integrations and recovery; measured scale and terminal polish. Use the original catalog and 12 acceptance journeys as the complete requirement list. Do not count a menu, helper, fake success message, or unrelated test as a completed workflow.

Verify the pinned Lazygit reference and record deliberate interaction differences. Keep native backend coverage separate from optional editor/browser/clipboard/signing/provider capabilities. Explain unsupported features in the UI while their implementation remains incomplete.

## Acceptance and reporting

All 12 review regressions must pass without weakening their safety contracts. Extend them to assert exact worktree bytes, staged entries, ref OIDs, parent chains, messages, operation state and visible results. Include restart after conflict, valid and stale leases, repeated patch context, CRLF/lone CR/no final newline, invalid UTF-8, and dirty worktree/submodule isolation.

Run the fresh binary and workspace gates after coherent cross-cutting changes:

```text
cargo build --workspace --locked
cargo build --bin ox --locked
cargo test --workspace --all-targets --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
```

Use isolated HOME/global/system config and controlled fixture settings. Add native-runtime independence checks and supported Windows/Unix/MSRV checks. Exercise real terminal restoration where available; record an explicit unverified platform case when unavailable. Do not perform destructive operations in this project's checkout or push to real remotes to test fixes.

Update the matrix, checkpoint and test evidence at each completed workflow. Continue through actionable work in the active run. If interrupted, leave an exact checkpoint with next executable steps and current failing tests. Final reporting must distinguish repaired regressions, complete user journeys, partial features, skipped/unverified environments and remaining blockers. Never replace missing evidence with another “100% parity” claim.
