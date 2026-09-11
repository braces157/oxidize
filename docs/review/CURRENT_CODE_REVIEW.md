# Current Oxidize code review for the expansion prompt

Reviewed 2026-09-11. Purpose: make the Lazygit expansion prompt grow the existing native Rust Git engine and TUI. This review changed documentation only.

## Read coverage and snapshot boundary

Read all 60 workspace Rust source/test files in the final reviewed inventory, all 11 Cargo manifests, the new rustfmt configuration, and all three GitHub workflows. This includes complete CLI command bodies, every engine crate, every TUI module, all seven TUI integration-test files, and the twelve root integration-test files. Comments and whitespace were omitted in many displayed reads; truncated outputs were reread in smaller ranges. Generated build files and third-party dependency source are outside this review.

The source was changing during review. An initial isolated snapshot contained 69 files and 27,130 lines. After reading live changes, the final source/config inventory contains 72 files and 29,436 lines, plus three separately read workflow files. `CURRENT_SOURCE_MANIFEST.json` records the final hashes and line counts, including workflows. Source hashes matched the live checkout at the final comparison. Future changes must be reconciled before implementation.

The later additions were explicitly inspected: wired mouse event handling, syntax highlighting and its unit tests, mouse integration tests, `ox fmt`, the formatting help test, repository path normalization, and the Windows relative-path helper. The initial observation that mouse code was unwired is superseded.

## Verification

- Initial snapshot: build the CLI, then run `cargo test --workspace --locked`: **139 passed**.
- Final refreshed snapshot including mouse/syntax/fmt additions: `cargo build -p ox --locked`, then `cargo test --workspace --locked`: **156 passed, zero failed**.
- A clean `cargo test` invocation initially failed because the root integration tests locate `target/debug/ox.exe` and the CLI executable had not yet been built. This is a test setup dependency, not five product regressions. CI already builds the binary before testing.
- Refreshing the isolated snapshot with preserved file timestamps briefly reused an outdated library artifact. Touching only the snapshot source files forced a rebuild; the final result above is from that rebuild.
- Rendered and inspected a synthetic 100×30 Ratatui TestBackend screen using the actual current renderer and syntax highlighter. It showed clipped branch tabs, inspector hints, and footer actions at this size. Existing tests also exercise small and larger viewports. This was a headless inspection, not a live interactive terminal session.
- Did not run remote authenticated operations, cross-platform CI, MSRV, clippy, or a full fuzz campaign during this documentation review. Passing tests do not certify all current Git semantics.

Evidence logs are stored alongside this review as `current-review-build.log` and `current-review-tests.log`.

## Current capabilities and the improvement boundary

| Area | Already implemented | What the prompt should build next |
|---|---|---|
| TUI shell | Five panels, branch/commit subtabs, sidebar/inspector focus, modals, mouse routes, terminal restoration | Shared action routing, visible errors, capability checks, confirmations, search/palette, width-aware interaction |
| Files | Native stage/unstage/discard, stage-all, staged/unstaged/unmerged status, exact-OID rename detection | Correct directory/batch operations, stable side-aware selection, tree view, ranges, byte-preserving partial staging |
| Inspector | Unified text, scrolling, language detection and highlighting | Structured hunks/line coordinates, selection, file drilldown, horizontal/large scrolling, cached viewport rendering |
| Commits | Native creation, HEAD amend, metadata and first-parent diff | Draft persistence, multiline composer, real DAG graph, filters, comparisons, selected-commit editing after replay prerequisites |
| Branches/tags | Local create/checkout/delete, remote/tag lists, packed-aware ref reader | Consistent visible selection, generic packed/nested refs, annotated tags, configured upstream, rename and target menus |
| Replay | CLI merge/reset/rebase/cherry-pick/revert | Shared native sequencer, full conflict stages, durable continue/skip/abort, interactive rebase and safe restart |
| Stash | Two-parent save, list/apply/pop/drop, pop retains conflicts | Correct index restoration choices, third-parent untracked stashes, selected paths/hunks, coordinated ref/log changes |
| Network | Native local/HTTP/SSH clone/fetch/push, TUI background push/pull | Capability plumbing, report validation, progress/cancel/timeout, explicit targets/strategies, lease, prune/tags/refspecs |
| Repository context | Discovery distinguishes worktree/git/common directories and bare repositories | Carry those distinctions through object/ref/index consumers; complete worktree/submodule operations |
| Power tools | CLI blame, bisect, clean, config, aliases, completions, new Rust formatter | Correct DAG bisect, file-history UI, keymaps/integrations, staged-blob and hook semantics |

