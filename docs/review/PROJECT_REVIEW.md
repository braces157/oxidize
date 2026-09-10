# Oxidize project review

Reviewed 2026-09-11 at commit `3f84a72365fc332f98a1ec470de118f13a18318f`.

## Assessment

Oxidize has a useful foundation: nine focused implementation crates, a combined loose/packed object reader, differential tests against Git, and a substantial Ratatui interface. However, it is not ready for the README's production-grade, daily-driver, or 100% compatibility claims. Ordinary commands can destroy uncommitted work, corrupt an index, bypass another process's locks, or mishandle history. Repository-controlled paths and SSH arguments also need security hardening before processing untrusted repositories.

Prioritize repository integrity over new commands, visual redesign, SHA-256, or additional parallelism. The most valuable architecture improvement is a shared repository operations layer used by both CLI and TUI; duplicated mutation logic already produces incompatible behavior.

This review changed no production source. Its deliverables are this report, an implementation prompt, and reproducible evidence under `docs/review/`.

## Scope and validation

Inspected the workspace manifests, CI, documented guarantees, CLI command implementations, object storage, index/status/tree handling, refs/reflogs, pack/delta parsing, transport framing/HTTP/SSH/local paths, config/ignore handling, merge/diff code, and TUI operations/event lifecycle. This is a source and behavior review, not a formal verification or exhaustive protocol certification.

Local environment: Windows, Rust 1.95.0, Git 2.52.0.windows.1.

