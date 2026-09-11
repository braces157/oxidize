# Oxidize Lazygit Feature Matrix (128 Items)

Reference: **Lazygit v0.65.0**  
Engine: **Native Oxidize Engine (Pure Rust)**  
Status classifications: `verified`, `partial`, `in-progress`, `missing`, `blocked`, `not-applicable`.

---

## A. Navigation, Help, and Application Shell

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **A01** | Contextual action registry | foundation | core | verified | `tui::ops`, `tui::mouse`, `tui::ui` | `tui_packet0_test.rs::test_packet0_confirmation_modal_workflow` |
| **A02** | Panel and subview navigation | upstream-parity | core | verified | `tui::lib`, `tui::app` | `tui_lazygit_test.rs::test_vim_window_focus_navigation` |
| **A03** | Contextual help and action search | upstream-parity | core | verified | `tui::app::ActiveModal::Help`, `tui::ui` | `tui_lazygit_test.rs::test_contextual_help_modal_for_all_panels` |
| **A04** | Layout modes & responsive split | upstream-parity | core | verified | `tui::ui::compute_layout`, `AppLayout` | `tui_stage1_test.rs::test_ui_rendering_headless_backend` |
| **A05** | Lists and range selection | upstream-parity | core | partial | `tui::app`, `tui::ui::window_range` | `tui_robustness_test.rs::test_selection_stability_across_refresh` |
| **A06** | Mouse and keyboard consistency | upstream-parity | core | verified | `tui::mouse`, `tui::lib`, `tui::ui` | `tui_mouse_test.rs`, `tui_packet0_test.rs` |
| **A07** | Recent repositories & nav stack | upstream-parity | optional | missing | `tui::app`, `oxidize_core::RepoContext` | To be scheduled in Milestone 6 |
| **A08** | Operation/status area | upstream-parity | core | verified | `tui::app::status_message`, `tui::ui` | `tui_packet0_test.rs::test_packet0_footer_unicode_width_alignment` |

---

## B. Files, Directories, and Status

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **B01** | Complete file-state presentation | upstream-parity | core | verified | `oxidize_index::compute_status`, `tui::model` | `tui_stage2_test.rs` |
| **B02** | Flat and tree layouts | upstream-parity | core | partial | `tui::ui::render_files_panel`, `tui::app` | Scheduled for Packet 2 |
| **B03** | Stage/unstage file, selection, directory, all | upstream-parity | core | verified | `tui::ops::stage_paths`, `stage_path`, `stage_all` | `tui_packet1_packet2_test.rs::test_packet2_untracked_directory_batch_staging` |
| **B04** | Safe discard variants | upstream-parity | core | verified | `tui::ops::discard_file`, `tui::app` | `tui_packet0_test.rs::test_packet0_confirmation_modal_workflow` |
| **B05** | Ignore/exclude actions | upstream-parity | core | missing | `oxidize_config::GitIgnore`, `tui::ops` | Scheduled for Milestone 2 |
| **B06** | File operations and tools | upstream-parity | optional | missing | `tui::app`, `std::process::Command` | Scheduled for Milestone 2 |
| **B07** | Refresh and change detection | upstream-parity | core | verified | `tui::app::refresh`, `oxidize_index` | `tui_robustness_test.rs::test_selection_stability_across_refresh` |
| **B08** | Path/file-type correctness | foundation | core | partial | `oxidize_core::path`, `tui::ops` | `version_test.rs`, path normalization tests |

---

