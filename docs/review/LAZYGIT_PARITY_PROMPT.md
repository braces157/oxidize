# Oxidize: improve the existing native Rust Git engine and TUI

Copy this document into an implementation task, or instruct the coding agent to read and execute this file. This is a substantial implementation specification with a persistent work loop. The loop operates while the agent is running; the checkpoint files support continuation after interruption. It does not require an infinite shell loop, scheduler, or unattended remote actions.

---

## Mission

Act as the senior Rust engineer and terminal application maintainer for Oxidize. **Oxidize is already an independent Git implementation written in Rust.** Expand its existing `ox ui` interface with Lazygit-style interaction and feature coverage, powered by Oxidize's own Git engine. Preserve and extend the engine and existing working features.

The product objective is: **Oxidize's native Git engine + a comprehensive Lazygit-style TUI.** Lazygit is the reference for user workflows and interaction. Do not rebuild basic Git functionality that already exists, replace the engine, or turn the project into a frontend for the system `git` executable.

This is an instruction to implement and verify the product, not just write another roadmap. Continue through multiple complete feature slices in the same active run. Do not stop after scaffolding, creating a feature list, implementing a single easy action, or rendering a menu of unfinished commands. Every feature counted as complete must be reachable, functional, tested, and documented.

Project location: `C:\Users\PC\Documents\OXIDIZE`.

The existing TUI already has status, files, branches/remotes/tags, commits/reflog, stash, navigation, inspectors, and basic operations. Reuse and extend the current product. The earlier hardening review was followed by commit `1907fcd` and `docs/review/STATUS.md` marking fixes. These are historical starting points, not proof of the current state. Inspect the actual checkout, current instructions, and tests before deciding what remains. Do not repeat already verified work or assume the old defects still exist.

Read these existing documents when available:

- `docs/review/PROJECT_REVIEW.md`
- `docs/review/IMPLEMENTATION_PROMPT.md`
- `docs/review/STATUS.md`
- `README.md`, `docs/ARCHITECTURE.md`, `docs/CLI_REFERENCE.md`, and `DECISIONS.md`
- Applicable `AGENTS.md` files and the current Cargo manifests.

## Source-grounded starting point and first implementation queue

This revision follows a full read of the workspace Rust implementation and tests, including the TUI event loop, state, renderer, operations, mouse handling, and newly added syntax highlighter. See `docs/review/CURRENT_CODE_REVIEW.md` and its source manifest for the review boundary and test evidence. The checkout was changing during review. Revalidate these observations against the current symbols before editing; preserve fixes that have landed since then. Historical line numbers are less useful than the named functions below.

### What already exists and must be extended

- The Ratatui/Crossterm application has five panels: Status, Files, Branches, Commits, and Stash. Branches contains Local/Remotes/Tags; Commits contains Commits/Reflog. Preserve this familiar layout and its existing navigation while adding depth.
- `AppLayout`, `compute_layout`, `window_range`, tab hit ranges, and modal geometry already exist in `ui.rs`. Lists already render visible row windows. Reuse these; the performance problem is eager repository loading and full diff styling, not an absence of all viewport handling.
- `FooterAction` and clickable footer ranges already exist. Evolve these into a common action system shared by keyboard, mouse, footer, help, and command palette. Do not create another competing command enumeration.
- `mouse.rs` is now declared and routed from `lib.rs`. It handles panel/row/tab clicks, double-click actions, right-click staging, contextual scrolling, footer actions, and modal interaction. New `tui_mouse_test.rs` covers many of these paths. Do not implement mouse support from scratch or act on the earlier snapshot's missing-module observation.
- `syntax.rs` already detects many languages and highlights diffs. Improve correctness, fallback behavior, and cost. Do not sell syntax highlighting as an entirely new feature.
- Native whole-file staging/unstaging, stage-all, discard, commit, HEAD amend, branch create/checkout/delete, stash save/apply/pop/drop, and background push/pull exist in `ops.rs` and `app.rs`.
- `TerminalSessionGuard` already restores terminal state through normal and panic paths. Extend its lifecycle for editor/helper suspension and cancellation instead of replacing it.
- Push/pull already use a worker thread and `mpsc`. Add progress, cancellation, resource cleanup, stale-result protection, and mutation coordination to this design.
- The CLI already implements merge, reset, rebase, cherry-pick, revert, tags, remotes, blame, bisect, clone, clean, configuration, object/index/pack plumbing, aliases, and completions. CLI stash already calls `oxidize_tui::ops`; this dependency is evidence that domain services should move below both frontends.
- `ox fmt`/`format`, staged-path formatting, and hook installation have just been added. Preserve this feature while verifying its actual semantics. Formatting a working file selected because it is staged is different from validating the staged blob.

### Packet 0: remove blockers in the current interaction path

Start here, before adding more actions. Turn each confirmed defect into a small regression, then fix the existing path.

1. **Highlighter termination.** In `syntax::tokenize_code`, the identifier condition accepts `$`, but the consuming loop accepts only alphanumeric characters and `_`. An unquoted `$` can repeatedly append empty spans without advancing. Verify with a bounded subprocess probe using shell `$HOME`, JavaScript `$value`, PHP `$variable`, and GraphQL variables. Guarantee progress in every tokenization branch. Also verify that concatenating spans reproduces input exactly, unknown file types reset language, quoted/space-containing diff paths work, and multiline strings/comments have an honest fallback.
2. **Action errors.** `lib.rs` and mouse dispatch discard many results with `let _ = ...`. Route operation outcomes to a shared visible error/result path. Retain the commit/branch/stash draft on recoverable submission failure. Distinguish operation failure from refresh failure after a successful mutation so the user does not retry a completed commit.
3. **Subtab targeting.** Keyboard branch actions are guarded by panel without consistently checking Local/Remotes/Tags. A key on Tags or Remotes must never operate on the stale local branch selection. Local rendering filters a vector that also contains remote branches; selection/navigation must use the same visible collection and stable ref identity.
4. **Confirmation and truthful labels.** Discard, untracked deletion, stash drop, destructive reset, force deletion, and history rewriting need a concrete target preview and explicit destructive choice. Keyboard, double-click, right-click, footer, and palette must share the same preconditions and confirmation. Make HEAD amend visibly distinct from editing the selected historical commit.
5. **Footer and modal geometry.** `build_footer` ignores its width and counts bytes; Unicode separators and status text make hit ranges diverge from rendered cells. Generate visible labels and hit regions from one width-aware layout, with overflow actions in a discoverable menu. Use graphemes and terminal display cells for input cursor movement, clipping, and mouse placement. Preserve long/multiline drafts across resize and focus changes.
6. **Stash conflict result.** `apply_selected_stash` ignores the clean/conflicted boolean returned by native apply and reports success. Represent clean apply, apply with conflicts, rejection before mutation, and partial failure explicitly. Preserve stash identity by OID and reflog identity, not only shifting numeric position.
7. **Concurrent mutation.** `active_job` currently guards push/pull against each other but does not stop local commit/stage/checkout/stash operations while pull is mutating. Introduce per-repository coordination shared by all mutation entry points; keep read/navigation responsive. External Git processes still require index/ref locking and expected-state checks.

Acceptance: drive actual events through the shared dispatcher, including failed commit under an existing lock, Tags/Remotes destructive keys, narrow Unicode footer clicks, resize during a draft, and staging while a fake pull is paused. Assert exact repository state and user feedback, not merely that a helper returned `Ok`.

### Packet 1: shared native services and repository truth

Extract incrementally as complete slices require it. Prefer a small shared porcelain/service crate or appropriate existing lower-level modules; do not spend the run constructing a framework.

