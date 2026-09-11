# Upstream Lazygit Reference Specification

## Pinned Reference
- **Upstream Project**: [Lazygit](https://github.com/jesseduffield/lazygit)
- **Pinned Release**: `v0.65.0`
- **Pinned Commit**: `v0.65.0` (tag commit `f24e93f`)
- **Pinned Documentation**:
  - Main: https://github.com/jesseduffield/lazygit/tree/v0.65.0
  - Keybindings: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/keybindings/Keybindings_en.md
  - Config: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Config.md
  - Custom Commands: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Custom_Command_Keybindings.md
  - Custom Diff Renderers: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Custom_DiffRenderers.md
  - Fixup Commits: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Fixup_Commits.md
  - Range Select: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Range_Select.md
  - Searching: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Searching.md
  - Stacked Branches: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Stacked_Branches.md
  - Undoing: https://github.com/jesseduffield/lazygit/blob/v0.65.0/docs/Undoing.md

---

## Architecture Alignment

| Area | Lazygit v0.65.0 Convention | Oxidize Native Engine Policy |
|---|---|---|
| **Core Engine** | Spawns system `git` CLI executable for operations | Pure Rust native Git engine (`oxidize_core`, `oxidize_index`, `oxidize_refs`, `oxidize_diff`, `oxidize_pack`, `oxidize_transport`). Never delegate Git protocol or object operations to external system Git. |
| **TUI Shell** | Go gocui multi-view layout | Pure Rust Ratatui + Crossterm 5-panel layout (Status, Files, Branches, Commits, Stash) with Vim navigation and mouse support. |
| **Diff / Patch** | Myers diff via `git diff` output parsing | Native Myers diff engine (`oxidize_diff`), byte-preserving patch model, structured hunks. |
| **Undo / Redo** | Reflog-based undo for checkout/commit/rebase; explicit limitation on discarded working tree bytes | Same explicit boundary: Reflog-based recovery for ref/history changes, clear warning that discarded working-tree edits cannot be recovered. |
| **External Tools** | Editor (`$EDITOR`), Pager, Diff tool, Browser, `gh` | Platform-safe argument construction, capability detection, terminal suspension & restoration via `TerminalSessionGuard`. |