## Findings that determine the implementation order

These are source-review findings unless described as a test/render result above. The expansion prompt requires focused reproductions before fixes; it does not present all of these as independently reproduced failures.

1. **Rendering can hang on `$`.** In `syntax.rs::tokenize_code`, the outer identifier condition accepts `$` while its consuming loop does not. The branch can emit empty spans forever. Existing highlighting tests do not cover this token. Guarantee scanner progress before broadening highlighting or rendering large diffs.
2. **The input path can hide failures or choose the wrong target.** `lib.rs` and `mouse.rs` discard operation errors. Keyboard branch actions do not consistently constrain the active subtab. Local branch rendering filters a vector that navigation and selection still use more broadly. Centralize dispatch and identity before adding many more commands.
3. **Rendering and mouse geometry disagree on display width.** `ui.rs::build_footer` ignores its width argument and computes ranges from byte lengths. Modal cursor positions count scalar characters rather than graphemes/display cells. The synthetic render visibly clipped footer actions and branch tabs at 100 columns.
4. **Partial staging needs a real mutation representation.** `DiffView` is display text. Existing `Hunk`/Myers primitives are reusable, but unified formatting loses newline distinctions. Never mutate files/index blobs from lossy inspector text.
5. **Whole-file staging already has a directory mismatch.** Status collapses untracked directories to `dir/`; TUI `stage_path` does not recursively stage them. CLI recursive add logic should become a shared native service. Batch staging should use a coordinated index transaction.
6. **Repository truth is inconsistently consumed.** Discovery provides `common_dir`, but stores still root directly at their supplied `git_dir`. TUI discovery/index reads have silent fallback paths. Packed ref enumeration, tag peeling, and upstream resolution are incomplete across frontends.
7. **Replay is substantially less complete than merge.** CLI rebase/cherry-pick/revert use lossy text, file unions, and generated stage-1 conflict blobs. They lack a durable sequencer. Exposing them unchanged would make the TUI promise recovery it cannot deliver.
8. **Mutation coordination is narrower than the UI suggests.** Background jobs guard only push/pull. Index locking starts at write time after callers loaded state. Windows `LockFile::commit` removes the old target before rename. Shared services need preflight, lock/expected-state discipline, and explicit partial-failure results.
9. **Stash apply loses an important outcome.** `App::apply_selected_stash` ignores the native clean/conflict boolean. Ordinary apply/index restoration and third-parent stashes need semantic tests. The CRLF comparison removes all carriage returns, including lone CR bytes.
10. **Network helpers exist, but their composition needs tests.** Fetch capability information is often discarded. Negotiated sideband push reports are passed to a direct report parser, and an unrecognized/missing report currently counts as success. Remote controls must depend on validated transport outcomes.
11. **Performance improvements should target actual hot paths.** Lists already use windows. History loads eagerly, diff selection opens stores/expands trees/reads many blobs, and every render highlights the full diff. Worker requests need generation IDs, bounded caching, cancellation, and visible partial results.
12. **New formatting options need honest semantics.** `fmt --staged` formats working paths selected from staged status, not staged blob snapshots; hook installation overwrites the hook file, and native commit does not thereby gain hook execution. The prompt preserves the new command and adds focused correctness work.

Additional library prerequisites are included in the prompt where relevant: packed-object reads for reset/tag/commit checks; ref create/update/delete semantics; index v4 byte-boundary handling and extension preservation; parser allocation/depth bounds; binary/symlink/gitlink handling; separate author/committer identity; and errors that are currently replaced with empty values. The older F01–F18 “fixed” ledger is useful history, not proof these related paths are complete.

## How the rewritten prompt uses this review

`LAZYGIT_PARITY_PROMPT.md` now opens with seven concrete packets: current interaction blockers, shared native services, structured patch workspace, history/refs, replay/conflicts, stash/network completion, and advanced catalog work. Each identifies current symbols, what to reuse, behavioral gaps, and acceptance journeys.

The original 128 feature IDs remain as the long-term coverage checklist. The persistent checkpoint loop remains for long active runs and resumptions. The first cycle now starts with confirmed current blockers and progresses to partial staging, rather than rebuilding existing Git functionality or issuing a generic roadmap.