- Move duplicated CLI/TUI checkout, staging, commit, replay, stash, and remote orchestration behind typed requests/results. Frontends supply explicit repository context and user choices; services must not read ambient cwd, print UI messages, or call `process::exit`.
- Carry `RepoContext { worktree, git_dir, common_dir, is_bare }` through the service layer. `RepoObjectStore::open` and `RefStore::new` currently use the supplied directory directly; discovering `commondir` alone does not make linked worktree operations correct. Route common objects/shared refs separately from per-worktree HEAD/index/replay state. Reject unsupported bare mutations clearly.
- Remove `App::load_repository`'s fabricated-context fallback on arbitrary discovery errors. Treat absent index on an unborn repository differently from corrupted/unreadable index. Stop converting object/index errors into empty history, empty text, or an apparently clean status.
- Reuse packed-aware ref reads, but extend the ref abstraction: generic local/remote/tag enumeration, loose-over-packed precedence, nested refs, annotated tag peeling, symbolic refs, packed deletion, rename, and expected-old/create-if-absent updates. TUI `load_branches` currently scans loose refs; `read_tags` and local transport tag enumeration are incomplete.
- Use `GitConfig` for remotes/upstream and effective identity. `compute_ahead_behind` guesses `origin/<same-name>`; replace this with `branch.<name>.remote` and `.merge` resolution. Support differing local/upstream names and multiple remotes without guesses.
- Review `LockFile::commit` on Windows: it removes the old target before rename. Lock ownership alone is not atomic replacement or a complete read-modify-write transaction. `Index::write_to` acquires its lock after callers have read and mutated an in-memory snapshot. Prevent lost updates and preserve original files when replace fails. Stash log/ref changes and checkout file/index/ref changes need coordinated failure handling.
- Preserve native packed reads throughout; several CLI commands and commit unchanged checks still use `LooseObjectStore` where existing packed objects must be readable. Do not mistake `store.loose()` writes for a problem; the distinction is reads versus writes.

Acceptance: packed-only branches/tags/HEAD, linked worktrees with independent indexes, detached and unborn HEAD, corrupt index, stale expected ref, Windows replacement failure, and two competing index edits. Assert no fabricated clean state and no changes to the wrong worktree.

### Packet 2: turn the current inspector into a real patch workspace

This is the first major feature payoff after Packet 0 prerequisites.

`model::DiffView` and `DiffLine` currently store formatted text and color categories. `diff::Hunk` already has old/new coordinates and `DiffOp`, but `create_hunks` is private and the unified formatter uses `.lines()`, losing line-ending/final-newline distinctions. Reuse the algorithm and introduce an owned, byte-preserving patch representation; never apply a patch reconstructed from lossy rendered text.

Required model: file identity, old/new paths and modes, old/new blob identity, diff source (HEAD/index/worktree/commit pair), hunk ranges, line coordinates, exact newline state, binary/type metadata, selection, and the repository/index/worktree generation on which the patch was computed. Keep display sanitization separate from mutation data.

Implement in this order: file drilldown and hunk navigation; hunk stage/unstage; line/range selection; partial discard with preview; split view and whitespace/context options. Stage selected worktree changes against the index. Unstage selected index changes against HEAD. Preserve unrelated staged and unstaged edits. A stale preimage must reject or rebase with verified semantics, never guess.

Fix directory and batch staging along the way. `status::scan_untracked` intentionally returns collapsed `dir/` rows, while TUI `stage_path` handles single files and does no useful staging for a directory. CLI `collect_files_to_add`/`cmd_add` already supply recursive collection and tracked-deletion logic. Extract reusable behavior, honor nested ignores/tracked exceptions, deduplicate, preflight, and perform one coordinated index transaction. Avoid one index reload/write per selected file.

Selection identity must include path plus staged/unstaged side, old path for renames when relevant, and repository identity. Refresh must not jump to the other side of the same partially staged file. Add tree/flat presentation, collapse state, search/filter, range selection, and multi-file operations over this model.

Acceptance: two hunks and two commits from one file; selected lines from a replacement; existing staged edits plus new unstaged edits; CRLF, lone CR, no final newline, empty file, deletion, rename+edit, executable mode, binary and invalid UTF-8; nested untracked directory; stale index/worktree after selection; exact blob OIDs and remaining bytes checked with Git as an independent oracle in disposable fixtures.

### Packet 3: history and refs using existing object data

`CommitItem` already contains parents, but UI graph rows render a literal `*`. `load_repository` eagerly traverses reachable history; `compute_commit_diff` expands trees and reads every blob on selection. Build incremental, deterministic history loading and actual DAG lanes from parent relationships. Support all/local/current ref scopes, stable OID selection, search, author/path/date filters, navigation to parents and refs, file history, commit-file drilldown, comparison endpoints, and merge-parent choice.

Diff trees by OID/mode first and load changed blobs on demand. Cache immutable objects/diffs by identity; invalidate worktree/index views by generation. Move costly scans, diffs, and highlighting out of the input/render path. Extend current list windows to inspector rendering; avoid the current `usize` to `u16` scroll truncation. Precompute language context or checkpoints so viewport rendering does not require recoloring the entire diff every frame.

Expose native branch/tag creation at selected revisions, rename/delete, remote tracking checkout, upstream configuration, reflog details, and reset modes through the shared service. Existing reset/tag/revision routines need packed-object, annotated-tag, validation, and expected-ref tests before reuse. `RefStore::resolve_rev` currently handles a limited grammar; implement only verified grammar and return explicit errors for unsupported forms.

Acceptance: merge-heavy graph with skewed timestamps, packed refs, nested annotated tags, search that preserves selection, loading cancellation, >65,535 diff lines, and a commit containing many unchanged files. Measure first useful frame and navigation latency independently from total repository load.

### Packet 4: repair replay semantics, then add interactive history editing

The CLI's `cmd_rebase`, `cmd_cherry_pick`, and `cmd_revert` are implementation seeds, not finished sequencers. They iterate file unions, use lossy `get_blob_text`, write merged text for absent paths, and represent replay conflicts using a generated stage-1 blob rather than base/ours/theirs. Rebase force-checks out upstream and has no persisted todo/continue/skip/abort API. Do not expose these unchanged behind shiny menus.

Create one typed native replay engine with correct add/delete/mode/binary handling, clean-tree and collision preflight, real index stages, expected ref updates, and durable operation state. Persist original HEAD/ref, current step, remaining todo, rewritten OID mapping, author/message choices, and recovery information before mutation. On reopening `ox ui`, detect an unfinished operation and offer valid continue/skip/abort actions. Preserve unrelated local edits or reject before mutation. Abort is not a blanket hard reset over unrelated user work.

Build conflict UI over index stages and operation state: base/ours/theirs/result, block navigation, choose side/both, manual editor, stage resolved path, binary/modify-delete/type-conflict choices, and continuation only when valid. Keep merge and replay semantics consistent; existing `cmd_merge` handles three-stage conflicts and MERGE_HEAD more fully than the replay routines but still needs mutation preflight and failure-path review.

Then add cherry-pick/revert sequences and merge-parent choice; interactive rebase todo with reorder/reword/edit/drop/squash/fixup; autosquash; amend selected old commit; split commit; move changes between commits; custom patch workflows. Use immutable OIDs for targets, show affected descendants, and avoid silently rewriting published history.

Acceptance: restart after a conflict, continue/skip/abort each operation, deletion versus empty-file distinction, add/add and modify/delete, binary bytes, root and merge commits, staged/untracked collisions, interruption between steps, and preserved author metadata with a new committer.

### Packet 5: complete stash and networking without replacing them

Stash save already writes two-parent stash commits. Extend it to selected paths/hunks, keep-index, staged-only, include-untracked, and restore-index semantics. Ordinary apply must not indiscriminately stage applied content; restore the second-parent index only when requested. Support Git-created third-parent untracked stashes. Keep pop-on-conflict retention, and perform drop using verified entry identity under coordinated ref/log updates. Replace `bytes_equal_ignoring_crlf`, which removes all CR bytes, with explicit line-ending policy that preserves binary/lone-CR differences.

Native local/HTTP/SSH transport already exists. TUI pull is fast-forward only while CLI pull fetches then merges. Unify explicit FF-only/merge/rebase strategies after their prerequisites pass. Add remote/ref selection, fetch/prune/tags/refspecs, upstream setup, push tags/deletion, and force-with-lease using the expected remote OID. Never map lease to the existing bare force boolean.

Audit capability plumbing: discovery returns capabilities, but fetch callers often call `fetch_pack` with empty capabilities; sideband/raw-pack handling must match what was negotiated. Receive-pack can negotiate sideband while `parse_push_report` expects direct report lines and currently treats no recognized report as success. Validate acknowledgements for the requested refs, model unsupported capabilities, and test raw and sideband responses with deterministic local fixtures. Extend workers with streaming progress, cancellation, timeout, child cleanup and credential redaction; inherited SSH stderr must not corrupt the terminal.