## C. Diff Inspection and Partial Staging

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **C01** | Structured diff model | foundation | core | verified | `oxidize_diff::patch`, `tui::model::DiffView` | `tui_packet1_packet2_test.rs::test_packet2_partial_hunk_staging` |
| **C02** | Hunk stage/unstage | upstream-parity | core | verified | `oxidize_diff::patch`, `tui::ops::stage_hunk`, `unstage_hunk` | `tui_packet1_packet2_test.rs::test_packet2_partial_hunk_staging` |
| **C03** | Individual-line and range staging | upstream-parity | core | partial | `oxidize_diff::patch` line models | Hunk-level staging complete |
| **C04** | Partial discard | upstream-parity | core | verified | `oxidize_diff::patch`, `tui::ops::discard_hunk` | `tui_packet1_packet2_test.rs::test_packet2_partial_hunk_discard` |
| **C05** | Diff presentation controls | upstream-parity | core | verified | `tui::ui::render_inspector`, `app::next_hunk`, `prev_hunk` | `ui.rs` hunk badges & navigation |
| **C06** | Search and selection in diffs | upstream-parity | core | missing | `tui::app`, `tui::ui` | Scheduled for Packet 3 |
| **C07** | External edit/render tools | upstream-parity | optional | missing | `tui::ops`, `TerminalSessionGuard` | Scheduled for Milestone 7 |
| **C08** | Patch safety and scale | foundation | core | verified | `oxidize_diff::patch`, byte-level CRLF/EOF fidelity | `tui_packet1_packet2_test.rs::test_packet2_crlf_line_ending_fidelity` |

---

## D. Commit Creation and Metadata

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **D01** | Commit composer | upstream-parity | core | verified | `ActiveModal::CommitPrompt`, `tui::app` | `tui_stage3_test.rs::test_successful_interactive_commit_workflow` |
| **D02** | External Git editor workflow | upstream-parity | optional | missing | `tui::ops`, `std::process::Command` | Scheduled for Milestone 2 |
| **D03** | Amend HEAD | upstream-parity | core | verified | `ActiveModal::CommitAmend`, `tui::app` | `tui_lazygit_test.rs::test_amend_commit_modal` |
| **D04** | Author/trailer controls | upstream-parity | core | missing | `oxidize_refs::Signature`, `tui::ops` | Scheduled for Milestone 2 |
| **D05** | Fixup/squash commit creation | upstream-parity | core | missing | `tui::ops`, `crates/cli` | Scheduled for Milestone 4 |
| **D06** | Hooks and signing capabilities | upstream-parity | optional | partial | `cli::cmd_fmt`, `tui::ops` | `version_test.rs::test_ox_fmt_help_and_check` |
| **D07** | Commit boundaries | foundation | core | verified | `oxidize_index::write_tree_and_commit` | `root_tests::test_f06_unmerged_index_rejected_in_write_tree_and_commit` |
| **D08** | Commit result and recovery | foundation | core | verified | `tui::app`, `tui::lib` | `tui_packet0_test.rs::test_packet0_recoverable_draft_preservation` |

---

## E. History, Inspection, Search, and Comparisons

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **E01** | Real commit DAG | upstream-parity | core | verified | `tui::app::load_commits_with_graph`, `tui::ui` | `tui_packet3_test.rs::test_packet3_commit_dag_lanes_and_topological_order` |
| **E02** | Commit details and files | upstream-parity | core | verified | `tui::app::compute_commit_diff` (tree diff optimization & OID cache) | `tui_packet3_test.rs::test_packet3_commit_diff_tree_filtering` |
| **E03** | History filters | upstream-parity | core | verified | `tui::app::set_search_filter`, `filtered_commit_indices`, `/` modal | `tui_packet3_test.rs::test_packet3_commit_dag_lanes_and_topological_order` |
| **E04** | Two-ref comparison | upstream-parity | core | missing | `oxidize_diff`, `tui::app` | Scheduled for Milestone 4 |
| **E05** | File history and blame | upstream-parity | core | missing | `cli::cmd_blame`, `tui::app` | Scheduled for Milestone 4 |
| **E06** | Commit actions | upstream-parity | core | verified | `tui::ops::checkout_commit`, `cherry_pick_commit`, `reset_to_commit` | `tui_packet3_test.rs` (cherry-pick, checkout, reset suites) |
| **E07** | Copy/open commit attributes | upstream-parity | optional | missing | `tui::app`, clipboard helper | Scheduled for Milestone 3 |
| **E08** | History scalability | foundation | core | verified | `tui::ui::render_inspector` (virtual slice window `[start..end]`, Ratatui u16 safe) | `tui_packet3_test.rs::test_packet3_inspector_virtualization_massive_diff` |

---

