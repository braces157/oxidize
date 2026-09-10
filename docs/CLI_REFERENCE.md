# Oxidize CLI Reference Manual

`ox` is the unified command-line entry point for Oxidize. It provides both human-friendly porcelain workflows and low-level scriptable plumbing commands.

```
Usage: ox [OPTIONS] <COMMAND>
```

---

## Global Options

| Option | Description |
|---|---|
| `-C <path>` | Run as if `ox` was started in `<path>` instead of current directory |
| `--git-dir <path>` | Set path to the repository directory (`.git`) |
| `--work-tree <path>` | Set path to working tree root |
| `-v, --version` | Print version information (`ox 0.2.0`) |
| `-h, --help` | Print help or subcommand help |

---

## Porcelain Commands

### `ox init`
Create an empty Git repository or reinitialize an existing one.
```bash
ox init [path] [--bare] [--initial-branch <name>]
```
- Creates `.git/` directory structure: `objects/`, `refs/heads/`, `refs/tags/`, `HEAD` (pointing to `refs/heads/master` or specified initial branch), and default `.git/config`.

### `ox status` (alias: `ox st`)
Show working tree and staging area status.
```bash
ox status [--short]
```
- Categorizes files into:
  - Changes to be committed (staged)
  - Changes not staged for commit (modified/deleted in worktree)
  - Untracked files

### `ox add`
Add file contents to the index (staging area).
```bash
ox add <pathspec>... [-A | --all]
```
- Writes blobs to `.git/objects` and records entry in `.git/index`.
- Multi-threaded file scanning and hashing with Rayon.

### `ox commit` (alias: `ox ci`)
Record changes to the repository.
```bash
ox commit -m <message> [--amend] [--author "<name> <email>"]
```
- Writes canonical tree object from current `.git/index`.
- Creates commit object pointing to previous `HEAD` commit.
- Updates branch reference and writes reflog entry.

### `ox diff` (alias: `ox di`)
Show changes between commits, commit and working tree, or index and working tree.
```bash
ox diff [--staged | --cached] [<commit>] [<path>]
```
- `--staged`: Show changes between index and `HEAD`.
- Default: Show changes between working tree and index.
- Outputs unified diff hunks with ANSI color highlights.

### `ox log`
Show commit logs and history.
```bash
ox log [-n <number>] [--oneline] [--graph] [<revision-range>]
```
- `--oneline`: Compact output with 7-character abbreviated hash and commit subject.
- `--graph`: Draws ASCII DAG commit branch visualization.

### `ox show`
Show various types of objects (commits, trees, blobs, tags).
```bash
ox show [<object>]
```
- For commits: displays commit metadata and the unified patch/diff against the parent commit.
- For trees: displays tree entry listings with permissions and OIDs.
- For blobs: prints raw file contents.
- For tags: displays annotated tag details.

### `ox branch` (alias: `ox br`)
List, create, or delete branches.
```bash
ox branch [-a | --all] [-d | --delete <branch>] [<new-branch-name>]
```
- List branches: marks active branch with `*`.
- Create branch: writes reference pointing to `HEAD`.
- Delete branch: removes reference file under `.git/refs/heads/`.

### `ox checkout` / `ox switch`
Switch branches or restore working tree files.
```bash
ox checkout <branch-or-commit> [-b <new-branch>]
ox switch <branch-name> [-c <new-branch>]
```
- Updates `HEAD` to point to target branch or detached commit.
- Checks out index and working tree matching target tree.

### `ox merge`
Join two or more development histories together.
```bash
ox merge <branch> [--no-ff] [-m <message>]
```
- Fast-forward merge when target branch is a direct descendant.
- 3-way merge with common ancestor resolution.

### `ox rebase`
Reapply commits on top of another base tip.
```bash
ox rebase <upstream> [--continue | --abort]
```

### `ox reset`
Reset current HEAD to specified state.
```bash
ox reset [--soft | --mixed | --hard] [<commit>]
```
- `--soft`: Moves `HEAD` only; index and working tree untouched.
- `--mixed`: Moves `HEAD` and resets index; working tree untouched (default).
- `--hard`: Moves `HEAD`, index, and working tree.