Acceptance: stash with different index/worktree versions, Git-created untracked stash, conflicted apply feedback, stale stash index, non-origin upstream with different branch name, stale lease, rejected/missing report, raw/sideband fetch, cancelled SSH process, and partial network/ref-update failure.

### Packet 6: complete the long-term catalog from proven primitives

After these slices, use the 128-item catalog below for worktrees/submodules, repository switching, bisect, branch stacks, configuration/keymaps, command palette, external editor/difftool, provider links, and custom patches. Discovery and gitlink constants are not complete worktree/submodule support. Native bisect currently chooses a middle entry from a BFS list and tracks one good/bad boundary; extend to correct DAG candidate selection, multiple good boundaries, skip, restoration of detached HEAD, and persistent UI controls.

Keep `ox fmt` useful without violating partial staging: state whether it operates on worktree or index blobs, preserve partial index contents, propagate rustfmt errors, expand directory arguments intentionally, and do not overwrite an existing pre-commit hook. Hook installation does not prove Oxidize's native commit path executes hooks; implement and document hook behavior deliberately.

Finish with platform and scale evidence, not feature-count inflation. A view-only tag list, a literal-star graph, a command enum, and a helper tested without a reachable action are partial capabilities. Count a workflow only when service, interaction, feedback, failure recovery, and verification are connected.

## Existing native engine: start here

Source inspection confirms these implementation areas already exist. This map identifies reusable code, not a certification that every edge case is complete:

| Existing area | Responsibility to inspect and reuse |
|---|---|
| `crates/core` | Git objects, OIDs, loose storage, `RepoContext`, path validation, and `LockFile` |
| `crates/index` | Binary index, status computation, entries, and tree construction |
| `crates/refs` | References, reflogs, revision resolution, and identity/signatures |
| `crates/diff` | Diff algorithms, unified output, and three-way merge |
| `crates/pack` | Pack/index formats, deltas, `RepoObjectStore`, and reachable-object collection |
| `crates/transport` | Native Git protocol logic for local, HTTP, and SSH transport |
| `crates/config` | Git configuration and ignore handling |
| `crates/cli/src/main.rs` | Existing native porcelain/plumbing command orchestration |
| `crates/tui/src/ops.rs` | Existing native staging, commit, checkout, stash, push/fetch/pull operations |
| `crates/tui/src/app.rs`, `model.rs`, `ui.rs`, `lib.rs` | Existing TUI state, views, interaction, background jobs, and terminal lifecycle |

The current TUI's remote operations already call native services such as `push_to_remote`, `fetch_from_remote`, and `pull_from_remote`; these use Oxidize object/ref/pack/transport implementations. Preserve this direction. Using a system SSH client for the SSH channel does not mean Git protocol/object logic should be delegated to system Git.

Before implementing any feature, trace its prerequisites into these areas and classify its engine support as `already-usable`, `needs-shared-extraction`, `needs-extension`, or `missing`. Prefer wiring existing services, then extracting reusable orchestration, then implementing genuinely missing primitives. Record exact symbols and current relevant tests. A feature absent from the TUI may already work in the CLI or libraries.

If native orchestration currently lives in CLI or TUI modules, extract it into a suitable shared Rust service without introducing dependency cycles. Do not make the TUI spawn `ox` or `git` to avoid sharing an existing operation. Keep domain operations reusable by both interfaces.

## Product target and reference discipline

Use Lazygit **v0.65.0** as the initial pinned behavioral reference. This reference was checked while this prompt was written. Inspect the actual reference documentation and, when useful, source or a local executable; record the tag/commit and URLs in the project feature matrix. A newer upstream release is a separate comparison update, not a reason to continuously expand the denominator or abandon the pinned work.

Start from these upstream references:

- https://github.com/jesseduffield/lazygit/tree/v0.65.0
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/README.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/keybindings/Keybindings_en.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Config.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Custom_Command_Keybindings.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Custom_DiffRenderers.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Fixup_Commits.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Range_Select.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Searching.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Stacked_Branches.md
- https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Undoing.md

Use Git documentation and Git itself as the repository semantics oracle. Lazygit is the interaction/workflow reference, not the authority for byte-level storage formats. For example, Lazygit's documentation explicitly limits reflog-based undo to relevant history/checkout operations: it does not recover discarded working-tree content, stash deletion, or a remote push. Keep those limits visible.

The catalog below mixes documented Lazygit workflows with related Oxidize quality requirements and a few extension candidates. Classify each item as `upstream-parity`, `foundation`, or `extension` after checking the reference. Never call an Oxidize extension a Lazygit feature without evidence. Upstream research is read-only; use it to understand behavior and respect any applicable license obligations if reusing code.

## Success criteria

Deliver a usable native Rust Git TUI that supports complete everyday and advanced workflows, not superficial visual imitation. Favor features with high real-world value: partial staging, commit inspection, safe branch management, search, rebase and fixup, conflict resolution, stash variants, recovery, worktrees, and robust remote operations.

Measure progress in completed user workflows and acceptance cases. Keep the original feature IDs and denominator stable. Report `verified / applicable target items`, native versus optional integration coverage, partial work, and blockers separately. Do not inflate progress by splitting a trivial keybinding into several features, excluding hard items without explanation, or counting disabled menu items as implemented.

Completion of a phase is a milestone, not permission to end the task while other unblocked target work remains. Broad parity is a long-term objective; if the active execution is interrupted or resource-limited, checkpoint honestly and make the next continuation precise.

## Architecture and backend policy

Preserve Rust, Cargo, Ratatui, Crossterm, and the established multi-crate architecture unless the actual code provides a compelling reason for a narrow change. Do not replace Oxidize with a wrapper around Lazygit, a web app, or a blanket system-Git subprocess frontend to claim quick parity.

Use native shared services for core operations. Where an optional external tool is appropriate—editor, browser, clipboard, diff/merge tool, signing program, credential helper, `gh`, or Gitflow integration—model that dependency explicitly. Optional tooling must have capability detection, precise argv construction, cancellation/error handling, and a useful unavailable state. A core operation cannot silently fall back to an unrelated executable on PATH.

If advanced Git semantics are not yet supported natively, extend Oxidize's engine and expose the operation through shared services. While incomplete, keep that particular operation safely unavailable with an actionable explanation, track it as missing/partial, and continue its prerequisites or other features. Do not enable unsafe behavior to increase feature counts. A system-Git backend is outside this task's scope; do not introduce or propose one as a shortcut.

Prefer these responsibilities; adapt names and crate boundaries to existing code rather than creating every suggested module automatically:

- Repository context and capability discovery: worktree/git/common directories, object format, index state, config, refs, operation state, permissions.
- Application services: typed requests/results for staging, commits, refs, replay, conflicts, stash, worktrees, remotes, and recovery.
- Action registry: stable action ID, contexts, label, help, shortcut, capability, enabled reason, risk/confirmation policy, and execution handler.
- Diff and patch model: original bytes and paths, old/new line coordinates, structured hunks, selection, identity/preimage validation, and application result.
- Jobs: background execution, progress, cancellation, serialization of conflicting mutations, and external process lifecycle.
- Snapshot/view models: immutable display state, selection identity, revision/generation, and invalidation.
- TUI routing: focus, panels, tabs, overlays, selection/search, action dispatch, and rendering.
- Configuration: schema, defaults, overrides, validation, migration, and atomic persistence.
- Operation journal: recoverable metadata where supported, honest limits, and error recovery.

CLI and TUI must share semantic implementations. Rendering must not mutate repository state or launch processes. Input handlers should dispatch actions rather than contain ad hoc Git algorithms. Avoid global mutable state and giant mutually dependent application structs.

## Non-negotiable correctness rules