## F. Branches, Tags, and Refs

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **F01** | Local branches | upstream-parity | core | verified | `tui::app::branches`, `tui::ui` | `tui_packet0_test.rs::test_packet0_subtab_safety_remotes_and_tags` |
| **F02** | Branch creation and rename | upstream-parity | core | verified | `ActiveModal::BranchCreate`, `ops::create_branch` | `tui_stage4_test.rs::test_branch_creation_via_modal` |
| **F03** | Branch checkout and deletion | upstream-parity | core | verified | `ops::checkout_branch`, `delete_branch`, packed-ref atomicity | `tui_packet1_packet2_test.rs::test_packet1_packed_only_branch_deletion_atomicity` |
| **F04** | Upstream management | upstream-parity | core | verified | `ahead_behind`, `oxidize_config` branch.<name>.remote/.merge | `tui_packet1_packet2_test.rs::test_packet1_configured_upstream_resolution` |
| **F05** | Merge choices | upstream-parity | core | verified | `tui::ops::fast_forward_merge`, `cli::cmd_merge` | `tui_packet3_test.rs::test_packet3_fast_forward_merge` |
| **F06** | Remote branches | upstream-parity | core | partial | `tui::app::remotes`, `tui::ui` | `tui_lazygit_test.rs::test_remotes_tags_and_reflog_loading` |
| **F07** | Tags | upstream-parity | core | verified | `RefStore::create_tag`, `delete_tag`, `tui::ops`, decorations | `tui_packet3_test.rs::test_packet3_commit_decorations` |
| **F08** | Branch recovery and stacks | upstream-parity | core | missing | `oxidize_refs::reflog` | Scheduled for Milestone 3 |

---

## G. Interactive Rebase and History Editing

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **G01** | Rebase preparation | upstream-parity | core | verified | `tui::app::open_rebase_todo_modal`, `ops::start_interactive_rebase` | `tui_packet4_test.rs::test_packet4_rebase_todo_modal_navigation_and_actions` |
| **G02** | Todo editor | upstream-parity | core | verified | `ActiveModal::RebaseTodo`, `tui::app::handle_rebase_todo_key` | `tui_packet4_test.rs::test_packet4_rebase_todo_modal_navigation_and_actions` |
| **G03** | Replay state machine | foundation | core | verified | Durable sequencer `tui::sequencer::SequencerState`, `.git/rebase-merge/` | `tui_packet4_test.rs::test_packet4_rebase_conflict_stop_continue_and_resolution` |
| **G04** | Reword/edit older commits | upstream-parity | core | verified | `RebaseAction::Reword`, `Edit`, `tui::ops::run_rebase_loop` | `tui_packet4_test.rs::test_packet4_interactive_rebase_reword` |
| **G05** | Autosquash | upstream-parity | core | verified | `RebaseAction::Squash`, `Fixup` message combining | `tui_packet4_test.rs::test_packet4_interactive_rebase_squash_and_fixup` |
| **G06** | Amend old commit from staged | upstream-parity | core | missing | Replay sequencer | Scheduled for Milestone 4 |
| **G07** | Split/reorder/drop commits | upstream-parity | core | verified | `RebaseAction::Drop`, `K`/`J` reordering in Todo modal | `tui_packet4_test.rs::test_packet4_interactive_rebase_pick_reorder_and_drop` |
| **G08** | Complex ancestry handling | foundation | core | verified | 3-way tree merge replay with conflict resolution | `tui_packet4_test.rs::test_packet4_rebase_conflict_stop_continue_and_resolution` |

---

## H. Cherry-Pick, Revert, Reset, and Recovery

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **H01** | Commit copy basket | upstream-parity | core | missing | `tui::app::basket` | Scheduled for Milestone 4 |
| **H02** | Ordered cherry-pick | upstream-parity | core | verified | `tui::ops::cherry_pick_commit`, three-way tree merge | `tui_packet3_test.rs::test_packet3_cherry_pick_commit` |
| **H03** | Revert workflows | upstream-parity | core | verified | `tui::ops::revert_commit`, `ConfirmAction::Revert`, `t` shortcut | `tui_packet4_test.rs::test_packet4_commit_revert_workflow` |
| **H04** | Reset choices | upstream-parity | core | verified | `tui::ops::reset_to_commit` (Soft, Mixed, Hard) | `tui_packet3_test.rs::test_packet3_reset_*` |
| **H05** | Reflog inspection | upstream-parity | core | verified | `tui::app::reflog`, `tui::ui` | `tui_lazygit_test.rs::test_remotes_tags_and_reflog_loading` |
| **H06** | Reflog-based undo | upstream-parity | core | missing | Reflog state journal | Scheduled for Milestone 4 |
| **H07** | Redo and persistence | upstream-parity | core | missing | Reflog state journal | Scheduled for Milestone 4 |
| **H08** | Recovery boundaries | foundation | core | missing | Safe abort / revert | Scheduled for Milestone 4 |