| Check | Result |
|---|---|
| `cargo build --bin ox` | Passed; probes used the freshly built debug binary |
| `cargo test --workspace --all-targets` | 86 passed, 0 failed, 0 ignored, after explicitly rebuilding `ox` |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo doc --workspace --no-deps` | Passed |
| Additional isolated behavior probes | 14 scenarios expose failures; see JSON evidence |
| Additional parser probes | 4 problematic behaviors reproduced; see text evidence |
| Actual Rust 1.80 build | Not run; only stable is installed. Resolved package metadata already contradicts 1.80 support |
| Real remote HTTP/SSH, Linux/macOS, interactive terminal | Not exercised; related findings below are explicitly marked source analysis or parser probes |

The normal suite passing is valuable but does not cover these failure modes. Several integration helpers find `target/debug/ox` themselves, so an explicit binary build is necessary to avoid validating a stale executable. Existing transport integration scenarios use filesystem remotes; they do not establish HTTP or SSH interoperability.

Evidence files: `reproduction-results.json`, `parser-results.txt`, `test.log`, `clippy.log`, `doc.log`, and `dependency-msrv.txt`. Reproduction sources: `reproduce.py` and `parser_probe.rs`. The Python probes only mutate newly created temporary repositories and synthetic data; the local push case uses a second temporary repository, with no network access. Temporary repositories are retained for inspection. The parser panic is caught intentionally and is evidence, not an unexpected failure of the review process.

To rerun the review probes from the project root in PowerShell:

```powershell
cargo build --bin ox --locked
python docs/review/reproduce.py
powershell -NoProfile -File docs/review/run-parser.ps1
```

The probes report observed behavior rather than enforcing the future corrected contract. Promote each relevant scenario into an asserting regression test during implementation.

## Findings and fix requirements

P0 means a release-blocking boundary violation. P1 means high-priority data integrity, security, or essential interoperability failure. P2 means correctness, maintainability, or validation work that follows the safety foundation. “Reproduced” means observed locally; “source” identifies a finding established by inspection but not exercised end-to-end.

### F01 — P0: Checkout can write outside the repository

**Evidence: reproduced.** `crates/core/src/store.rs:258` accepts unchecked tree entry names; `crates/index/src/status.rs:56` concatenates them; `crates/cli/src/main.rs:2027` joins them directly to the worktree and writes. A synthetic tree entry named `../escaped-tree-sentinel` caused successful checkout to create a file outside the fixture repository, still inside the review's temporary directory. The TUI duplicates the unsafe checkout at `crates/tui/src/ops.rs:213`.

**Fix:** Validate tree components and index paths before materialization, reject absolute paths and traversal, protect `.git` and platform aliases, and prevent writes through existing symlink/reparse-point parents. Use platform-aware rooted filesystem operations; string-prefix checks alone are insufficient. Validate the entire operation before the first mutation.

**Acceptance:** Malicious trees/indexes and symlink-parent fixtures cannot write or remove any outside sentinel or repository metadata; failures preserve index, HEAD, and unrelated worktree bytes. Test Windows path forms and Unix symlinks separately.

### F02 — P1: Checkout overwrites uncommitted work without checking

**Evidence: reproduced.** `crates/cli/src/main.rs:2027` / `:2104` and `crates/tui/src/ops.rs:213` write target blobs and remove old tracked paths without a dirty-worktree or untracked-collision preflight. Git rejected the fixture checkout; `ox checkout other` returned success and replaced the user's edit. Several merge/reset/stash/replay commands reuse this helper.

**Fix:** Separate safe branch switching from explicitly destructive restore/reset operations. Calculate an operation plan against HEAD, index, worktree, and target; reject overwritten staged/unstaged/untracked data while preserving compatible local edits. Surface deletion/read/write failures instead of ignoring them. Define recovery for partial filesystem failures.

**Acceptance:** Differential cases for staged and unstaged edits, untracked collisions, file/directory transitions, missing objects, unreadable files, and failure injection. Assert both command result and unchanged repository state on preflight rejection.

### F03 — P1: `rm` ignores force protection and recursively deletes untracked files

**Evidence: reproduced.** `crates/cli/src/main.rs:3118` names the force argument `_force`; it deletes modified tracked files without checking HEAD/index/worktree. Its recursive branch calls `remove_dir_all` after tracked removals, deleting unrelated untracked contents. Both behaviors were reproduced.

**Fix:** Implement Git's `rm`, `--cached`, and `--force` checks. Preflight all pathspecs before mutation. Remove only selected tracked paths; remove directories only when empty. Return failures rather than pretending a deletion succeeded.

**Acceptance:** Dirty tracked files survive unforced removal; recursive removal retains untracked and ignored neighbors; staged-only differences and `--cached` match Git; multi-path errors do not partially remove earlier paths.

### F04 — P1: Index and ref lockfiles are not exclusive locks

**Evidence: reproduced existing-lock overwrite; source concurrency/durability analysis.** `crates/index/src/index.rs:91` and `crates/refs/src/ref_store.rs:149` use `File::create` on lock paths. Both operations succeeded with a pre-existing sentinel lock and consumed that lock. Ref compare-and-swap reads/checks the old OID before acquiring any exclusive lock. Branch creation and HEAD updates also write directly (`ref_store.rs:397`, `:426`). `flush()` is not a durability guarantee.

**Fix:** Introduce an exclusive lock guard using create-new semantics, ownership-aware cleanup, and durability policy. Hold the index lock across read/modify/write, and acquire the ref lock before reading/checking the old value. Handle create-if-absent semantics, packed refs, reflogs, Windows replacement, and failure recovery consistently. Do not claim a multi-file transaction is automatically atomic because each file uses rename.

**Acceptance:** Existing lock bytes survive rejection; concurrent writers cannot lose updates; CAS races reject stale writers; interrupted writes leave valid old or new state with recoverable logs. Fault-inject write, sync, rename, and reflog failures.

### F05 — P1: Mutating a v4 index corrupts its encoding

**Evidence: reproduced.** `crates/index/src/index.rs:91` writes `self.version` but serializes every entry with the v2 writer. A Git-created v4 index followed by `ox add b.txt` returned success, then `git ls-files --stage` failed with “malformed name field in the index.” The loader also accepts v3 and ignores trailing extensions without distinguishing required extensions; entry representation lacks the full extended-flag state.

**Fix:** Implement version-correct serialization, or deliberately convert supported inputs to v2 with a matching header and proven semantic preservation. Parse and preserve relevant flags/extensions; reject unsupported required extensions before mutation. Bound counts and path lengths against available bytes.

**Acceptance:** Git-created v2/v3/v4 indexes survive every supported mutator and remain readable by Git. Include shared-prefix/long paths, conflicts, skip-worktree/intent-to-add, checksum/truncation, and unsupported-extension fixtures. Unsupported combinations must fail without rewriting the original index.

### F06 — P1: Unresolved conflicts can be turned into a tree or commit

**Evidence: reproduced tree acceptance; source merge-state analysis.** `crates/index/src/tree.rs:79` inserts all index stages into one path map; it never rejects stages 1–3. With a Git-generated unresolved merge, Git refused `write-tree`, while `ox write-tree` returned a tree OID. `cmd_commit` (`main.rs:1305`) uses this writer. `cmd_merge` (`:2171`) stages conflict-marker content at stage 1 instead of recording base/ours/theirs and leaves no `MERGE_HEAD` state. A subsequent ordinary commit has only HEAD as parent. The conflict branch prints failure but falls through to `Ok(())`.

**Fix:** Reject unmerged indexes in write-tree/commit/amend; implement proper conflict entries and operation state. Preserve both parents when committing a resolved merge. Return nonzero on conflict. Integrate continue/abort semantics for merge, cherry-pick, revert, and rebase; persist replay state instead of leaving users unable to resume correctly.

**Acceptance:** Validate `git ls-files --unmerged`, merge parent OIDs, process exit codes, restart/continue/abort, and commits after resolving conflicts. Never silently select one index stage.

### F07 — P1: Stash operations can lose access to the remaining stack

**Evidence: reproduced CLI stack loss; source TUI divergence.** `crates/cli/src/main.rs:3412` pops by replacing the worktree with a stash tree, then removes `refs/stash`. With two Git stashes, popping the first left `git stash list` empty, although the reflog file remained. Drop ignores the selected index. TUI pop/drop at `crates/tui/src/ops.rs:334` / `:356` remove the entire stash reflog; TUI save at `:573` creates a one-parent commit, does not preserve a separate index parent, does not restore the worktree, and misses tracked deletions.

**Fix:** Share a Git-compatible stash implementation: base/index/worktree parents, stack maintenance, selected entry handling, conflict-aware apply, optional index restoration, and pop-only-after-success. Preserve the stash and remaining stack on failures.

**Acceptance:** Three-entry stack; middle drop; pop after unrelated edits; conflicts; staged versus unstaged changes; deletions; CLI/TUI interoperability with `git stash`. Surviving entries and reflogs must remain discoverable.

### F08 — P1: Essential commands fail once objects are packed

**Evidence: reproduced checkout; source other command paths.** `cmd_checkout` (`main.rs:2104`) constructs `LooseObjectStore`, as do reset, stash, and several object-reading paths. After `git gc --prune=now`, `ox checkout main` failed with object-not-found while Git succeeded. Separately, `crates/pack/src/store.rs:75` calls `read_pack_object_at` with no base resolver, so indexed REF_DELTA objects cannot be resolved through this lookup.

**Fix:** Use `RepoObjectStore` consistently for repository reads; keep a separate loose-object writer capability. Add REF_DELTA resolution with cycle/depth limits and verified base lookup. Distinguish missing, corrupt, and unsupported objects; do not convert read failures into empty content or unborn history.

**Acceptance:** Run the command compatibility matrix before and after Git/ox packing, with OFS_DELTA and REF_DELTA packs, annotated tags, mixed loose/packed objects, and missing bases.

### F09 — P1: Ref names can escape their namespace; packed branch updates are incomplete

**Evidence: reproduced namespace traversal; source packed-ref behavior.** `crates/refs/src/ref_store.rs:397` joins branch names directly; `ox branch ../../review-sentinel` wrote `.git/review-sentinel`. Normalize/read/update/delete do not centrally validate names. Creation only checks a loose file, allowing a packed branch to be shadowed; deletion only handles loose refs and can reveal a packed value again. `cmd_branch` treats `-d` and `-D` the same and omits the mergedness check.

**Fix:** Central validated ref types and Git-compatible ref-name rules, platform path containment, packed/loose transactional updates, correct symbolic ref behavior, and protected deletion semantics. Apply validation to remote-advertised names too.

**Acceptance:** Differential `check-ref-format` fixtures; traversal/absolute/newline/backslash/lock-suffix rejection; packed-only and shadowed refs; `-d` refuses unmerged history while an explicitly requested force operation follows its defined contract.

### F10 — P1: Push bypasses safety checks and transfers unrelated objects

**Evidence: reproduced filesystem push; source HTTP/SSH response handling.** `crates/cli/src/main.rs:2975` packages `collect_all_objects()` rather than the requested reachable closure. A local fixture push copied an unreachable synthetic blob. The local path updates the destination ref with no expected old OID, ancestry check, or checked-out-branch protection. The same probe overwrote divergent destination history while its worktree retained the old files. HTTP `push_pack` (`crates/transport/src/client.rs:117`) returns report text without interpreting `ng` or unpack failure, and the CLI then advances the tracking ref and prints success. `fetch_local_pack` also ignores `_wants` (`local.rs:100`).

**Fix:** Transfer only objects required by the requested refs, honoring negotiated haves. Enforce non-fast-forward/CAS and checked-out destination rules. Parse all unpack/ref statuses and update tracking refs only for confirmed successes. Quarantine/validate incoming objects before publishing refs. Avoid direct local transport mutations that bypass the shared receive policy.

**Acceptance:** Unreachable synthetic secrets are absent at destination; divergent/checked-out destinations reject safely; hook/server rejection, races, partial failures, and empty/up-to-date pushes return correct statuses and preserve tracking refs.

### F11 — P1: SSH command construction permits unsafe interpretation

**Evidence: source argument-construction analysis; absolute-path parser reproduced.** `crates/transport/src/ssh.rs:191` interpolates the endpoint path into a single-quoted remote shell command without escaping embedded quotes, and passes an unvalidated destination that can begin with `-` to SSH. Parsing strips leading `/` from `ssh://` paths. `find_ssh_binary` extracts only the first whitespace token from `GIT_SSH_COMMAND`, losing options and breaking executable paths containing spaces.