1. Preserve unrelated user changes in the development checkout. Run destructive workflow tests only in isolated fixture repositories or verified scratch worktrees. Do not test by resetting, cleaning, rebasing, stashing, or pushing this project itself.
2. Retain hardened path validation, exclusive locks, compare-and-swap ref updates, correct index formats, packed-object access, parser limits, and terminal cleanup. Revalidate relevant gates before enabling new mutation paths; do not spend the entire project redoing the original review.
3. Preflight destructive operations across the full selection before changing anything. Preserve dirty/untracked/ignored data outside the explicit operation. Use operation-specific checks, not a blanket dirty-worktree ban that prevents safe workflows.
4. Model conflicts and resumable operations explicitly. No unresolved stages converted to an ordinary tree; no “success” after a rejected remote or partial failed mutation; no silent lossy decoding of bytes that will be written back.
5. Distinguish raw repository data from display strings. Sanitize terminal control sequences and untrusted ANSI/OSC content in filenames, messages, diffs, and remote output without altering stored bytes. Prevent malicious content from triggering clipboard writes, terminal links, or escape commands.
6. Do not treat clipboard, browser, hooks, custom shell commands, or provider actions as automatic side effects of selection, search, refresh, or application startup. Trigger them only through the corresponding intentional action and configured trust policy.
7. In the application UI, destructive actions must describe their exact target and consequences. Do not use an unprompted generic confirmation for every harmless action. Default to cancel for destructive dialogs and distinguish working-tree, index, history, stash, local ref, and remote effects.
8. Keep user secrets out of logs, config examples, screenshots, reports, and fixtures. Redact credentials in remote URLs and external process output. Use synthetic fixture identities and loopback/fake providers in tests.
9. Development authorization covers local code, tests, docs, and fixtures. It does not authorize an actual remote push, force push, PR submission, release, deployment, real provider mutation, automatic hook execution in the user's checkout, or personal/global config modification.
10. A helper/library that is not wired to a visible usable action is incomplete. A button/keybinding that prints “not implemented” is incomplete. Fake data is only allowed in explicitly labeled test/demo fixtures.

## Persistent project memory

Create and maintain `docs/parity/` without overwriting existing valuable state. Keep these files compact, cross-linked, and useful to a resumed agent:

- `REFERENCE.md`: pinned Lazygit version/commit, verified URLs, interaction notes, and deliberate differences.
- `FEATURE_MATRIX.md`: all target IDs, classification, capability, prerequisites, priority, status, implementation paths, test evidence, and limitations.
- `PLAN.md`: dependency-ordered milestones and the current small executable slice.
- `PROGRESS.md`: dated concise completed slices and relevant validation; avoid dumping raw logs.
- `DECISIONS.md`: durable architectural/product decisions and rationale; link to existing ADRs when appropriate.
- `CHECKPOINT.md`: current code state, active slice, changed paths, exact last passing/failing commands, next executable steps, unresolved assumptions, and any relevant ongoing processes.
- `BLOCKERS.md`: evidence, attempted approaches, why a blocker is external/architectural, and what unlocks it; avoid calling ordinary implementation difficulty a blocker.
- `TEST_MATRIX.md`: fixture/scenario mapping, platform/backend coverage, and exact verification commands.

Use statuses `missing`, `in-progress`, `partial`, `verified`, `blocked`, or `not-applicable`. `not-applicable` requires a specific reason and remains visible; it does not mean “hard to implement.” Reuse existing features after checking them, and mark only newly missing work for implementation. A historical `[FIXED]` note is context, not a substitute for current relevant evidence.

Every feature row must identify how a user reaches it and which service executes it. Track availability separately from backend type and parity status. Allow one active primary slice, with narrowly scoped related prerequisites. Do not create an ocean of simultaneously “in-progress” features.

For each core workflow, record the existing engine symbols reused, the native extension if any, and its CLI/TUI entry points. Keep optional editor/provider/signing integrations separate from core Git execution. The primary coverage measure is verified workflows running on Oxidize's native engine.

## The repeated implementation loop

Execute this algorithm throughout the active run. This is an agent workflow, not a shell script to spawn forever:

```text
resume current checkpoint or establish it once
inspect actual git state and relevant current code
verify a trustworthy build/test baseline
map current functionality against the pinned reference and target catalog

while there is unblocked target work and the runtime allows useful execution:
    if a new safety regression blocks mutation:
        select the smallest root-cause fix that restores the invariant
    else:
        select the highest-value incomplete workflow whose prerequisites can be completed

    define observable acceptance cases and exact affected surfaces
    inspect enough source to design a coherent narrow change
    implement the necessary service, action, state, rendering, and errors
    add meaningful regression/integration tests
    run targeted checks and fix causes of failures
    exercise the complete workflow through the actual TUI action path
    review data safety, stale state, cancellation, and recovery

    if acceptance passes:
        update feature evidence, docs, and checkpoint
        select the next unblocked slice immediately
    else:
        reduce the failing case and try a materially different approach
        if an external prerequisite truly blocks this feature:
            preserve coherent partial work, record evidence, and select another feature

    at each coherent milestone:
        run the relevant broader gates once
        reconcile actual implemented behavior against the matrix
        continue rather than writing an early final answer

on interruption or exhausted runtime:
    preserve work without destructive cleanup
    record exact resumable state and outstanding checks
    report verified progress and remaining scope honestly

on exhaustion of all actionable scope:
    run final gates
    report verified native parity, integration parity, partials, and blockers separately
```

Do not ask “shall I continue?” after each slice. Make reasonable routine choices and continue. If a choice truly requires user input, ask one focused question and keep working on independent items. Do not hold the whole roadmap hostage to an optional preference.

Avoid unproductive loops. After three materially different failed attempts at the same local issue, reduce the case and reassess the architecture; move to other actionable work if necessary. Never repeat the same failing command without a changed hypothesis. Do not fill time with sleeps, repeated unchanged test suites, broad rereads, formatting churn, fake activity, or recursive task creation. Do not remove a feature from scope solely because it needs sustained work.

Use runtime-provided continuation/task features only when actually available and authorized. The prompt does not grant unlimited tokens, bypass permissions, or guarantee execution after the runtime stops. Maintain the checkpoint so a resumed run continues without losing progress. Do not create recurring automation or a new task merely to implement this loop.

If parallel agents are explicitly enabled by the user or repository instructions, assign disjoint bounded slices with clear ownership and integrate their work against the same acceptance gates. Otherwise use one agent. Parallel tool reads are fine; competing repository mutations are not.

## Target catalog: 128 concrete feature items

Treat each ID as a stable backlog row. These are targets to audit and implement, not a claim that they are currently missing or all belong to upstream Lazygit. Check the pinned reference and classify them. Implement prerequisite-aware vertical slices; do not finish every visual item before building the operations underneath it.

### A. Navigation, help, and application shell

- **A01 — Contextual action registry.** Centralize action names, handlers, availability, shortcut resolution, help, and confirmation policy; use the same registry for menus and keybindings.
- **A02 — Panel and subview navigation.** Numbered panel focus, directional/Vim-style movement, tab cycling, main/secondary diff focus, previous-view return, and consistent Escape behavior.
- **A03 — Contextual help and action search.** Searchable available actions, descriptions, keybindings, disabled reasons, and commands relevant to the current item/operation.
- **A04 — Layout modes.** Normal, enlarged, and full-panel modes; adjustable splits where useful; retain focus/selection/scroll state across resize; minimum-terminal fallback.
- **A05 — Lists and range selection.** Page/home/end navigation, horizontal scrolling, contiguous range selection, multi-item toggles, and explicit selection count/scope.
- **A06 — Mouse and keyboard consistency.** Accurate hit targets, wheel scrolling, selection, opt-out mouse capture, and equivalent keyboard access to core workflows.
- **A07 — Recent repositories and navigation stack.** Repository picker, recent/favorite entries, nested worktree/submodule navigation, valid return behavior, and graceful unavailable-path handling.
- **A08 — Operation/status area.** Current branch/upstream, ahead/behind, active merge/rebase/bisect, selected backend/capability when relevant, running jobs, and actionable errors.

Acceptance: navigating, filtering, resizing, and refreshing never silently changes the semantic target of a pending action; help and actual shortcuts agree in every context.

### B. Files, directories, and status

- **B01 — Complete file-state presentation.** Separate staged/unstaged state on the same path; untracked, deleted, renamed, conflicted, mode/type changes, and relevant submodule states.
- **B02 — Flat and tree layouts.** Toggle layouts, expand/collapse directories and all trees, show counts, and preserve selection by path/status identity rather than row number.
- **B03 — Stage/unstage file, selection, directory, and all.** Precise scope with deletion/rename handling; keep valid unstaged edits; one shared operation contract.
- **B04 — Safe discard variants.** Selected worktree edits, selected staged state, selected untracked files, and explicit reset/clean variants with preflight and exact consequences.
- **B05 — Ignore/exclude actions.** Add appropriate escaped relative patterns to repository ignore or local exclude files; preserve existing content and show which scope will change.
- **B06 — File operations and tools.** Rename/move, editor/open action, copy relative/absolute path, and diff-tool launch with platform-safe arguments and no accidental symlink traversal.
- **B07 — Refresh and change detection.** Manual refresh, background external-change detection/debounce, conflict-aware updates, and file/status filtering without unnecessary full reloads.
- **B08 — Path/file-type correctness.** Spaces, unusual Unicode, control-character display, large/binary files, symlinks, executable bits, non-UTF-8 paths where supported, and case collisions.