---

## I. Conflict Resolution

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **I01** | Conflict discovery | upstream-parity | core | verified | `oxidize_index::IndexEntry::stage` | `root_tests::test_f06_merge_conflict_records_stages_and_merge_head` |
| **I02** | Conflict views | upstream-parity | core | verified | Unmerged stages index detection, `[U]` Conflicted badge | `tui_packet4_test.rs::test_packet4_rebase_conflict_stop_continue_and_resolution` |
| **I03** | Hunk choices | upstream-parity | core | missing | Resolver interactive mode | Scheduled for Milestone 5 |
| **I04** | File-level choices | upstream-parity | core | verified | Take ours (`o`), theirs (`t`), both (`b`) | `tui_packet4_test.rs::test_packet4_conflict_resolution_theirs_and_both` |
| **I05** | External merge/editor tools | upstream-parity | optional | missing | Mergetool invocation | Scheduled for Milestone 4 |
| **I06** | Resolution staging | upstream-parity | core | verified | Auto stage 0 promotion and unmerged stages clearance | `tui_packet4_test.rs::test_packet4_conflict_resolution_theirs_and_both` |
| **I07** | Local resolution undo | upstream-parity | core | missing | Resolver journal | Scheduled for Milestone 5 |
| **I08** | Continue/skip/abort | upstream-parity | core | verified | `tui::ops::rebase_continue`, `rebase_skip`, `rebase_abort` | `tui_packet4_test.rs::test_packet4_rebase_abort_workflow` |

---

## J. Stash Workflows

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **J01** | Stash list and inspection | upstream-parity | core | verified | `tui::app::stashes`, `tui::ops::list_stashes` | `tui_stage1_test.rs::test_stash_loading_and_inspection` |
| **J02** | Save ordinary tracked changes | upstream-parity | core | verified | `tui::ops::stash_save`, `ActiveModal::StashSave` | `tui_lazygit_test.rs::test_stash_save_and_apply` |
| **J03** | Untracked/ignored variants | upstream-parity | core | verified | `tui::ops::stash_save_with_options` (include untracked) | `tui_packet5_test.rs::test_packet5_untracked_stash_save_and_apply` |
| **J04** | Staged/selected variants | upstream-parity | core | verified | `tui::ops::stash_save_with_options` (staged only, keep index) | `tui_packet5_test.rs::test_packet5_staged_only_stash_save` |
| **J05** | Apply/pop selected stash | upstream-parity | core | verified | `tui::ops::apply_stash`, `pop_stash`, 3rd-parent collision safety | `parity_review_probes.rs::stash_untracked_restore_must_preserve_existing_file` |
| **J06** | Drop/rename selected stash | upstream-parity | core | verified | `tui::ops::drop_stash` | `tui_stage4_test.rs::test_stash_pop_and_drop` |
| **J07** | Branch/worktree from stash | upstream-parity | core | verified | `tui::ops::branch_from_stash` | `tui_packet5_test.rs::test_packet5_stash_branch_workflow` |
| **J08** | Stash interoperability | foundation | core | verified | Round-trip with git; conflict retention | `root_tests::test_f07_stash_stack_middle_drop_and_conflict_preservation` |

---