**Fix:** Validate endpoint authority and disallow option-shaped destinations; quote remote arguments correctly for the supported SSH remote-shell contract. Preserve absolute versus SCP-relative paths. Implement documented `GIT_SSH`/`GIT_SSH_COMMAND` behavior with correct platform parsing, without introducing another shell-injection path.

**Acceptance:** A recording fake SSH helper verifies exact argv and remote command for spaces, quotes, metacharacters, IPv6, absolute/relative paths, options, and Windows executable paths. Rejected endpoints never spawn a process. No real remote exploitation was attempted in this review.

### F12 — P1: Transport framing is not a protocol state machine

**Evidence: parser reproduction and source.** `crates/transport/src/pkt_line.rs:146` reads to EOF; SSH discovery calls it before sending a request or terminating the child (`ssh.rs:216`). A normal upload-pack server sends a flush and then waits for input, creating a read-to-EOF deadlock. `SidebandDemuxer` treats any non-band packet as pack content: `NAK\n` followed by band-1 `PACK` produced bytes for `NAK\nPACK`. Incomplete 1–3-byte pkt-line prefixes are silently accepted. `protocol.rs:83` requests a fixed capability set without intersecting the advertisement.

**Fix:** Separate advertisement, negotiation, ACK/NAK, pack, sideband, and report-status phases. Use incremental bounded reads and protocol-specific flush boundaries. Add timeouts, cancellation, child wait/cleanup, strict truncation errors, and advertised capability negotiation. Handle supported HTTP versions and content types explicitly; reject unsupported protocol versions.

