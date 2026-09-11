# Oxidize Test Matrix & Acceptance Journey Coverage

## Automated Test Suites

| Crate / Target | Test File | Primary Coverage |
|---|---|---|
| `oxidize-tui` (unit) | `crates/tui/src/syntax.rs` | Language detection, tokenization, diff line highlighting |
| `oxidize-tui` (integration) | `tests/tui_lazygit_test.rs` | Window navigation, subtabs, status panel, help modals, stash, amend |
| `oxidize-tui` (integration) | `tests/tui_mouse_test.rs` | Mouse clicks, double clicks, right clicks, wheel scrolling, buttons |
| `oxidize-tui` (integration) | `tests/tui_robustness_test.rs` | Unicode text editing, terminal guards, small viewports, background jobs |
| `oxidize-tui` (integration) | `tests/tui_stage1_test.rs` | Layouts, panels, repository loading, inspector scrolling |
| `oxidize-tui` (integration) | `tests/tui_stage2_test.rs` | File staging, unstaging, discarding |
| `oxidize-tui` (integration) | `tests/tui_stage3_test.rs` | Interactive commit composer, modal text editing |
| `oxidize-tui` (integration) | `tests/tui_stage4_test.rs` | Branch creation, deletion, switching, stash pop/drop |
| `oxidize-tui` (integration) | `tests/tui_packet0_test.rs` | Packet 0 safety: highlighter loop, error visibility, subtabs, footer width, lock, draft |
| Root integration tests | `tests/*_test.rs` | CLI/TUI engine parity, object stores, index formats, transports |

## Acceptance Journeys (from Specification)

- **Journey 1**: Split an ordinary change into two commits (Milestone 2)
- **Journey 2**: Stale patch protection (Milestone 2)
- **Journey 3**: Everyday branch and remote flow (Milestones 1 & 6)
- **Journey 4**: Interactive rebase with restart (Milestone 4)
- **Journey 5**: Cherry-pick range and revert (Milestone 4)
- **Journey 6**: Custom patch across history (Milestone 5)
- **Journey 7**: Stash stack integrity (Milestone 1 & 5)
- **Journey 8**: Conflict types (Milestone 4)
- **Journey 9**: Worktree/submodule isolation (Milestone 6)
- **Journey 10**: Recovery and bisect (Milestones 3, 4, 7)
- **Journey 11**: Configuration and external tools (Milestone 7)
- **Journey 12**: Scale and hostile display content (Milestones 1 & 8)