## K. Custom Patches and Advanced Change Movement

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **K01** | Persistent patch basket | upstream-parity | core | verified | `oxidize_diff::patch::CustomPatchBasket`, `app.custom_patch_basket` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K02** | Patch preview/export | upstream-parity | core | verified | `CustomPatchBasket::format_unified_diff`, `app.open_custom_patch_menu` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K03** | Apply patch to worktree/index | upstream-parity | core | verified | `ops::apply_custom_patch_to_worktree`, `apply_custom_patch_to_index` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K04** | Move patch into a new commit | upstream-parity | core | verified | `ops::create_commit_from_custom_patch` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K05** | Move patch between commits | upstream-parity | core | partial | `ops::apply_custom_patch_to_commit` (HEAD amend supported; non-HEAD returns explicit error) | `parity_review_probes.rs::custom_patch_to_non_head_must_fail_safely` |
| **K06** | Remove selected changes | upstream-parity | core | verified | `app.custom_patch_basket.remove_matching_hunk` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K07** | Patch extraction as inverse | upstream-parity | core | verified | Reverse apply in `ops::apply_custom_patch_to_worktree`/`index` | `tui_packet5_test.rs::test_packet5_custom_patch_basket_operations` |
| **K08** | Patch conflicts and recovery | foundation | core | verified | Slice bounds safety and multi-path preflight | `parity_review_probes.rs::stale_short_patch_must_return_error_without_panicking` |

---

## L. Remotes, Networking, and Provider Integration

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **L01** | Remote management | upstream-parity | core | verified | `ops::add_remote`, `rename_remote`, `remove_remote`, `ActiveModal::RemoteAdd` | `tui_packet6_test.rs::test_packet6_remote_management_native` |
| **L02** | Fetch workflows | upstream-parity | core | partial | `oxidize_transport::protocol`, `ops::fetch_all_remotes` | `transport_compatibility_test.rs` |
| **L03** | Pull choices | upstream-parity | core | partial | `tui::ops::pull_from_remote` | Background pull worker |
| **L04** | Push workflows | upstream-parity | core | partial | `tui::ops::push_to_remote` | Background push worker |
| **L05** | Force-with-lease | upstream-parity | core | verified | `App::push_force_lease` with upstream tracking ref OID preflight | `parity_review_probes.rs::app_force_lease_must_use_recorded_remote_oid` |
| **L06** | Credentials and SSH | upstream-parity | core | partial | `oxidize_transport::ssh` | SSH test suite |
| **L07** | Provider links and PR metadata | upstream-parity | optional | verified | `ops::get_provider_urls`, `app.yank_commit_url`, `app.yank_repo_url` | `tui_packet6_test.rs::test_packet6_yank_and_provider_urls` |
| **L08** | Jobs and failure semantics | foundation | core | verified | `BackgroundJob`, `mpsc`, concurrency lock | `tui_packet0_test.rs::test_packet0_concurrent_mutation_lock` |

---

## M. Worktrees, Submodules, and Repository Context

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **M01** | Worktree list | upstream-parity | core | verified | `ops::list_worktrees`, `app.open_worktree_list_modal` | `tui_packet5_test.rs::test_packet5_linked_worktree_management` |
| **M02** | Worktree creation | upstream-parity | core | verified | `ops::create_worktree`, `ActiveModal::WorktreeCreate` | `tui_packet5_test.rs::test_packet5_linked_worktree_management` |
| **M03** | Worktree switching/tools | upstream-parity | core | verified | `app.switch_to_worktree`, re-anchoring `RepoContext` | `tui_packet5_test.rs::test_packet5_linked_worktree_management` |
| **M04** | Worktree removal and maintenance | upstream-parity | core | verified | `ops::remove_worktree` (reciprocal checks, dirty tree protection) | `parity_review_probes.rs::worktree_removal_must_reject_dirty_unlocked_tree` |
| **M05** | Submodule presentation | upstream-parity | core | verified | `ops::list_submodules`, `SubmoduleItem`, `ActiveModal::SubmoduleList` | `tui_packet6_test.rs::test_packet6_submodule_discovery_init_update_and_navigation` |
| **M06** | Submodule lifecycle | upstream-parity | core | verified | `ops::submodule_init`, `ops::submodule_update` (dirty worktree preflight) | `parity_review_probes.rs::submodule_update_must_reject_dirty_submodule_worktree` |
| **M07** | Bulk/nested operations | upstream-parity | core | verified | `app.enter_submodule`, `app.return_to_parent_repo` (navigation stack) | `tui_packet6_test.rs::test_packet6_submodule_discovery_init_update_and_navigation` |
| **M08** | Cross-context correctness | foundation | core | verified | `oxidize_core::RepoContext::discover`, `RefStore`, `RepoObjectStore` | `tui_packet1_packet2_test.rs::test_packet1_linked_worktree_discovery_and_shared_objects` |