**Acceptance:** Local upload-pack/receive-pack process fixtures and a loopback HTTP server exercise valid advertisements, ACK/NAK, streamed pack chunks, stderr/error bands, truncation, timeouts, rejection, and capability differences. No hang, negotiation-byte contamination, or success on rejected push.

### F13 — P1: Untrusted pack/delta inputs can panic or exhaust resources

**Evidence: truncated delta panic reproduced; source resource-limit analysis.** `crates/pack/src/delta.rs:6` directly indexes optional copy operands. `apply_delta(b"a", &[1, 1, 0x91])` panicked at line 37 instead of returning `PackError`. Target sizes feed allocation directly. `packfile.rs:306` allocates declared decompressed size, and `:344` recursively resolves offset deltas without a depth/visited guard; zero backward offsets can recurse into the same entry. Loose-object and packet readers accumulate unbounded input. Checked shifts alone do not prove that shifted values retain all significant bits.

**Fix:** Checked slice readers, checked arithmetic, validated lengths, configurable resource budgets, bounded inflation, and delta depth/cycle checks. Validate object hashes, declared sizes, pack/index correspondence, and trailer checksums at appropriate trust boundaries. Treat the mmap immutability requirement explicitly: read-only opening does not itself prevent another process from modifying/truncating the underlying file.

