# Oxidize Lazygit Parity Execution Plan

## Milestone Structure and Dependency Order

1. **Milestone 1: Baseline, Safety, and Interaction Path (Packet 0 & Core Fixes)**
   - 0.1 Highlighter Termination & Robustness (Progress guarantee in `syntax.rs`, `$HOME`/`$val`/etc.)
   - 0.2 Action Error Routing (Replace discarded `let _ = ...` with visible feedback in `status_message`)
   - 0.3 Subtab Targeting (Guard branch deletion and checkout by active subtab; ensure Local vs Remotes vs Tags isolation)
   - 0.4 Truthful Labels & Destructive Confirmation (Preview and explicit choices for discard, branch delete, stash drop)
   - 0.5 Width-Aware Geometry (Fix `build_footer` byte length vs display cells, click bounds, clipping)
   - 0.6 Stash Conflict Outcome (Surface conflict boolean from `ops::apply_stash` in status message)
   - 0.7 Concurrent Mutation Coordination (Prevent stage, commit, branch, stash mutations during background jobs)

2. **Milestone 2: Structured Patch Workspace and Everyday Staging (Packet 1 & Packet 2)**
   - Structured `Patch` / `Hunk` model preserving byte and line-ending truth
   - File drilldown and hunk navigation in inspector
   - Hunk stage / unstage
   - Line and range selection staging / unstaging
   - Safe partial discard with preview
   - Multi-file and directory recursive staging

3. **Milestone 3: History DAG and Enhanced Ref Control (Packet 3)**
   - Real commit DAG lanes and topology rendering
   - Filtering and searching history (author, message, path)
   - Two-ref comparison mode
   - Reflog details and navigation
   - Configured upstream resolution via `GitConfig`

4. **Milestone 4: Native Replay Sequencer and Conflict Resolution (Packet 4)**
   - Durable sequencer state machine (continue / skip / abort)
   - Three-way conflict views and interactive resolver
   - Interactive rebase todo editor (pick / squash / fixup / drop / reword)
   - Autosquash and old commit amend
   - Ordered cherry-pick and revert

5. **Milestone 5: Advanced Stash and Patch Movement (Packet 5)**
   - Stash variants (keep-index, staged-only, untracked)
   - Third-parent untracked stash support
   - Persistent patch basket and cross-commit change movement

6. **Milestone 6: Remotes, Networking, and Worktrees (Packet 5 & 6)**
   - Fetch / pull / push with remote and refspec selection
   - Force-with-lease with expected remote OID
   - Streaming progress and cancellation
   - Linked worktrees management and submodule presentation

7. **Milestone 7: Power-User Tools and Configuration (Packet 6)**
   - Bisect setup, navigation, and automated run
   - Stack-aware branch views
   - Keybindings configuration and custom commands
   - External editor / difftool lifecycle

8. **Milestone 8: Parity Sweep, Scale Benchmarks, and Final Evidence**
   - Full acceptance journeys 1–12 verification
   - Large repository scalability
   - Final audit against Lazygit v0.65.0
