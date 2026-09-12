# Oxidize CLI Reference Manual

`ox` is the command-line entry point for Oxidize. This reference matches the current Clap command surface in `crates/cli/src/main.rs` for version `0.2.1`.

```text
Usage: ox [COMMAND]
```

Global options are limited to `-h/--help` and `-V/--version`. For the authoritative syntax of any command, run `ox <command> --help`.

## Porcelain commands

| Command | Current usage | Purpose |
|---|---|---|
| `init` | `ox init [DIRECTORY]` | Create or reinitialize a repository. |
| `clone` | `ox clone <REPOSITORY> [DIRECTORY]` | Clone from a local, Smart HTTP, or supported SSH repository. |
| `add` | `ox add [FILES]...` | Add file contents to the index. |
| `rm` | `ox rm [OPTIONS] [FILES]...` | Remove tracked paths from the index and optionally the working tree. |
| `mv` | `ox mv <SOURCE> <DESTINATION>` | Move or rename a path. |
| `restore` | `ox restore [OPTIONS] [FILES]...` | Restore working-tree files or staged entries. |
| `status` | `ox status` | Show working-tree and index status. |
| `diff` | `ox diff [OPTIONS]` | Show working-tree changes, or staged changes with `--staged`. |
| `commit` | `ox commit [OPTIONS]` | Record the current index as a commit. |
| `log` | `ox log [OPTIONS]` | Show commit history; supports `--oneline`, `--graph`, `-n/--max-count`, and `--tui`. |
| `branch` | `ox branch [OPTIONS] [NAME]` | List, create, or delete branches. |
| `checkout` | `ox checkout [OPTIONS] [TARGET]` | Switch branches/commits; `-b` creates a branch. |
| `switch` | `ox switch [OPTIONS] [BRANCH]` | Switch branches; `-c/--create` creates one. |
| `merge` | `ox merge [OPTIONS] [COMMIT]` | Merge a commit/branch; `--abort` aborts an in-progress merge. |
| `rebase` | `ox rebase <UPSTREAM>` | Reapply commits on another base. |
| `cherry-pick` | `ox cherry-pick <COMMIT>` | Apply one commit on top of `HEAD`. |
| `revert` | `ox revert <COMMIT>` | Create a commit that reverses another commit. |
| `reset` | `ox reset [OPTIONS] [COMMIT]` | Reset `HEAD` with `--soft`, `--mixed`, or `--hard`. |
| `stash` | `ox stash [COMMAND]` | Manage the stash stack. |
| `tag` | `ox tag [OPTIONS] [NAME] [TARGET]` | List, create, annotate, or delete tags. |
| `remote` | `ox remote [COMMAND]` | List and manage remotes. |
| `fetch` | `ox fetch [REMOTE]` | Fetch objects and refs. |
| `push` | `ox push [OPTIONS] [REMOTE] [BRANCH]` | Push commits/refs; supports `-f/--force`. |
| `pull` | `ox pull [REMOTE] [BRANCH]` | Fetch and integrate a remote branch. |
| `gc` | `ox gc` | Pack loose objects and clean repository storage. |
| `fsck` | `ox fsck` | Verify repository object connectivity and integrity. |
| `reflog` | `ox reflog` | Show reflog information. |
| `blame` | `ox blame <FILE>` | Attribute file lines to commits/authors. |
| `bisect` | `ox bisect [ARGS]...` | Drive the bisect workflow (`start`, `bad`, `good`, `reset`). |
| `ui` | `ox ui` | Launch the interactive terminal UI. Aliases: `lg`, `lazygit`, `tui`. |
| `clean` | `ox clean [OPTIONS]` | Remove untracked files (`-f`, `-d`, `-n/--dry-run`). |
| `config` | `ox config [OPTIONS] [KEY] [VALUE]` | Read or modify local/global configuration. |
| `show` | `ox show [OBJECT]` | Show a commit, tree, blob, or tag. |
| `merge-base` | `ox merge-base <COMMIT1> <COMMIT2>` | Find a common merge ancestor. |

### Remote management

```text
ox remote
ox remote add <NAME> <URL>
ox remote remove <NAME>
ox remote rename <OLD> <NEW>
```

`remote rename` updates the configured remote name, rewrites matching fetch destinations and branch remote settings, and migrates remote-tracking refs.

### Stash management

```text
ox stash push [-m|--message <MESSAGE>]
ox stash pop
ox stash list
ox stash drop [INDEX]
```

### Rust formatter integration

`ox fmt` (alias `ox format`) runs `rustfmt` for Rust sources:

```text
ox fmt [FILES]...
ox fmt --check [FILES]...
ox fmt --staged
ox fmt --check --staged
ox fmt --install-hook
```

With `--staged`, Oxidize materializes the Rust blobs currently stored in the index, formats/checks those staged contents, and updates index object IDs when formatting changes are needed. Working-tree files are left unchanged, which preserves partial staging.

`--install-hook` installs a pre-commit hook that runs `ox fmt --check --staged`.

### Shell completions

```text
ox completions <bash|elvish|fish|powershell|zsh>
```

Example for PowerShell:

```powershell
ox completions powershell >> $PROFILE
```

## Plumbing commands

| Command | Current usage | Purpose |
|---|---|---|
| `hash-object` | `ox hash-object [OPTIONS] [FILE]` | Compute an object ID and optionally write a blob. |
| `cat-file` | `ox cat-file [OPTIONS] <OBJECT>` | Print object type, size, or content. |
| `update-index` | `ox update-index [OPTIONS] [FILES]...` | Modify index entries. |
| `write-tree` | `ox write-tree` | Write the current index as a tree. |
| `read-tree` | `ox read-tree <TREE_ISH>` | Load a tree into the index. |
| `commit-tree` | `ox commit-tree [OPTIONS] -m <MESSAGE> <TREE>` | Create a commit object. |
| `ls-tree` | `ox ls-tree [OPTIONS] <TREE_ISH>` | List tree entries. |
| `ls-files` | `ox ls-files [OPTIONS]` | List index entries. |
| `rev-parse` | `ox rev-parse [OPTIONS] [ARGS]...` | Resolve revision names/arguments. |
| `rev-list` | `ox rev-list <COMMIT>` | Walk commit history. |
| `symbolic-ref` | `ox symbolic-ref <NAME> [TARGET]` | Read or set a symbolic ref. |
| `update-ref` | `ox update-ref <REF_NAME> <NEW_VALUE> [OLD_VALUE]` | Atomically update a ref. |
| `show-ref` | `ox show-ref [OPTIONS]` | List refs and object IDs. |
| `mktree` | `ox mktree` | Build a tree from `ls-tree` formatted stdin. |
| `pack-objects` | `ox pack-objects <BASE_NAME>` | Create a pack archive. |
| `unpack-objects` | `ox unpack-objects` | Unpack objects from stdin. |
| `index-pack` | `ox index-pack <FILE>` | Create an index for a pack file. |
| `verify-pack` | `ox verify-pack [OPTIONS] [FILES]...` | Verify pack/index files. |

## Built-in shorthand aliases

The CLI expands these before Clap parsing:

| Alias | Command |
|---|---|
| `st` | `status` |
| `co` | `checkout` |
| `ci` | `commit` |
| `br` | `branch` |
| `df` | `diff` |
| `rb` | `rebase` |
| `cp` | `cherry-pick` |
| `lg`, `lazygit`, `tui` | `ui` |

Aliases defined under `[alias]` in the local `.git/config` or global `~/.gitconfig` are also expanded by `ox`.