**Acceptance:** Regression tests plus fuzz/property tests for delta, pack headers, offsets, index, object, and pkt-line parsers. Malformed input returns typed errors without panic, stack overflow, uncontrolled allocation, or persistent repository mutation. Crash-prone fuzz cases run in isolated processes.

### F14 — P1: Merge/replay treats binary data and deletions as text

**Evidence: source.** `get_blob_text` (`main.rs:2305`) maps absent objects and read failures to an empty string and decodes bytes with `from_utf8_lossy`. Merge/rebase/cherry-pick/revert write merged strings for paths regardless of file existence or mode. Missing and empty are different states; invalid UTF-8 must not be replaced during a repository mutation. `crates/diff/src/merge.rs:15` uses `.lines()` and recreates newlines on its nontrivial path, changing CRLF and final-newline semantics.

**Fix:** A tree merge model must retain existence, object ID, mode/type, and bytes. Implement structural add/delete/type conflicts separately from text merging. Use a binary policy that preserves original blobs and reports conflicts when necessary. Propagate object errors. Preserve newline state or document and gate unsupported text transformations.

**Acceptance:** Binary/NUL/invalid-UTF-8 blobs, CRLF/no-final-newline, add/add, modify/delete, delete/delete, executable modes, symlinks, and file/directory conflicts produce correct trees and conflict state without lossy byte conversion.

### F15 — P2: Staging/status/file-type semantics have important gaps

**Evidence: mode-only false-clean result reproduced; source remainder.** `crates/index/src/status.rs:97` compares staged OIDs but ignores modes, collapses conflict stages by path, and treats matching size/mtime as clean without racy-index handling. Read/stat failures can also become clean. `cmd_add` (`main.rs:1072`) scans existing files only, so tracked deletions are not staged; `add .` uses repository root even from a subdirectory. Symlink-aware I/O and file-mode restoration are absent in relevant paths. `GitIgnore::load_from_dir` loads only the root ignore file (`config/src/ignore.rs:96`).

**Fix:** Model conflict and type/mode changes explicitly; validate the stat cache against index timestamp and relevant metadata; surface read errors. Implement correct pathspec scope, tracked deletion staging, symlink/gitlink behavior or safe rejection, nested ignore scope, and tracked-file exceptions to ignore filtering. Preserve byte paths where the OS/Git support them.

**Acceptance:** Compare Git status/index for same-size rewrites, coarse timestamps, chmod-only changes, deletions, subdirectory commands, ignored tracked files, nested ignores, symlinks, non-UTF-8 Unix names, and Windows case collisions.

### F16 — P2: Empty-index commits and repository discovery are incorrect/incomplete

**Evidence: deletion commit reproduced; discovery source.** `cmd_commit` (`main.rs:1305`) and TUI `create_commit` (`ops.rs:157`) reject an empty index unconditionally, blocking a legitimate commit deleting the last tracked file. `find_git_dir` (`core/src/store.rs:204`) only finds a `.git` directory; it does not resolve gitfiles, linked-worktree common directories, or bare repository layout. Callers derive worktree root from `git_dir.parent()`, which does not hold for those layouts.

**Fix:** Determine “nothing to commit” by comparing the staged tree and operation state to HEAD. Introduce a repository context that distinguishes worktree, git dir, common dir, bare mode, and supported repository format/extensions. Support layouts deliberately or reject them safely; never discover a parent repository through an unsupported nested gitfile and mutate the wrong project.

**Acceptance:** Commit the final deletion through CLI/TUI; linked-worktree/submodule gitfile/bare fixtures resolve correctly or report explicit unsupported-layout errors before any write. Unsupported hash formats and index modes fail closed.

### F17 — P2: TUI duplicates business logic and can block or leave terminal state dirty

**Evidence: source.** `crates/tui/src/ops.rs` duplicates CLI checkout, commit, stash, and ref operations with observable semantic drift (F02/F07/F16). `app.rs:1689` / `:1726` synchronously execute `git push/pull` from the event flow, contradicting the zero-runtime-Git claim and potentially freezing input during network/auth work. `lib.rs:47` restores terminal state only after successful setup reaches `run_loop`; setup failures and panics bypass cleanup, and one cleanup error can prevent remaining restoration.

**Fix:** Thin CLI/TUI adapters over shared typed operations; explicit backend policy; background jobs with progress/cancellation and no overlapping repository mutation; selection-stable refresh. Use a terminal session guard that attempts all cleanup on partial setup, errors, and unwind. Keep destructive action descriptions precise and consistent with actual scope.