Acceptance: multi-file operations preflight the entire selection, preserve unselected data, and agree with Git's index/worktree result. Tests inspect bytes and modes, not only screen text.

### C. Diff inspection and partial staging

- **C01 — Structured diff model.** Separate data from colored display; accurate old/new coordinates, hunk ranges, EOF/newline state, binary/type/mode metadata, and stable identities.
- **C02 — Hunk stage/unstage.** Apply/reverse selected hunks against the index using validated preimages; retain other staged and unstaged changes.
- **C03 — Individual-line and range stage/unstage.** Support additions, deletions, replacements, adjacent changes, multi-hunk selection, and empty-file transitions.
- **C04 — Partial discard.** Discard selected unstaged lines/hunks with an explicit preview and stale-state check; never substitute whole-file restore for a partial request.
- **C05 — Diff presentation controls.** Adjustable context, whitespace view toggle, hunk navigation, line numbers, wrapping/horizontal scroll, unified and side-by-side views where useful.
- **C06 — Search and selection in diffs.** Highlighted matches, next/previous, copy selected text, consistent keyboard range selection, and correct handling of tabs/wide characters.
- **C07 — External edit/render tools.** Optional edit-hunk, pager/diff renderer, and difftool integration with terminal restoration and validated returned patches.
- **C08 — Patch safety and scale.** Reject changed preimages; rebase selection onto refreshed content only when unambiguous; bounded rendering for large diffs; explicit unsupported binary partial staging.

Acceptance: a three-version fixture with HEAD=A, index=B, worktree=C allows selecting only intended changes into the index, while unselected bytes stay unchanged. Verify added/deleted/replaced lines, CRLF, no final newline, duplicate context, and concurrent edits. Whitespace-hidden display must not change patch semantics or stage invisible edits silently.

### D. Commit creation and metadata

- **D01 — Commit composer.** Summary/body, message preview, multiline input, cursor/selection/clipboard behavior, draft preservation after failure/cancel, and staged-file overview.
- **D02 — External Git editor workflow.** Open the configured editor with correct environment/terminal lifecycle; preserve messages across cancellation and failure.
- **D03 — Amend HEAD.** Message-only and staged-content amend, preserve author where appropriate, refresh affected refs/history, and describe published-history implications.
- **D04 — Author/trailer controls.** Set/reset author, distinct committer identity, co-author/sign-off trailers, templates/prefixes, and Git-compatible signature formatting.
- **D05 — Fixup/squash commit creation.** Select a target, create correct marker messages, show target identity, and connect to autosquash flow.
- **D06 — Hooks and signing capabilities.** Honor supported commit hooks/signing settings using deliberate external helpers; display failures; explicit user action for supported bypass. Never silently omit a required hook/signature.
- **D07 — Commit boundaries.** Empty messages, empty-tree deletion commits, explicitly requested empty commits, unmerged index rejection, and commit during merge/replay state.
- **D08 — Commit result and recovery.** Correct branch/detached-HEAD handling, reflogs, resulting parents, refresh, safe retry after failure, and no duplicate commit from double submit.

Acceptance: compare resulting trees, metadata, parents, messages, and reflogs with Git. A failed hook/signing operation leaves refs unchanged and message drafts recoverable.

### E. History, inspection, search, and comparisons

- **E01 — Real commit DAG.** Accurate branch/merge lanes and decorations, stable topological/date ordering options, incremental loading, and highlighting selected ancestry.
- **E02 — Commit details and files.** Full metadata/parents/message, changed file tree, per-file diff, and parent selection for merge-commit inspection.
- **E03 — History filters.** Search message/hash/author; filter by branch/path/range/date as supported; visible active filters; reliable clearing/back navigation.
- **E04 — Two-ref comparison.** Mark one ref/commit as base, compare another, reverse direction, drill into changed files, and leave comparison mode predictably.
- **E05 — File history and blame.** Inspect a file's historical revisions and line attribution, navigate to associated commits, and preserve path/rename context where supported.
- **E06 — Commit actions.** Checkout detached, create branch/tag/worktree, restore selected historical file, and route reset/revert/cherry-pick to their safe workflows.
- **E07 — Copy/open commit attributes.** Full/abbreviated OID, message, author, diff/path/URL, and provider-aware browser action with exact item identity.
- **E08 — History scalability.** Virtualized visible rows, cancellable loading, stable selection by OID, bounded detail caches, and complete reachability semantics on merge histories.

Acceptance: fixtures with divergent branches, octopus/ordinary merges, equal timestamps, annotated tags, and packed history render the correct graph and results; no missing parent paths or first-parent-only traversal presented as a complete DAG.

### F. Branches, tags, and refs

- **F01 — Local branches.** List/search/sort, current/checked-out-elsewhere indicators, upstream, ahead/behind, recent ordering, and branch details.
- **F02 — Branch creation and rename.** Create at HEAD/selected ref, optionally switch, rename with ref/config/reflog consistency, and validate names/collisions.
- **F03 — Branch checkout and deletion.** Preserve compatible dirty work; reject overwritten data; safe versus force delete; checked-out worktree guards and packed-ref support.
- **F04 — Upstream management.** Set/unset/change tracking, inspect divergence, establish upstream on first push, and explicit reset-to-upstream choices.
- **F05 — Merge choices.** Regular merge, fast-forward-only, no-fast-forward, and squash where supported; preview target/source and route conflicts into the resolver.
- **F06 — Remote branches.** Inspect, compare, create local tracking branch, detached checkout, choose upstream, and explicit remote delete through the transport action.
- **F07 — Tags.** Lightweight/annotated creation at a selected commit, details, checkout, local/remote delete distinction, selected tag push, and optional signing capability.
- **F08 — Branch recovery and stacks.** Move mistaken unpushed work to a new branch, select a rebase base, and describe upstream/stack relationships using verified ancestry.

Acceptance: a ref operation updates only its intended ref/config/logs, respects linked-worktree usage, and behaves correctly when refs are packed or remote-tracking names overlap.

### G. Interactive rebase and history editing

- **G01 — Rebase preparation.** Choose upstream/base/onto, compute a correct commit sequence, show affected/published commits, and preflight dirty or in-progress state.
- **G02 — Todo editor.** Pick/reword/edit/drop/squash/fixup actions, keyboard reorder, validation, visible replay order, and message combination rules.
- **G03 — Replay state machine.** Persist original branch/tip, onto, todo/done/current commit, and conflict state; continue, skip, abort, and restart after TUI/process interruption.
- **G04 — Reword/edit older commits.** Change a selected historical message or stop for content editing without collapsing unrelated commits or losing authorship.
- **G05 — Autosquash.** Resolve fixup/squash targets deterministically, preview sequence and ambiguous targets, apply selected/all eligible fixes, and preserve intended messages.
- **G06 — Amend old commit from staged changes.** Use a recoverable replay flow, retain unrelated work, and surface downstream conflicts correctly.
- **G07 — Split/reorder/drop commits.** Guided splitting via partial staging, reorder range, drop selection, clean todo semantics, and recovery refs/state where appropriate.
- **G08 — Complex ancestry handling.** Rebase onto marked base, stacked branch workflows, and merge-preserving replay only when correctly implemented; reject unsupported topology before mutation.

Acceptance: a multi-commit rebase interrupted at a conflict resumes after restart; abort restores the original branch/index/worktree according to the documented contract. No partial replay is reported as a complete rebase. Never silently flatten merge topology under a merge-preserving option.

### H. Cherry-pick, revert, reset, and recovery