---

## N. Bisect, Branch Stacks, and Advanced Workflows

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **N01** | Bisect setup | upstream-parity | core | verified | `ops::bisect_start`, `ops::get_bisect_state`, `.git/BISECT_*` persistence | `tui_packet6_test.rs::test_packet6_git_bisect_workflow` |
| **N02** | Bisect navigation | upstream-parity | core | verified | `ops::bisect_mark`, `ops::bisect_skip`, midpoint selection | `tui_packet6_test.rs::test_packet6_git_bisect_workflow` |
| **N03** | Bisect run/reset | upstream-parity | core | verified | `ops::bisect_reset`, culprit convergence detection (`culprit_oid`) | `tui_packet6_test.rs::test_packet6_git_bisect_workflow` |
| **N04** | Bisect edge cases | foundation | core | partial | Linear DAG bisect supported; complex octopus DAG merge bisect deferred | `tui_packet6_test.rs` |
| **N05** | Stack-aware branch view | upstream-parity | core | missing | Branch stack hierarchy | Scheduled for future milestone |
| **N06** | Rebase stack choices | upstream-parity | core | missing | Stack rebase | Scheduled for future milestone |
| **N07** | Gitflow optional commands | upstream-parity | optional | missing | Gitflow integration | Optional extension |
| **N08** | Repository diagnostics | extension | core | partial | `cli::cmd_status` | Status diagnostics |

---

## O. Configuration, Shortcuts, and Integrations

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **O01** | Validated app config | upstream-parity | core | partial | `oxidize_config::GitConfig` parser | Configuration tests |
| **O02** | Custom keybindings | upstream-parity | core | missing | Keymap registry | Scheduled for future milestone |
| **O03** | Appearance/accessibility | upstream-parity | core | verified | Theme colors in `ui.rs`, Unicode width rendering | Headless rendering and alignment tests |
| **O04** | Editor, opener, clipboard | upstream-parity | optional | partial | Item yanking (`y`) and web provider permalinks (`O`) | `tui_packet6_test.rs::test_packet6_yank_and_provider_urls` |
| **O05** | Custom commands | upstream-parity | optional | missing | User command exec | Optional extension |
| **O06** | Custom prompts and menus | upstream-parity | core | verified | Interactive command palette (`ActiveModal::CommandPalette`, `Ctrl+P`) | `tui_packet6_test.rs::test_packet6_command_palette_and_filtering` |
| **O07** | Diff renderers and pagers | upstream-parity | optional | missing | External diff pipe | Optional extension |
| **O08** | Config trust and tooling | foundation | core | missing | Command trust policy | Optional extension |

---

## P. Responsiveness, Robustness, and Release Quality

| ID | Feature Name | Classification | Capability | Status | Implementation Path | Verification Evidence |
|---|---|---|---|---|---|---|
| **P01** | Background job framework | foundation | core | verified | `BackgroundJob`, `App` concurrency locks | `tui_packet0_test.rs::test_packet0_concurrent_mutation_lock` |
| **P02** | Snapshot invalidation | foundation | core | verified | `App::refresh`, preflight cache invalidation across mutations | `tui_robustness_test.rs::test_selection_stability_across_refresh` |
| **P03** | Large-repository performance | foundation | core | verified | Inspector slice window virtualization & diff tree-cache | `tui_packet3_test.rs::test_packet3_inspector_virtualization_massive_diff` |
| **P04** | Terminal lifecycle | foundation | core | verified | `TerminalSessionGuard` drop cleanup | `tui_robustness_test.rs::test_terminal_session_guard_state_and_drop` |
| **P05** | Unicode and output safety | foundation | core | verified | `build_footer`, `syntax` highlighter loop guard | `tui_packet0_test.rs` |
| **P06** | Testing and CI | foundation | core | verified | Workspace-wide cargo test suites and probes | All workspace tests passed, 0 failures |
| **P07** | Docs and discoverability | foundation | core | verified | `docs/parity/` suite, review logs, and feature matrix | Full documentation suite |
| **P08** | Benchmark and completion evidence | foundation | core | partial | Release benchmarks across index, status, and pack operations | `performance_benchmark_test.rs` |