**Acceptance:** Headless tests invoke the same operation contract as CLI; slow/failing jobs keep navigation responsive; error/panic/setup-failure terminal tests restore raw mode, screen, mouse capture, and cursor. Exercise small terminals and Unicode modal editing without requiring a visual redesign.

### F18 — P2: Release claims and toolchain contract are not backed by the build

**Evidence: metadata/source.** The workspace advertises Rust 1.80, but packages do not inherit `rust-version`, and locked dependencies include Clap 4.6.6 requiring 1.85 and Instability 0.3.13/Darling 0.24.1 requiring 1.88. CI tests only latest stable. The README claims 100% parity, zero panics, crash-proof writes, fully supported advanced commands, and a standalone runtime; the findings above contradict those guarantees. Performance tests are small process-timing smoke tests, insufficient to establish universal superiority. Config is stored as one string per key (`config/src/ini.rs:19`), dropping multivalued settings and unsupported syntax on serialization.

**Fix:** Choose and enforce a tested MSRV, either pin compatible dependencies or raise the promise; inherit the field in every package and test `--locked`. Publish a tested command/format/transport support matrix and known limitations. Preserve or safely reject config syntax instead of silently deleting semantics. Replace hard-coded parity/test badges and performance conclusions with reproducible evidence.

**Acceptance:** Clean locked build/test at declared MSRV and current stable on supported platforms; docs match tested behavior; benchmark metadata includes hardware, OS, toolchain, datasets, repetitions, cache policy, medians/variation, and correctness verification.

## Improvement roadmap

| Phase | Scope | Dependencies / exit gate | Relative size |
|---|---|---|---|
| A: Safety baseline | F01–F05, F09, F11, parser bounds from F13; honest support limits | Probes become regressions; no path escape, dirty-data loss, lock theft, or v4 corruption | Large |
| B: Storage and operation correctness | F06–F08, F14–F16; ref/pack publication recovery | Shared repository context and transaction primitives from A; Git validates resulting refs/index/trees/conflicts | Very large |
| C: Transport correctness | F10–F12 and remaining F13 integrity checks | Reachability and CAS from A/B; loopback servers prove rejection handling and bounded streaming | Large |
| D: Shared operations and TUI | Complete F17; modularize CLI command families and TUI state/jobs | Extract shared operations incrementally throughout A–C; CLI/TUI use identical mutation contracts | Large |
| E: Compatibility and quality | Remaining config/ignore/path semantics, MSRV/CI/docs, structured errors | F18 MSRV/docs corrections start immediately; expand advertised support only when gated by tests | Medium to large |
| F: Measured optimization | Batch index updates, OID-map rename pairing, object cache, bounded pack streaming, incremental TUI refresh | Correctness corpus stays green; representative release benchmarks establish bottlenecks first | Medium |

Sizes express relative complexity, not calendar promises. Break phases into small reviewable changes, each with an invariant and a Git-backed regression. Parallelism in `add` does not compensate for `Index::add_entry` repeatedly retaining and sorting the whole vector; bulk sort/merge is a concrete later optimization. Exact rename pairing currently scans deletion candidates for each new file; an OID-indexed structure avoids quadratic behavior. Do not optimize either until safety and path semantics are stable.

## Remaining questions to resolve during implementation

- Select the initial supported repository contract: SHA-1, normal indexes, supported file types, and explicit behavior for linked worktrees, gitlinks, split/sparse indexes, attributes, and filters.
- Define durability/recovery guarantees for object/index/ref/reflog publication and garbage collection; the current `gc` writes final pack/index names then prunes loose objects without explicit synchronization or recovery protocol (`main.rs:2557`).
- Test identity/config precedence and round-tripping, annotated tags/extra commit headers, complete DAG traversal and merge-base selection, binary diff presentation, and pack checksum/REF_DELTA coverage beyond the probes here.
- Determine whether system Git is a documented optional TUI backend or whether every TUI operation must use the native implementation. Do not silently change architecture or claim no dependency while spawning Git.
- Dependency vulnerability/advisory scanning, sustained fuzzing, crash-injection campaigns, and real authenticated remote interoperability remain future verification work; this review does not claim they passed.