- **H01 — Commit copy basket.** Select one/range/multiple commits, visible ordered basket, remove/reset selection, and preserve identities across filters/refresh.
- **H02 — Ordered cherry-pick.** Apply the selected sequence with correct ordering, empty-commit behavior, no-commit mode where supported, and conflict recovery.
- **H03 — Revert workflows.** Revert one/selection, explicit mainline parent for merges, conflict handling, and continuation/abort with correct messages.
- **H04 — Reset choices.** Soft/mixed/hard to selected ref, display exact effects on branch/index/worktree, protect untracked content, and validate stale targets.
- **H05 — Reflog inspection.** Search/details, checkout, create recovery branch, compare, and recover a previous tip without conflating reflog entry indexes with OIDs.
- **H06 — Reflog-based undo.** Preview a supported prior history/checkout operation, apply safe recovery, preserve dirty data, and clearly explain unsupported actions.
- **H07 — Redo and persistence.** Journal undo/redo where supported, invalidate unsafe redo after intervening operations, and retain state across restart.
- **H08 — Recovery boundaries.** Refuse undo during unsupported replay states; guide abort; distinguish history recovery from unrecoverable discarded worktree bytes or remote effects.

Acceptance: no action labeled undo silently runs a blanket hard reset. Tests cover external Git actions, intervening ref updates, missing/expired reflog entries, restart, and dirty worktrees.

### I. Conflict resolution

- **I01 — Conflict discovery.** Enumerate actual index stages and operation metadata; distinguish content/add-add/modify-delete/type/binary conflicts and unresolved count.
- **I02 — Conflict views.** Base/ours/theirs/result presentation, current hunk highlighting, navigation between hunks/files, and operation-aware labels during rebase.
- **I03 — Hunk choices.** Choose ours/theirs/both/manual result for a conflict region; preserve unaffected bytes and newline semantics; show pending resolution.
- **I04 — File-level choices.** Take an entire side, keep/delete path appropriately, handle binary/mode conflicts explicitly, and avoid confusing absent side with empty file.
- **I05 — External merge/editor tools.** Correct base/local/remote/result paths, temporary-file lifecycle, user cancellation, and result validation before marking resolved.
- **I06 — Resolution staging.** Stage exactly the resolved result, remove correct stages, optionally advance to next conflict, and detect unresolved markers as assistance rather than an infallible parser.
- **I07 — Local resolution undo.** Undo the last resolver edit using its own scoped buffer/journal; do not conflate this with global reflog-based undo.
- **I08 — Continue/skip/abort integration.** Route to the active operation, enable only when preconditions hold, preserve failed resolution state, and restore correctly on abort.

Acceptance: handle real Git-generated conflicts plus conflicts generated by Oxidize; verify stages and resulting parents with Git. A dirty external edit cannot be overwritten by a stale resolver action.

### J. Stash workflows

- **J01 — Stash list and inspection.** Identity/message/base/date, file list/diff, search, and selection stable across creation/drop/refresh.
- **J02 — Save ordinary tracked changes.** Preserve separate staged/unstaged states, custom messages, deletions, clean-state no-op, and correct parents/reflog.
- **J03 — Untracked/ignored variants.** Explicit include-untracked/include-ignored options with clear scope and safe restoration/collision handling.
- **J04 — Staged/unstaged/selected variants.** Keep-index, staged-only, selected paths, and partial stash where supported, using proven patch primitives.
- **J05 — Apply/pop selected stash.** Apply without dropping, pop only after successful application, optional index restoration, and conflict preservation.
- **J06 — Drop/rename selected stash.** Exact target identity, remaining stack/reflog preservation, and stale-index protection; no accidental clear-all behavior.
- **J07 — Branch/worktree from stash.** Begin from the correct base, restore changes into the intended new context, and retain recoverability on any failure.
- **J08 — Stash interoperability.** Round-trip with Git, multiple entries, binary/type changes, packed stash objects, restart, and informative partial/conflict states.

Acceptance: three or more entries survive middle drop and selected pop correctly; conflicts retain the selected stash; unselected staged/unstaged/untracked content is preserved.

### K. Custom patches and advanced change movement

- **K01 — Persistent patch basket.** Add/remove selected files/hunks/lines from inspected commits, show provenance and scope, and never confuse it with ordinary staging.
- **K02 — Patch preview/export.** View a correct applyable patch, copy/save through explicit actions, preserve paths/newlines, and warn about unsupported binary/metadata forms.
- **K03 — Apply patch to worktree/index.** Choose destination semantics, preflight matching context and locks, and preserve unrelated changes.
- **K04 — Move patch into a new commit.** Construct a new commit from selected changes with a preview of remainder and appropriate recovery.
- **K05 — Move patch between commits.** Remove changes from one historical commit and apply to another through replay, preserving intervening commits or surfacing conflicts.
- **K06 — Remove selected historical changes.** Drop a file/hunk/line contribution from an old commit without silently dropping all changes in that commit.
- **K07 — Patch extraction as inverse/revert.** Explicit direction and target; validate expected resulting content rather than trusting a rendered diff.
- **K08 — Patch conflicts and recovery.** Immutable source OIDs, stale-state checks, replay continuation/abort, basket preservation on error, and clear unsupported cases.

Acceptance: moving one hunk across a three-commit chain changes only the intended historical contribution and dependent descendants, with correct final bytes and a recoverable failed replay. This phase depends on C, G, and I; do not implement it as string manipulation on colored diffs.

### L. Remotes, networking, and provider integration

- **L01 — Remote management.** Add/edit/rename/remove URLs and remotes with config/tracking semantics, masked credentials, URL validation, and optional fork-remote convenience.
- **L02 — Fetch workflows.** One/all remotes, prune options, tags/refspecs as supported, progress/cancel, and visible distinction between refresh and fetch.
- **L03 — Pull choices.** Upstream-aware fast-forward/merge/rebase choices, preflight, progress, and explicit conflict/replay state.
- **L04 — Push workflows.** Current/selected branch, initial upstream setup, selected tags, explicit remote ref deletion, rejection handling, and no tracking-ref advancement on failure.
- **L05 — Force-with-lease.** Deliberate advanced action with expected remote OID and clear preview; reject remote races. Never treat ordinary push as force.
- **L06 — Credentials and SSH.** Respect supported helper/agent configuration, manage interactive handoff when needed, handle unavailable auth without freezing or leaking secrets.
- **L07 — Provider links and PR metadata.** Open branch/commit/compare/PR URL, optionally show PR state through a detected configured helper such as `gh`, and remain useful offline.
- **L08 — Jobs and failure semantics.** Bounded live progress, cancel/timeout, safe cleanup, rejected push/report-status errors, and controllable background fetch only after user configuration.

Acceptance: loopback/local server fixtures cover success, rejected hooks, non-fast-forward, lease race, unavailable auth, cancel, and timeout. Provider metadata uses fake responses in tests; no real PR or remote mutation is part of development validation.

### M. Worktrees, submodules, and repository context

- **M01 — Worktree list.** Main/linked locations, branch/detached status, locked/prunable/missing state, and correct common directory handling.
- **M02 — Worktree creation.** From existing/new branch or selected commit; path collision and already-checked-out-branch checks; coherent rollback on failure.
- **M03 — Worktree switching/tools.** Switch TUI context, maintain independent selection/drafts/jobs, open in editor, and return through repository navigation history.
- **M04 — Worktree removal and maintenance.** Protect dirty/locked worktrees, explicit force, lock/unlock/prune where appropriate, and contained removal of only the selected worktree.
- **M05 — Submodule presentation/navigation.** Gitlink OIDs, initialized/missing/dirty/new-commit state, enter/return, and recursive context without treating submodule contents as ordinary parent files.
- **M06 — Submodule lifecycle.** Add/init/update/sync/URL-edit/remove where supported, with proper `.gitmodules`, config, gitfile, index, and on-disk semantics.
- **M07 — Bulk/nested operations.** Explicit scope, progress, per-item failures, nested guards, and preserve user modifications across submodules/worktrees.
- **M08 — Cross-context correctness.** Cancel or isolate old jobs on context change; shared refs versus per-worktree HEAD/index/replay state; bare/unborn/unsupported layouts handled safely.

Acceptance: two linked worktrees and a dirty submodule remain independent. Removing one cannot delete another or the parent repository; branch checks account for checkout elsewhere. Use loopback/local fixture remotes for submodule tests.

### N. Bisect, branch stacks, and advanced workflows