### `ox clean`
Remove untracked files from working tree.
```bash
ox clean [-f | --force] [-d] [-n | --dry-run]
```
- `-f, --force`: Required to confirm file deletion.
- `-d`: Remove untracked directories in addition to untracked files.
- `-n, --dry-run`: Don't actually remove anything, just show what would be done.

### `ox config`
Get and set repository or global options.
```bash
ox config [--global] [--list] [--get <key>] [--unset <key>] [<key>] [<value>]
```
- `--global`: Read or write to user global configuration (`~/.gitconfig`).
- `-l, --list`: List all configured settings as `key=value`.
- `--get <key>`: Query the value of a specific setting (e.g. `user.name`).
- `--unset <key>`: Remove a configuration key.
- `<key> <value>`: Set a configuration setting.


### `ox reflog`
Manage and inspect reflog information.
```bash
ox reflog [show] [<ref>]
```
- Lists past positions of references with operation descriptors (commit, checkout, reset, merge).

### `ox clone`
Clone a repository into a new directory.
```bash
ox clone <repository-url> [<directory>]
```
- Supports Smart HTTP (`https://...`) and native SSH (`git@...`).
- Negotiates refs via pkt-line, downloads packfile, indexes pack, and checks out initial branch.

### `ox fetch`
Download objects and refs from remote repository.
```bash
ox fetch [<remote>] [<refspec>]
```

### `ox push`
Update remote refs along with associated objects.
```bash
ox push [<remote>] [<branch>] [-u | --set-upstream]
```

### `ox ui`
Interactive terminal user interface (TUI).
```bash
ox ui
```
- Full-screen dashboard rendered with `ratatui`.
- Navigate branch history, view diffs, stage/unstage files interactively.

### `ox completions`
Generate shell auto-completion scripts.
```bash
ox completions <bash | zsh | fish | powershell | elvish>
```

---

## Plumbing Commands

### `ox cat-file`
Provide content or type and size information for repository objects.
```bash
ox cat-file (-t | -s | -p) <object>
```
- `-t`: Object type (`commit`, `tree`, `blob`, `tag`).
- `-s`: Object size in bytes.
- `-p`: Pretty-print object contents.

### `ox hash-object`
Compute object ID and optionally create a blob from a file.
```bash
ox hash-object [-w] [--stdin] <file>
```
- `-w`: Write the object into `.git/objects`.

### `ox ls-tree`
List the contents of a tree object.
```bash
ox ls-tree [-r] [--name-only] <tree-ish>
```
- Formats entries: `<mode> <type> <hash>\t<filename>`.
- `-r`: Recurse into sub-trees.

### `ox rev-parse`
Parse revision (or other objects) and parameters.
```bash
ox rev-parse <rev> [--verify]
```
- Resolves symbolic references, branch names, and tags into 40-character hexadecimal SHA-1.

### `ox fsck`
Verify the connectivity and validity of the objects in the database.
```bash
ox fsck [--full]
```
- Scans all loose objects, packfiles, and index entries.
- Detects dangling commits, dangling blobs, and SHA-1 corruption.

### `ox rev-list`
Lists commit objects in reverse chronological order along the commit history DAG.
```bash
ox rev-list <commit>
```

### `ox symbolic-ref`
Read, modify and delete symbolic refs.
```bash
ox symbolic-ref <name> [<target>]
```
- `ox symbolic-ref HEAD`: Output current branch reference (e.g. `refs/heads/master`).
- `ox symbolic-ref HEAD <target>`: Atomically redirect symbolic reference.

### `ox update-ref`
Update the object name stored in a ref safely.
```bash
ox update-ref <ref_name> <new_val> [<old_val>]
```

### `ox show-ref`
List references in a local repository and their SHA-1 hashes.
```bash
ox show-ref [-q | --quiet]
```

### `ox read-tree`
Reads tree information into the index staging cache.
```bash
ox read-tree <tree-ish>
```

### `ox merge-base`
Find the lowest common ancestor (merge base) between two commits.
```bash
ox merge-base <commit1> <commit2>
```