- **N01 — Bisect setup.** Choose known good/bad revisions, validate ancestry/state, show the search interval, and retain original checkout context.
- **N02 — Bisect navigation.** Mark good/bad/skip, inspect candidate changes, report remaining/ambiguous candidates, and identify the final culprit correctly.
- **N03 — Bisect run/reset.** Optional user-configured test command, argument/exit-code semantics, progress/cancel, clean terminal handoff, and restore starting state.
- **N04 — Bisect edge cases.** Skipped ranges, non-linear history, missing objects, external Git state, and restart during an investigation.
- **N05 — Stack-aware branch view.** Verify upstream relationships, show parent/child stack, compare relevant bases, and avoid inferring stack ancestry from names alone.
- **N06 — Rebase stack choices.** Marked-base/onto flows and dependent-branch replay with explicit scope and recovery; do not silently rewrite unrelated branches.
- **N07 — Gitflow optional commands.** Feature/release/hotfix workflow integration only when a configured compatible helper is present; capability-gated, explicit external actions.
- **N08 — Repository diagnostics.** Read-only integrity/unsupported-state explanation, useful operation logs, and recovery guidance; maintenance actions remain explicit and safe.

Acceptance: bisect identifies a known introduced regression in fixture history; reset restores the original context; stack operations identify every rewritten ref in their preview.

### O. Configuration, shortcuts, and integrations

- **O01 — Validated app config.** Documented defaults/schema, global versus repository/app-session precedence, graceful malformed config handling, and safe atomic writes.
- **O02 — Custom keybindings.** Contextual remapping, conflict detection, disabling actions/keys where appropriate, generated help, and an optional Lazygit-like preset based on verified reference.
- **O03 — Appearance/accessibility.** Theme colors/attributes, readable contrast, ASCII/no-icon fallback, optional Nerd Font symbols, author/branch colors, and accessible focus indicators.
- **O04 — Editor, opener, clipboard.** Configured platform tools, safe paths/argv, explicit actions, useful errors, and graceful fallback when tools are absent.
- **O05 — Custom commands.** Named context-specific actions, stable selected-item variables, explicit argv versus shell mode, output handling, and cancellation.
- **O06 — Custom prompts and menus.** Input/select/confirm flows, safe variable substitution, current-context validation after asynchronous prompts, and no stale selection execution.
- **O07 — Diff renderers and pagers.** Optional configured renderer list/cycling, capability and terminal behavior, bounded output, and safe built-in fallback.
- **O08 — Config trust and tooling.** Validate examples/migrations, edit/reload config, and require explicit trust for repository-supplied executable commands/hooks; opening an untrusted repository must not execute its custom commands.

Acceptance: remapping changes help and actual behavior together; a config error cannot trap users without usable defaults. Filenames with quotes/metacharacters remain literal arguments. Avoid writing broad personal config as an installation side effect.

### P. Responsiveness, robustness, and release quality

- **P01 — Background job framework.** Cancellable jobs, progress/error lifecycle, per-repository mutation queue, bounded concurrency, and no long operations on render/input thread.
- **P02 — Snapshot invalidation.** Generation IDs and action preconditions, coalesced refresh, selected identity recovery, and rejection of stale asynchronous results.
- **P03 — Large-repository performance.** Virtualized panels, incremental DAG/diff loading, bounded caches, measured refresh costs, and controlled memory growth.
- **P04 — Terminal lifecycle.** Restore terminal after setup errors, panic/unwind, subprocess/editor/suspend/resume, resize, and cancellation as platform capabilities permit.
- **P05 — Unicode and output safety.** Grapheme-aware editing, display-width-correct layout, bracketed paste behavior where supported, sanitized control sequences, and no panics on narrow/empty views.
- **P06 — Testing and CI.** Meaningful unit/property/differential/action/headless/terminal tests, explicit fresh binary build, supported OS/MSRV matrix, locked dependency validation, and isolated environment.
- **P07 — Docs and discoverability.** Updated commands, keybindings, troubleshooting, supported workflow/backend matrix, and honest feature limitations with verified examples.
- **P08 — Benchmark and completion evidence.** Repeatable release benchmarks and full user journeys, documented results, no fake parity/speed guarantees, and a precise remaining-work report.

Acceptance: input stays responsive during a controlled slow job; cancel works; stale results cannot target the wrong repository; logs show actionable sanitized errors; all relevant safety regressions remain green.

## Delivery milestones and dependency order

Use these as outcome groups, not rigid reasons to postpone a prerequisite fix. Limit initial inventory work to what is needed to find the first valuable slice. Start implementation during the first work cycle.

1. **Baseline and trustworthy actions.** Inspect current hardening status; establish current tests; wire/reuse action registry, capability states, and jobs; preserve existing complete workflows. Main groups A and P plus essential core fixes.
2. **Everyday staging and commit workflow.** Files/tree/range selection, structured diff, hunk/line staging, safe partial discard, commit composer/amend, file inspection. Main groups B, C, D.
3. **History and branch control.** Real graph, search/filter, comparison, refs/tags/upstream, selection basket, reset/reflog recovery. Main groups E, F, H foundations.
4. **Replay and conflict competence.** Persistent replay state, interactive todo, cherry-pick/revert, conflict resolver, continue/abort, fixup/autosquash, old-commit amend. Main groups G, H, I.
5. **Stash and change movement.** Stash variants and selected entries, custom patch builder, commit splitting and hunk movement using the verified replay/patch engines. Main groups J, K.
6. **Repositories and remote workflows.** Fetch/pull/push/lease, worktrees, submodules, provider links, recent repository navigation. Main groups L and M plus relevant A items.
7. **Power-user completeness.** Bisect, stack awareness, custom commands/prompts, full keymap/configuration, renderer/editor integration, optional Gitflow/provider conveniences. Main groups N and O.
8. **Parity sweep and measured polish.** Recheck pinned keybindings/reference against the matrix; complete remaining actionable items; run full journeys and platform gates; optimize measured bottlenecks. Remaining P and prior groups.

Bring simple high-value features forward when their dependencies already exist. Do not delay partial staging because an optional provider integration is unresolved, or implement a complex patch-rebase trick before the conflict state machine is correct. Once a hard feature's prerequisites exist, select it; do not indefinitely circle around it with low-impact polish.

## Required end-to-end acceptance journeys

These are minimum integration journeys, not the entire test suite. Exercise through actual action dispatch and verify repository state independently. Use headless Ratatui backends for deterministic UI state and real PTY/ConPTY/manual terminal validation where available. State platform limitations honestly.

### Journey 1: split an ordinary change into two commits

Create a fixture with several files and staged/unstaged edits on one path. Use file tree, search, line/hunk selection, partial stage, message composer, and commit. Verify the first commit contains only selected changes and the second commit contains the remainder. Repeat with deletion, adjacent replacements, duplicate context, CRLF, and no final newline.

### Journey 2: stale patch protection

Open a diff, select a hunk, modify the same file externally before applying, then attempt staging/discard. The action must recompute or refuse safely with selection context preserved; it must not apply to a similar but wrong hunk. Switch repositories during a background diff load and confirm the old result cannot overwrite the new view.

### Journey 3: everyday branch and remote flow

Create/switch branch, edit, stage, commit, inspect history, configure fixture remote/upstream, push, fetch, and pull. Verify ahead/behind indicators and refs. Inject remote rejection and a concurrent remote advancement; report failure correctly and preserve tracking state. Test explicit force-with-lease against its recorded remote expectation.

### Journey 4: interactive rebase with restart

Create at least five commits. Reword one, squash/fixup another, reorder an independent commit, and edit a chosen commit. Induce a conflict, quit/restart the TUI, resolve and continue. Verify todo/done state, parents, final bytes, authors, and messages. Repeat with abort; the original branch/context must be restored according to its contract.

### Journey 5: cherry-pick range and revert

Copy several commits from another branch, switch back, paste in order, resolve a conflict, and verify result. Revert one commit and a merge with explicitly selected mainline; ensure ambiguous merge reversion cannot proceed silently. Retain basket/error state after interruption.

### Journey 6: custom patch across history

Build a patch from one file and a subset of another file in an old commit. Preview/export it, move it into another commit via replay, and verify unrelated historical changes survive. Induce a downstream conflict and abort; source history and recoverable selection remain intact.

### Journey 7: stash stack integrity

Save at least three stashes with different staged/unstaged/untracked states. Inspect and rename, drop the middle entry, apply another without dropping, and pop a selected entry with a conflict. Verify Git can read surviving entries and that the conflicted stash is retained. Exercise stash-to-branch/worktree.

### Journey 8: conflict types

Exercise content, add/add, modify/delete, binary, mode/type, and file/directory conflicts. Use hunk/file choices, manual editor, resolution undo, staging, and continue. Verify index stages, final blobs/modes, and operation parents. Do not treat finding no marker text as proof that a structural conflict is resolved.

### Journey 9: worktree/submodule isolation

Create two linked worktrees and an initialized submodule with local modifications. Switch contexts, perform isolated commits, inspect shared refs, and remove a clean selected worktree. Attempt removal of a dirty one and a checked-out branch deletion. Verify other worktrees, parent metadata, and dirty submodule files remain unchanged.

### Journey 10: recovery and bisect

Undo/redo a supported history action, reopen the TUI, inspect reflog, and create a recovery branch. Confirm unsupported worktree/stash/remote undo is clearly rejected. Start bisect on fixture history with a known regression, mark/skip/run candidates, find the culprit, and reset to original context.

### Journey 11: configuration and external tools

Remap a key, change theme, open help/action menu, invoke an explicit custom action with a selected filename containing spaces/quotes, launch a recording editor/difftool helper, cancel it, and return. Malformed config remains recoverable; untrusted repository config does not automatically execute commands; URLs/tokens are masked in logs.

### Journey 12: scale and hostile display content

Load a generated large repository and a slow loopback job. Navigate, filter, resize, scroll, and cancel while background work runs. Include long paths, wide Unicode, combining marks, huge messages, and ANSI/OSC-like bytes. Verify correct rendering bounds, sanitized display, bounded memory, no terminal side effects, and terminal restoration after injected failure.

## Engineering and verification discipline

Run the current baseline once, then targeted checks for each changed slice. Re-run wider tests at coherent milestones or after cross-cutting changes. Do not repeatedly run everything after a README-only adjustment. Do not add tests that merely restate an enum or method implementation while missing user behavior.

Standard workspace gates, adapted only for a documented toolchain/platform constraint:

```text
cargo build --workspace --locked
cargo build --bin ox --locked
cargo test --workspace --all-targets --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
```

Use Git fixture validation where applicable: `git status --porcelain=v2`, `git diff --cached`, `git ls-files --stage`, `git ls-files --unmerged`, `git cat-file`, `git log --format=raw`, `git reflog`, `git fsck --full`, and `git verify-pack`. Prefer stable machine-readable/NUL-delimited formats over localized human output for structured tests. Explicitly rebuild `ox` so tests do not accidentally use a stale binary.

Isolate test HOME/global/system Git config and relevant environment variables. Use fixed identities/time when byte comparisons require determinism. Control line-ending/filemode settings per fixture. Include Windows and Unix-specific cases with clear capability skips; a skip must not masquerade as parity verification. Make tests reproducible offline except local loopback networking.

Add a native-runtime independence check: prepare fixture repositories with the Git oracle, then run representative core `ox`/TUI workflows in a child environment where invoking system `git` or `lazygit` is unavailable or intercepted as a failure. Include stage, commit, history, checkout, stash, and local transport. Keep the development test runner's environment unchanged, and allow separately documented external tools such as SSH/editor/signing helpers only in tests explicitly exercising those integrations.

For parsers and patch engines, add bounded property/fuzz cases for malformed input and meaningful round-trip/inverse invariants. For transactions/jobs, inject write/lock/rename/process/cancel failures. For TUI actions, assert enabled/disabled conditions, model updates, active view, and resulting repository state. For terminal lifecycle, use available PTY/ConPTY facilities or document a specific manual check instead of claiming headless rendering proves raw-mode restoration.

A complete feature requires all applicable layers:

1. Verified behavioral specification and scope.
2. Shared native service or explicit optional integration.
3. Capability and precondition checks.
4. Discoverable action, help, and keybinding/menu route.
5. Correct model/rendering/selection updates.
6. Progress/cancellation/timeout for slow work.
7. Safe errors, conflict state, and recovery.
8. Relevant regression and end-to-end evidence.
9. Documentation and feature-matrix entry.

Keep the codebase coherent throughout. If a slice is interrupted, leave it documented as partial rather than pretending a half-wired feature is finished. Preserve the original source where useful, remove abandoned scaffolding only when owned and unused, and do not erase user changes to make tests pass.

## Performance goals

Establish the current baseline on the actual machine before setting claims. Optimize the interaction path first: rendering/input should not synchronously perform filesystem walks, object traversal, network calls, or full diff generation.

Use targets such as responsive navigation during a multi-second fixture job, incremental loading for long histories, and bounded memory for huge diffs. Treat any numerical latency/memory budget as a recorded benchmark target tied to fixture/hardware, not a universal promise. Collect repetitions, medians/spread, OS/toolchain, dataset size, and warm/cold cache policy.

Do not precompute the full repository on every keystroke. Reuse verified immutable object data, invalidate mutable state precisely, bound worker queues, cancel obsolete requests, and avoid quadratic full-list rescans. Performance work must not weaken stat-cache correctness, conflict handling, path safety, or output validation.

## Progress communication and stop conditions

Give concise milestone updates while working: completed workflow, relevant evidence, and next slice. Do not narrate every command or keep saying “continuing.” Keep the bulk of detail in the persistent matrix/checkpoint so resumed work is efficient.

Continue while meaningful authorized work remains and the runtime permits. Valid stopping conditions are:

- All agreed applicable feature targets and relevant gates are complete.
- The user explicitly pauses, cancels, or changes direction.
- The runtime ends or reaches a resource limit; save an honest checkpoint.
- Every remaining item depends on a genuine external prerequisite or unresolved product decision; list exact blockers and required inputs.

A difficult feature, several remaining phases, a passing old test suite, or a completed visual shell is not a stopping condition. Do not claim total Lazygit parity unless the pinned-reference matrix and workflow evidence actually establish it. If the run stops incomplete, report the exact next feature IDs and first executable steps without labeling the mission complete.

Final or checkpoint report should include verified feature IDs and user-visible outcomes, meaningful tests run, files/architectural changes, native versus optional helper coverage, remaining partial/blocker IDs, and the checkpoint path. Keep unsupported platform/auth/provider checks explicitly unverified.

## First cycle: start now

Read the current repository instructions, `docs/review/CURRENT_CODE_REVIEW.md`, and the source-grounded packets at the beginning of this document. Reconcile the live mouse/syntax/formatting changes with the recorded snapshot. Build the actual CLI binary before running integration tests: the tests package locates `target/debug/ox` and a fresh `cargo test` alone may not create it. Establish the baseline and create/update the feature matrix/checkpoint.

Begin with confirmed Packet 0 blockers, especially highlighter termination, discarded errors, wrong-subtab actions, and mutation coordination. Do not restart a broad generic audit when the current code already supplies the next executable step. Deliver a visible interaction fix, then take the narrow Packet 1 prerequisites needed for Packet 2's structured diff and partial staging. Subsequent milestone choices must reference concrete packet items and catalog IDs.

Implement it through the service, action, state, view, help, and tests. Verify the real user journey. Update evidence and immediately select the next workflow. Continue the loop through the remaining milestones while the active runtime allows progress.

---

## Short resume instruction

Use this only to restart the same implementation objective after an interruption:

```text
Resume the Oxidize Lazygit feature expansion from docs/parity/CHECKPOINT.md.
Read docs/review/LAZYGIT_PARITY_PROMPT.md and the current feature matrix.
Inspect the actual checkout and validate the last incomplete slice; do not
restart the inventory or assume prior status claims are still accurate.
Continue implementing and testing the next unblocked high-value feature IDs
through the persistent work loop. Preserve user changes and safety gates.
Do not stop after one slice while other authorized work remains actionable.
Update the checkpoint and evidence at each completed slice. Do not create
automation or perform real remote mutations as a workaround for runtime limits.
```

## Prompt construction reference

The persistence/checkpoint/verification workflow follows the general autonomy, compaction, and anti-repetition guidance in the official Codex prompting guide: https://developers.openai.com/cookbook/examples/gpt-5/codex_prompting_guide . This document does not require a specific model or depend on API harness implementation details.
