//! `ox` — Production-quality, daily-driver-capable Git implementation in Rust.

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use oxidize_config::{GitConfig, GitIgnore};
use oxidize_core::store::parse_object_from_content;
use oxidize_core::{
    find_git_dir, strip_verbatim_prefix, Blob, Commit, FileMode, LockFile, LooseObjectStore,
    Object, ObjectId, ObjectReader, ObjectType, RepoContext, Signature, Tag as CoreTag, Tree,
    TreeEntry,
};
use oxidize_diff::{format_unified_diff, three_way_merge};
use oxidize_index::{
    compute_status_with_ignore, flatten_tree, write_tree, Index, IndexEntry, StagedChange,
    UnstagedChange,
};
use oxidize_pack::{
    index_packfile, read_pack_object_at, unpack_packfile, write_pack, PackIndex, RawPackObject,
    RepoObjectStore,
};
use oxidize_refs::{get_default_signature, RefStore};
use oxidize_transport::{
    discover_local_refs, fetch_local_pack, is_ssh_url, resolve_local_path, SmartHttpClient,
    SshClient,
};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "ox",
    version,
    about = "Oxidize: A complete, byte-compatible Git implementation in Rust",
    long_about = "Oxidize: A from-scratch, production-quality reimplementation of Git in Rust.\nCompatible with real Git repositories, packfiles, index formats, and protocols."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    // --- Plumbing ---
    /// Compute object ID and optionally creates a blob from a file
    HashObject {
        /// Actually write the object into the object database
        #[arg(short = 'w')]
        write: bool,

        /// Read object from stdin
        #[arg(long)]
        stdin: bool,

        /// Specify the object type (default: 'blob')
        #[arg(short = 't', default_value = "blob")]
        object_type: String,

        /// File path to hash
        file: Option<String>,
    },

    /// Provide content or type and size information for repository objects
    CatFile {
        /// Pretty-print the contents of `<object>` based on its type
        #[arg(short = 'p')]
        pretty: bool,

        /// Show the object type
        #[arg(short = 't')]
        show_type: bool,

        /// Show the object size
        #[arg(short = 's')]
        show_size: bool,

        /// The name or SHA-1 of the object to show
        object: String,
    },

    /// Register file contents in the working tree to the index
    UpdateIndex {
        /// If a specified file isn't in the index already then it's added
        #[arg(long)]
        add: bool,

        /// Set the execute permissions on the updated files (+x or -x)
        #[arg(long)]
        chmod: Option<String>,

        /// Set the version of the index file
        #[arg(long)]
        index_version: Option<u32>,

        /// Files to act on
        #[arg(default_value = "")]
        files: Vec<String>,
    },

    /// Create a tree object from the current index
    WriteTree,

    /// Reads tree information into the index
    ReadTree {
        /// The tree-ish to read
        tree_ish: String,
    },

    /// Create a new commit object
    CommitTree {
        /// An existing tree object
        tree: String,

        /// Parent commit object(s)
        #[arg(short = 'p')]
        parents: Vec<String>,

        /// Commit message
        #[arg(short = 'm')]
        message: String,
    },

    /// List the contents of a tree object
    LsTree {
        /// Recurse into sub-trees
        #[arg(short = 'r')]
        recurse: bool,

        /// Show object size of blob (file) entries
        #[arg(short = 'l', long)]
        long: bool,

        /// Id of the tree to inspect
        tree_ish: String,
    },

    /// Show information about files in the index and working tree
    LsFiles {
        /// Show staged contents' mode bits, object name and stage number in the output
        #[arg(short = 's', long)]
        stage: bool,
    },

    /// Pick out and massage parameters
    RevParse {
        /// Verify that exactly one parameter is provided, and that it can be resolved as an object
        #[arg(long)]
        verify: bool,

        /// Arguments to resolve
        args: Vec<String>,
    },

    /// Lists commit objects in reverse chronological order
    RevList {
        /// Commit or range to list
        commit: String,
    },

    /// Read, modify and delete symbolic refs
    SymbolicRef {
        /// Ref name to query or set
        name: String,

        /// Target ref if setting
        target: Option<String>,
    },

    /// Update the object name stored in a ref safely
    UpdateRef {
        /// The ref to update
        ref_name: String,

        /// The new object id
        new_value: String,

        /// An old object id which must match the current ref value
        old_value: Option<String>,
    },

    /// List references in a local repository
    ShowRef {
        /// Do not print any results to stdout
        #[arg(short = 'q', long)]
        quiet: bool,
    },

    /// Build a tree-object from ls-tree formatted text
    Mktree,

    /// Create a packed archive of objects
    PackObjects {
        /// Base name of the packfile
        base_name: String,
    },

    /// Unpack objects from a packed archive
    UnpackObjects,

    /// Build pack index file for an existing packed archive
    IndexPack {
        /// The pack file to index
        file: String,
    },

    /// Verify packed git archive files
    VerifyPack {
        /// Show detailed information
        #[arg(short = 'v', long)]
        verbose: bool,

        /// Pack index files to verify
        files: Vec<String>,
    },

    // --- Porcelain ---
    /// Create an empty Git repository or reinitialize an existing one
    Init {
        /// Directory where repository should be created
        directory: Option<String>,
    },

    /// Clone a repository into a new directory
    Clone {
        /// The repository to clone from
        repository: String,

        /// The name of a new directory to clone into
        directory: Option<String>,
    },

    /// Add file contents to the index
    Add {
        /// Files to add content from
        files: Vec<String>,
    },

    /// Remove files from the working tree and from the index
    Rm {
        /// Remove from index only
        #[arg(long)]
        cached: bool,

        /// Allow recursive removal when a leading directory name is given
        #[arg(short = 'r')]
        recursive: bool,

        /// Override the up-to-date check
        #[arg(short = 'f', long)]
        force: bool,

        /// Files to remove
        files: Vec<String>,
    },

    /// Move or rename a file, a directory, or a symlink
    Mv {
        /// Source path
        source: String,
        /// Destination path
        destination: String,
    },

    /// Restore working tree files
    Restore {
        /// Restore the index instead of the working tree
        #[arg(long)]
        staged: bool,

        /// Files to restore
        files: Vec<String>,
    },

    /// Show the working tree status
    Status,

    /// Show changes between commits, commit and working tree, etc
    Diff {
        /// Show staged changes vs HEAD
        #[arg(long)]
        staged: bool,
    },

    /// Record changes to the repository
    Commit {
        /// Use the given message as the commit message
        #[arg(short = 'm', long)]
        message: Option<String>,
    },

    /// Show commit logs
    Log {
        /// Number of commits to show
        #[arg(short = 'n', long)]
        max_count: Option<usize>,

        /// Pretty-print the contents on a single line
        #[arg(long)]
        oneline: bool,

        /// Draw a text-based graphical representation of the commit history
        #[arg(long)]
        graph: bool,

        /// Launch interactive TUI log viewer
        #[arg(long)]
        tui: bool,
    },

    /// List, create, or delete branches
    Branch {
        /// Delete a branch
        #[arg(short = 'd', long)]
        delete: bool,

        /// Force delete a branch
        #[arg(short = 'D')]
        force_delete: bool,

        /// List all branches
        #[arg(short = 'a', long)]
        all: bool,

        /// Branch name to create
        name: Option<String>,
    },

    /// Switch branches or restore working tree files
    Checkout {
        /// Create and checkout a new branch
        #[arg(short = 'b')]
        create_branch: Option<String>,

        /// Target branch or commit
        target: Option<String>,
    },

    /// Switch branches
    Switch {
        /// Create and switch to a new branch
        #[arg(short = 'c', long)]
        create: Option<String>,

        /// Branch name
        branch: Option<String>,
    },

    /// Join two or more development histories together
    Merge {
        /// Abort the current in-progress merge
        #[arg(long)]
        abort: bool,

        /// Commit or branch to merge into HEAD
        commit: Option<String>,
    },

    /// Reapply commits on top of another base tip
    Rebase {
        /// Upstream branch to compare against
        upstream: String,
    },

    /// Apply the changes introduced by some existing commits
    CherryPick {
        /// Commit to cherry-pick
        commit: String,
    },

    /// Revert some existing commits
    Revert {
        /// Commit to revert
        commit: String,
    },

    /// Reset current HEAD to the specified state
    Reset {
        /// Resets the index and working tree
        #[arg(long)]
        hard: bool,

        /// Does not touch the index or working tree
        #[arg(long)]
        soft: bool,

        /// Resets index but not working tree (default)
        #[arg(long)]
        mixed: bool,

        /// Target commit (defaults to HEAD)
        commit: Option<String>,
    },

    /// Stash the changes in a dirty working directory away
    Stash {
        /// Stash subcommand (push, pop, list, drop)
        #[command(subcommand)]
        subcommand: Option<StashCommand>,
    },

    /// Create, list, delete or verify a tag object signed with GPG
    Tag {
        /// Delete tag with given name
        #[arg(short = 'd', long)]
        delete: bool,

        /// Create an annotated tag
        #[arg(short = 'a', long)]
        annotate: bool,

        /// Tag message
        #[arg(short = 'm', long)]
        message: Option<String>,

        /// Tag name
        name: Option<String>,

        /// Target commit or object
        target: Option<String>,
    },

    /// Manage set of tracked repositories
    Remote {
        /// Subcommand (add, remove, -v)
        #[command(subcommand)]
        subcommand: Option<RemoteCommand>,
    },

    /// Download objects and refs from another repository
    Fetch {
        /// Remote repository to fetch from
        remote: Option<String>,
    },

    /// Update remote refs along with associated objects
    Push {
        /// Remote repository to push to
        remote: Option<String>,

        /// Refspec or branch to push
        branch: Option<String>,

        /// Force update of remote refs
        #[arg(short, long)]
        force: bool,
    },

    /// Fetch from and integrate with another repository or a local branch
    Pull {
        /// Remote repository
        remote: Option<String>,

        /// Branch to pull
        branch: Option<String>,
    },

    /// Cleanup unnecessary files and optimize the local repository
    Gc,

    /// Verifies the connectivity and validity of the objects in the database
    Fsck,

    /// Manage reflog information
    Reflog,

    /// Show what revision and author last modified each line of a file
    Blame {
        /// File to blame
        file: String,
    },

    /// Use binary search to find the commit that introduced a bug
    Bisect {
        /// Bisect subcommand
        args: Vec<String>,
    },

    /// Launch interactive terminal UI dashboard (LazyOx / LazyGit replica)
    #[command(alias = "lazygit", alias = "lg", alias = "tui")]
    Ui,

    /// Generate shell completions for the specified shell
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Remove untracked files from the working tree
    Clean {
        /// Force removal of untracked files
        #[arg(short = 'f', long)]
        force: bool,

        /// Remove untracked directories in addition to untracked files
        #[arg(short = 'd')]
        directories: bool,

        /// Don't actually remove anything, just show what would be done
        #[arg(short = 'n', long)]
        dry_run: bool,
    },

    /// Get and set repository or global options
    Config {
        /// Use global ~/.gitconfig file
        #[arg(long)]
        global: bool,

        /// List all variables set in config file
        #[arg(short = 'l', long)]
        list: bool,

        /// Get the value for a given key
        #[arg(long)]
        get: Option<String>,

        /// Remove a setting
        #[arg(long)]
        unset: Option<String>,

        /// Key name (e.g. user.name)
        key: Option<String>,

        /// Value to set
        value: Option<String>,
    },

    /// Show various types of objects (commits, trees, blobs, tags)
    Show {
        /// Object to show (default: HEAD)
        object: Option<String>,
    },

    /// Find as good common ancestors as possible for a merge
    MergeBase {
        /// First commit
        commit1: String,
        /// Second commit
        commit2: String,
    },

    /// Format Rust source code files across the repository or check formatting
    #[command(alias = "format")]
    Fmt {
        /// Check formatting without overwriting files (exits with non-zero if unformatted)
        #[arg(long)]
        check: bool,

        /// Format only staged files in git
        #[arg(long)]
        staged: bool,

        /// Install a Git pre-commit hook to automatically verify formatting before committing
        #[arg(long)]
        install_hook: bool,

        /// Specific files or directories to format (defaults to workspace)
        files: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum StashCommand {
    /// Save local modifications to a new stash
    Push {
        /// Description of the stash
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
    /// Remove a single stashed state from the stash list and apply it on top of the current working tree state
    Pop,
    /// List the stashes that you currently have
    List,
    /// Remove a single stashed state from the stash list
    Drop {
        /// Stash index
        index: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
enum RemoteCommand {
    /// Add a remote named `<name>` for the repository at `<url>`
    Add {
        /// Name of the remote
        name: String,
        /// URL of the remote
        url: String,
    },
    /// Remove the remote named `<name>`
    Remove {
        /// Name of the remote
        name: String,
    },
    /// Rename the remote named `<old>` to `<new>`
    Rename {
        /// Existing remote name
        old: String,
        /// New remote name
        new: String,
    },
}

fn main() -> Result<()> {
    // Windows default main thread stack is 1MB, which can overflow during Clap AST traversal
    // with 40+ subcommands. Spawn with an 8MB stack like cargo/rustc.
    let builder = std::thread::Builder::new().stack_size(8 * 1024 * 1024);
    let handler = builder.spawn(|| -> Result<()> {
        let raw_args: Vec<String> = std::env::args().collect();
        let expanded_args = expand_aliases(raw_args);
        let cli = Cli::parse_from(expanded_args);

        match cli.command {
            Some(cmd) => dispatch_command(cmd)?,
            None => {
                // If no subcommand given, print short help
                println!("ox: A complete Git implementation in Rust. Run `ox --help` for usage.");
            }
        }

        Ok(())
    })?;

    match handler.join() {
        Ok(res) => res,
        Err(e) => std::panic::resume_unwind(e),
    }
}

fn expand_aliases(raw_args: Vec<String>) -> Vec<String> {
    if raw_args.len() <= 1 {
        return raw_args;
    }

    let subcmd = &raw_args[1];
    if subcmd.starts_with('-') {
        return raw_args;
    }

    // Built-in standard Git aliases
    let builtin_expansion = match subcmd.as_str() {
        "st" => Some("status"),
        "co" => Some("checkout"),
        "ci" => Some("commit"),
        "br" => Some("branch"),
        "df" => Some("diff"),
        "rb" => Some("rebase"),
        "cp" => Some("cherry-pick"),
        "lg" => Some("ui"),
        "lazygit" => Some("ui"),
        "tui" => Some("ui"),
        _ => None,
    };

    // Config aliases: check local repository .git/config then global ~/.gitconfig
    let mut config_alias: Option<String> = None;
    if let Ok(git_dir) = find_git_dir(Path::new(".")) {
        let config_path = git_dir.join("config");
        if let Ok(config) = GitConfig::load_from_file(config_path) {
            if let Some(cmd) = config.get_alias(subcmd) {
                config_alias = Some(cmd.to_string());
            }
        }
    }

    if config_alias.is_none() {
        if let Some(home) = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
        {
            let global_path = home.join(".gitconfig");
            if let Ok(config) = GitConfig::load_from_file(global_path) {
                if let Some(cmd) = config.get_alias(subcmd) {
                    config_alias = Some(cmd.to_string());
                }
            }
        }
    }

    let expansion_str = if let Some(ref c) = config_alias {
        c.as_str()
    } else if let Some(b) = builtin_expansion {
        b
    } else {
        return raw_args;
    };

    let mut expanded = Vec::new();
    expanded.push(raw_args[0].clone());

    // Tokenize alias expansion respecting simple quotes
    let parts = tokenize_command(expansion_str);
    expanded.extend(parts);
    expanded.extend_from_slice(&raw_args[2..]);
    expanded
}

fn tokenize_command(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for c in cmd.chars() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn dispatch_command(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::HashObject {
            write,
            stdin,
            object_type,
            file,
        } => cmd_hash_object(write, stdin, object_type, file)?,
        Commands::CatFile {
            pretty,
            show_type,
            show_size,
            object,
        } => cmd_cat_file(pretty, show_type, show_size, object)?,
        Commands::Init { directory } => cmd_init(directory)?,
        Commands::LsTree {
            recurse,
            long,
            tree_ish,
        } => cmd_ls_tree(recurse, long, tree_ish)?,
        Commands::Mktree => cmd_mktree()?,
        Commands::Add { files } => cmd_add(files)?,
        Commands::Status => cmd_status()?,
        Commands::LsFiles { stage } => cmd_ls_files(stage)?,
        Commands::UpdateIndex {
            add,
            chmod,
            index_version,
            files,
        } => cmd_update_index(add, chmod, index_version, files)?,
        Commands::WriteTree => cmd_write_tree()?,
        Commands::Commit { message } => cmd_commit(message)?,
        Commands::Log {
            max_count,
            oneline,
            graph,
            tui,
        } => cmd_log(max_count, oneline, graph, tui)?,
        Commands::Diff { staged } => cmd_diff(staged)?,
        Commands::RevParse { verify, args } => cmd_rev_parse(verify, args)?,
        Commands::CommitTree {
            tree,
            parents,
            message,
        } => cmd_commit_tree(tree, parents, message)?,
        Commands::Branch {
            delete,
            force_delete,
            all,
            name,
        } => cmd_branch(delete, force_delete, all, name)?,
        Commands::Checkout {
            create_branch,
            target,
        } => cmd_checkout(create_branch, target)?,
        Commands::Switch { create, branch } => cmd_switch(create, branch)?,
        Commands::Merge { abort, commit } => cmd_merge(abort, commit)?,
        Commands::Reset {
            hard,
            soft,
            mixed,
            commit,
        } => cmd_reset(hard, soft, mixed, commit)?,
        Commands::PackObjects { base_name } => cmd_pack_objects(base_name)?,
        Commands::UnpackObjects => cmd_unpack_objects()?,
        Commands::IndexPack { file } => cmd_index_pack(file)?,
        Commands::VerifyPack { verbose, files } => cmd_verify_pack(verbose, files)?,
        Commands::Gc => cmd_gc()?,
        Commands::Fsck => cmd_fsck()?,
        Commands::Clone {
            repository,
            directory,
        } => cmd_clone(repository, directory)?,
        Commands::Fetch { remote } => cmd_fetch(remote)?,
        Commands::Pull { remote, branch } => cmd_pull(remote, branch)?,
        Commands::Push {
            remote,
            branch,
            force,
        } => cmd_push(remote, branch, force)?,
        Commands::Remote { subcommand } => cmd_remote(subcommand)?,
        Commands::Rm {
            cached,
            recursive,
            force,
            files,
        } => cmd_rm(cached, recursive, force, files)?,
        Commands::Mv {
            source,
            destination,
        } => cmd_mv(source, destination)?,
        Commands::Restore { staged, files } => cmd_restore(staged, files)?,
        Commands::Rebase { upstream } => cmd_rebase(upstream)?,
        Commands::CherryPick { commit } => cmd_cherry_pick(commit)?,
        Commands::Revert { commit } => cmd_revert(commit)?,
        Commands::Stash { subcommand } => cmd_stash(subcommand)?,
        Commands::Tag {
            delete,
            annotate,
            message,
            name,
            target,
        } => cmd_tag(delete, annotate, message, name, target)?,
        Commands::Blame { file } => cmd_blame(file)?,
        Commands::Bisect { args } => cmd_bisect(args)?,
        Commands::Reflog => cmd_reflog()?,
        Commands::Ui => {
            let git_dir = find_git_dir(Path::new("."))
                .context("fatal: not a git repository (or any of the parent directories): .git")?;
            oxidize_tui::run_tui(&git_dir)?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "ox", &mut io::stdout());
        }
        Commands::ReadTree { tree_ish } => cmd_read_tree(tree_ish)?,
        Commands::RevList { commit } => cmd_rev_list(commit)?,
        Commands::SymbolicRef { name, target } => cmd_symbolic_ref(name, target)?,
        Commands::UpdateRef {
            ref_name,
            new_value,
            old_value,
        } => cmd_update_ref(ref_name, new_value, old_value)?,
        Commands::ShowRef { quiet } => cmd_show_ref(quiet)?,
        Commands::Clean {
            force,
            directories,
            dry_run,
        } => cmd_clean(force, directories, dry_run)?,
        Commands::Config {
            global,
            list,
            get,
            unset,
            key,
            value,
        } => cmd_config(global, list, get, unset, key, value)?,
        Commands::Show { object } => cmd_show(object)?,
        Commands::MergeBase { commit1, commit2 } => cmd_merge_base(commit1, commit2)?,
        Commands::Fmt {
            check,
            staged,
            install_hook,
            files,
        } => cmd_fmt(check, staged, install_hook, files)?,
    }
    Ok(())
}

fn cmd_fmt(check: bool, staged: bool, install_hook: bool, files: Vec<String>) -> Result<()> {
    if install_hook {
        let git_dir = find_git_dir(Path::new("."))
            .context("fatal: not a git repository (or any of the parent directories): .git")?;
        let hooks_dir = git_dir.join("hooks");
        std::fs::create_dir_all(&hooks_dir)?;
        let hook_path = hooks_dir.join("pre-commit");
        let hook_content = "#!/bin/sh\n# Oxidize pre-commit code formatting check\nox fmt --check --staged || {\n    echo \"Error: Staged files have formatting issues. Run 'ox fmt --staged' to fix.\"\n    exit 1\n}\n";
        std::fs::write(&hook_path, hook_content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook_path, perms)?;
        }
        println!("✓ Pre-commit hook installed to {}", hook_path.display());
        return Ok(());
    }

    if staged {
        return format_staged_blobs(check);
    }

    if !files.is_empty() {
        return format_files(&files, check);
    }

    // Default: format entire workspace with cargo fmt or rustfmt
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("fmt");
    cmd.arg("--all");
    if check {
        cmd.args(["--", "--check"]);
    }
    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            if check {
                println!("✓ Code formatting check passed.");
            } else {
                println!("✓ Workspace code formatted.");
            }
        }
        Err(_) => {
            let mut rust_files = Vec::new();
            collect_rust_files(Path::new("."), &mut rust_files);
            if rust_files.is_empty() {
                println!("No Rust files found to format.");
                return Ok(());
            }
            format_files(&rust_files, check)?;
        }
    }
    Ok(())
}

fn format_staged_blobs(check: bool) -> Result<()> {
    let ctx = RepoContext::discover(Path::new("."))
        .context("fatal: not a git repository (or any of the parent directories): .git")?;
    ctx.worktree
        .as_deref()
        .context("cannot format staged files in a bare repository")?;
    let index_path = ctx.git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;
    let store = RepoObjectStore::open_with_common_dir(&ctx.git_dir, &ctx.common_dir)?;
    let ref_store = RefStore::with_common_dir(&ctx.git_dir, &ctx.common_dir);

    let head_files = match ref_store.resolve_head()?.1 {
        Some(head_oid) => match store.read_object(&head_oid)? {
            Object::Commit(commit) => flatten_tree(&store, &commit.tree, "")?,
            _ => bail!("HEAD does not point to a commit"),
        },
        None => BTreeMap::new(),
    };

    let staged_entry_indexes: Vec<usize> = index
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.stage == 0 && entry.path.ends_with(".rs"))
        .filter(|(_, entry)| {
            head_files
                .get(&entry.path)
                .is_none_or(|(mode, oid)| *oid != entry.oid || mode.0 != entry.mode)
        })
        .map(|(idx, _)| idx)
        .collect();

    if staged_entry_indexes.is_empty() {
        println!("No staged Rust files to format.");
        return Ok(());
    }

    let temp_dir = tempfile::Builder::new()
        .prefix("ox-fmt-staged-")
        .tempdir()
        .context("failed to create temporary directory for staged formatting")?;
    let mut unformatted = Vec::new();
    let mut changed = 0usize;

    for (temp_index, entry_index) in staged_entry_indexes.iter().copied().enumerate() {
        let entry = index.entries[entry_index].clone();
        let (obj_type, staged_bytes) = store.read_raw(&entry.oid)?;
        if obj_type != ObjectType::Blob {
            bail!("staged entry '{}' does not reference a blob", entry.path);
        }

        let temp_path = temp_dir.path().join(format!("{}.rs", temp_index));
        std::fs::write(&temp_path, &staged_bytes)
            .with_context(|| format!("failed to materialize staged blob for '{}'", entry.path))?;

        let mut cmd = std::process::Command::new("rustfmt");
        cmd.args(["--edition", "2021"]);
        if check {
            cmd.arg("--check");
        }
        cmd.arg(&temp_path);
        let status = cmd
            .status()
            .with_context(|| format!("failed to execute rustfmt for staged '{}'", entry.path))?;
        if !status.success() {
            if check {
                unformatted.push(entry.path);
                continue;
            }
            bail!("rustfmt failed for staged '{}'", entry.path);
        }

        if !check {
            let formatted = std::fs::read(&temp_path)
                .with_context(|| format!("failed to read formatted staged '{}'", entry.path))?;
            if formatted != staged_bytes {
                let new_oid = store.write_blob(&formatted)?;
                index.entries[entry_index].oid = new_oid;
                changed += 1;
            }
        }
    }

    if !unformatted.is_empty() {
        bail!(
            "staged Rust formatting check failed for: {}",
            unformatted.join(", ")
        );
    }

    if !check && changed > 0 {
        index.write_to(&index_path)?;
    }

    if check {
        println!(
            "✓ Formatting check passed for {} staged file(s).",
            staged_entry_indexes.len()
        );
    } else {
        println!(
            "✓ Formatted {} staged file(s) in the index; working tree left unchanged.",
            staged_entry_indexes.len()
        );
    }
    Ok(())
}

fn format_files(files: &[String], check: bool) -> Result<()> {
    let mut failed = false;
    for file in files {
        let mut cmd = std::process::Command::new("rustfmt");
        if check {
            cmd.arg("--check");
        }
        cmd.arg(file);
        match cmd.status() {
            Ok(status) => {
                if !status.success() {
                    failed = true;
                }
            }
            Err(e) => {
                bail!("failed to execute rustfmt on '{}': {}", file, e);
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
    if check {
        println!("✓ Formatting check passed for {} file(s).", files.len());
    } else {
        println!("✓ Formatted {} file(s).", files.len());
    }
    Ok(())
}

fn collect_rust_files(dir: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "target" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                collect_rust_files(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path.to_string_lossy().to_string());
            }
        }
    }
}

fn cmd_hash_object(
    write: bool,
    stdin: bool,
    object_type_str: String,
    file: Option<String>,
) -> Result<()> {
    let data = if stdin {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf)?;
        buf
    } else if let Some(path) = file {
        std::fs::read(&path).with_context(|| format!("failed to read '{}'", path))?
    } else {
        bail!("no file or --stdin specified");
    };

    let obj_type: ObjectType = object_type_str.parse()?;
    let object = match obj_type {
        ObjectType::Blob => Object::Blob(Blob::new(data)),
        ObjectType::Tree => {
            let mut full_bytes = format!("tree {}\0", data.len()).into_bytes();
            full_bytes.extend_from_slice(&data);
            oxidize_core::store::parse_loose_object(&full_bytes)?
        }
        ObjectType::Commit => {
            let mut full_bytes = format!("commit {}\0", data.len()).into_bytes();
            full_bytes.extend_from_slice(&data);
            oxidize_core::store::parse_loose_object(&full_bytes)?
        }
        ObjectType::Tag => {
            let mut full_bytes = format!("tag {}\0", data.len()).into_bytes();
            full_bytes.extend_from_slice(&data);
            oxidize_core::store::parse_loose_object(&full_bytes)?
        }
    };

    let oid = object.id();

    if write {
        let git_dir = find_git_dir(Path::new("."))?;
        let store = LooseObjectStore::new(git_dir.join("objects"));
        store.write_object(&object)?;
    }

    println!("{}", oid);
    Ok(())
}

fn cmd_cat_file(pretty: bool, show_type: bool, show_size: bool, object_ref: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let oid = store.find_by_prefix(&object_ref)?;
    let obj = store.read_object(&oid)?;

    if show_type {
        println!("{}", obj.object_type().as_str());
        return Ok(());
    }

    if show_size {
        println!("{}", obj.serialize_content().len());
        return Ok(());
    }

    if pretty {
        match obj {
            Object::Blob(blob) => {
                io::stdout().write_all(&blob.data)?;
            }
            Object::Tree(tree) => {
                for entry in tree.entries {
                    println!(
                        "{} {} {}\t{}",
                        entry.mode.display_str(),
                        entry.mode.object_type().as_str(),
                        entry.id,
                        entry.name
                    );
                }
            }
            Object::Commit(_) | Object::Tag(_) => {
                io::stdout().write_all(&obj.serialize_content())?;
            }
        }
        return Ok(());
    }

    bail!("one of -p, -t, or -s must be specified");
}

fn cmd_init(directory: Option<String>) -> Result<()> {
    let target = directory
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let git_dir = target.join(".git");

    if git_dir.exists() {
        println!(
            "Reinitialized existing Git repository in {}",
            git_dir.display()
        );
        return Ok(());
    }

    std::fs::create_dir_all(git_dir.join("objects"))?;
    std::fs::create_dir_all(git_dir.join("refs/heads"))?;
    std::fs::create_dir_all(git_dir.join("refs/tags"))?;

    // Default HEAD pointing to master
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/master\n")?;

    let config = "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\tlogallrefupdates = true\n\tsymlinks = false\n\tignorecase = true\n";
    std::fs::write(git_dir.join("config"), config)?;

    println!("Initialized empty Git repository in {}", git_dir.display());
    Ok(())
}

fn cmd_ls_tree(recurse: bool, long: bool, tree_ish: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let oid = store.find_by_prefix(&tree_ish)?;
    let obj = store.read_object(&oid)?;

    let tree_oid = match obj {
        Object::Tree(_) => oid,
        Object::Commit(commit) => commit.tree,
        _ => bail!("not a tree-ish: {}", tree_ish),
    };

    print_tree_entries(&store, &tree_oid, "", recurse, long)?;
    Ok(())
}

fn print_tree_entries(
    store: &impl oxidize_core::ObjectReader,
    tree_oid: &ObjectId,
    prefix: &str,
    recurse: bool,
    long: bool,
) -> Result<()> {
    let obj = store.read_object(tree_oid)?;
    let tree = match obj {
        Object::Tree(t) => t,
        _ => bail!("expected tree object {}", tree_oid),
    };

    for entry in tree.entries {
        let full_path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        if entry.mode.is_tree() && recurse {
            print_tree_entries(store, &entry.id, &full_path, recurse, long)?;
        } else if long {
            let size_str = if entry.mode.is_tree() {
                "-".to_string()
            } else {
                let item = store.read_object(&entry.id)?;
                format!("{}", item.serialize_content().len())
            };
            println!(
                "{} {} {} {:>7}\t{}",
                entry.mode.display_str(),
                entry.mode.object_type().as_str(),
                entry.id,
                size_str,
                full_path
            );
        } else {
            println!(
                "{} {} {}\t{}",
                entry.mode.display_str(),
                entry.mode.object_type().as_str(),
                entry.id,
                full_path
            );
        }
    }
    Ok(())
}

fn cmd_mktree() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = LooseObjectStore::new(git_dir.join("objects"));

    let stdin = io::stdin();
    let mut entries = Vec::new();

    for line_res in stdin.lock().lines() {
        let line = line_res?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: "<mode> <type> <sha1>\t<name>"
        let (left, name) = line
            .split_once('\t')
            .ok_or_else(|| anyhow::anyhow!("invalid mktree line (missing tab): {}", line))?;
        let mut parts = left.split_whitespace();
        let mode_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing mode in: {}", line))?;
        let _type_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing type in: {}", line))?;
        let sha_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing sha in: {}", line))?;

        let mode_num = u32::from_str_radix(mode_str, 8)?;
        let id: ObjectId = sha_str.parse()?;

        entries.push(TreeEntry {
            mode: FileMode(mode_num),
            name: name.to_string(),
            id,
        });
    }

    let tree = Tree::new(entries);
    let tree_obj = Object::Tree(tree);
    let oid = store.write_object(&tree_obj)?;
    println!("{}", oid);
    Ok(())
}

fn get_head_info(
    git_dir: &Path,
    store: &impl oxidize_core::store::ObjectReader,
) -> Result<(String, Option<ObjectId>, Option<ObjectId>)> {
    let head_path = git_dir.join("HEAD");
    if !head_path.exists() {
        return Ok(("master".to_string(), None, None));
    }
    let head_content = std::fs::read_to_string(head_path)?;
    let head_content = head_content.trim();

    if let Some(rest) = head_content.strip_prefix("ref: refs/heads/") {
        let branch_name = rest.to_string();
        let ref_path = git_dir.join("refs/heads").join(&branch_name);
        if ref_path.exists() {
            let commit_str = std::fs::read_to_string(ref_path)?.trim().to_string();
            if let Ok(commit_oid) = commit_str.parse::<ObjectId>() {
                if let Ok(Object::Commit(commit)) = store.read_object(&commit_oid) {
                    return Ok((branch_name, Some(commit_oid), Some(commit.tree)));
                }
            }
        }
        return Ok((branch_name, None, None));
    }

    if let Ok(commit_oid) = head_content.parse::<ObjectId>() {
        if let Ok(Object::Commit(commit)) = store.read_object(&commit_oid) {
            return Ok(("detached".to_string(), Some(commit_oid), Some(commit.tree)));
        }
    }

    Ok(("master".to_string(), None, None))
}

fn rel_path_from_root(target: &Path, root: &Path) -> Result<PathBuf> {
    if let Ok(rel) = target.strip_prefix(root) {
        return Ok(rel.to_path_buf());
    }
    #[cfg(windows)]
    {
        let canon_target = target
            .canonicalize()
            .map(|p| strip_verbatim_prefix(&p))
            .unwrap_or_else(|_| target.to_path_buf());
        let canon_root = root
            .canonicalize()
            .map(|p| strip_verbatim_prefix(&p))
            .unwrap_or_else(|_| root.to_path_buf());
        if let Ok(rel) = canon_target.strip_prefix(&canon_root) {
            return Ok(rel.to_path_buf());
        }
    }
    bail!("path '{}' is outside repository root", target.display())
}

fn cmd_add(files: Vec<String>) -> Result<()> {
    let ctx = RepoContext::discover(Path::new("."))?;
    let repo_root = ctx
        .worktree
        .as_deref()
        .context("cannot add in bare repository")?;
    let git_dir = &ctx.git_dir;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;
    let gitignore = GitIgnore::load_from_dir(repo_root)?;

    if files.is_empty() {
        eprintln!("Nothing specified, nothing added.\nMaybe you wanted to say 'ox add .'?");
        return Ok(());
    }

    let cur_dir = std::env::current_dir()?;
    let cur_dir = strip_verbatim_prefix(&cur_dir);

    // Track existing index paths for fast lookup
    let tracked_index_paths: BTreeSet<String> =
        index.entries().iter().map(|e| e.path.clone()).collect();
    let mut files_to_stage: Vec<PathBuf> = Vec::new();
    let mut deletions_to_stage: BTreeSet<String> = BTreeSet::new();

    for file_arg in &files {
        let is_dot = file_arg == ".";
        let target_path = if is_dot {
            cur_dir.clone()
        } else {
            let p = Path::new(file_arg);
            if p.is_relative() {
                cur_dir.join(p)
            } else {
                p.to_path_buf()
            }
        };

        // Determine path relative to repo_root
        let rel_target = rel_path_from_root(&target_path, repo_root)?;
        let rel_target_str = rel_target.to_string_lossy().replace('\\', "/");
        let rel_target_str = rel_target_str.trim_matches('/').to_string();

        let is_all = is_dot && rel_target_str.is_empty();
        let target_dir_prefix = if rel_target_str.is_empty() {
            String::new()
        } else {
            format!("{}/", rel_target_str)
        };

        // Check for tracked files matching this target that were deleted from working tree
        for path in &tracked_index_paths {
            let matches = is_all
                || path == &rel_target_str
                || (!target_dir_prefix.is_empty() && path.starts_with(&target_dir_prefix));

            if matches {
                let full = repo_root.join(path);
                if !full.exists() {
                    deletions_to_stage.insert(path.clone());
                }
            }
        }

        if target_path.is_dir() {
            collect_files_to_add(
                repo_root,
                &target_path,
                &gitignore,
                &tracked_index_paths,
                &mut files_to_stage,
            )?;
        } else if target_path.is_file() {
            files_to_stage.push(target_path);
        } else if !deletions_to_stage.iter().any(|d| {
            d == &rel_target_str
                || (!target_dir_prefix.is_empty() && d.starts_with(&target_dir_prefix))
        }) {
            bail!("pathspec '{}' did not match any files", file_arg);
        }
    }

    // Apply staged deletions
    for del_path in &deletions_to_stage {
        index.remove_entry(del_path);
    }

    files_to_stage.sort();
    files_to_stage.dedup();

    // Map existing modes so we preserve permissions (such as 100755) when updating on Windows
    #[cfg(not(unix))]
    let existing_modes: std::collections::HashMap<String, u32> = index
        .entries()
        .iter()
        .map(|e| (e.path.clone(), e.mode))
        .collect();

    // Parallelize reading, hashing, and writing loose objects across threads with Rayon
    use rayon::prelude::*;
    let entries: Vec<Result<IndexEntry>> = files_to_stage
        .par_iter()
        .map(|target| {
            let rel_path = rel_path_from_root(target, repo_root)?
                .to_string_lossy()
                .replace('\\', "/");

            let data = std::fs::read(target)?;
            let blob = Object::Blob(Blob::new(data));
            let oid = store.write_object(&blob)?;

            let meta = std::fs::metadata(target)?;
            #[allow(unused_mut)]
            let mut entry = IndexEntry::from_fs_metadata(rel_path.clone(), oid, &meta, 0);
            #[cfg(not(unix))]
            if let Some(&old_mode) = existing_modes.get(&rel_path) {
                if old_mode == 0o100755 {
                    entry.mode = 0o100755;
                }
            }
            Ok(entry)
        })
        .collect();

    let mut new_entries = Vec::with_capacity(entries.len());
    for entry_res in entries {
        let entry = entry_res?;
        new_entries.push(entry);
    }
    index.add_entries(new_entries);

    index.write_to(&index_path)?;
    Ok(())
}

fn collect_files_to_add(
    repo_root: &Path,
    target: &Path,
    gitignore: &GitIgnore,
    tracked_paths: &BTreeSet<String>,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    if target.is_dir() {
        for entry in std::fs::read_dir(target)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".git" {
                continue;
            }
            if let Ok(rel) = rel_path_from_root(&path, repo_root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let is_dir = path.is_dir();
                let is_tracked = tracked_paths.contains(&rel_str);

                if is_dir {
                    let dir_prefix = format!("{}/", rel_str);
                    let has_tracked = tracked_paths
                        .range(dir_prefix.clone()..)
                        .next()
                        .is_some_and(|p| p.starts_with(&dir_prefix));
                    if gitignore.is_ignored(&rel_str, true) && !has_tracked {
                        continue;
                    }
                } else if !is_tracked && gitignore.is_ignored(&rel_str, false) {
                    continue;
                }
            }
            collect_files_to_add(repo_root, &path, gitignore, tracked_paths, out)?;
        }
    } else if target.is_file() {
        out.push(target.to_path_buf());
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    let ctx = RepoContext::discover(Path::new("."))?;
    let repo_root = ctx
        .worktree
        .as_deref()
        .context("cannot check status in bare repository")?;
    let git_dir = &ctx.git_dir;
    let store = RepoObjectStore::open(git_dir)?;
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;
    let gitignore = GitIgnore::load_from_dir(repo_root)?;

    let (branch_name, head_commit, head_tree) = get_head_info(git_dir, &store)?;
    let status = compute_status_with_ignore(
        repo_root,
        &index,
        head_tree.as_ref(),
        &store,
        Some(&|p, is_dir| gitignore.is_ignored(p, is_dir)),
    )?;

    println!("On branch {}", branch_name);

    if head_commit.is_none() {
        println!("\nNo commits yet\n");
    }

    let mut clean = true;

    if !status.unmerged.is_empty() {
        clean = false;
        println!("Unmerged paths:");
        println!("  (use \"ox add <file>...\" to mark resolution)");
        for path in &status.unmerged {
            println!("\tboth modified:   {}", path);
        }
        println!();
    }

    if !status.staged.is_empty() {
        clean = false;
        println!("Changes to be committed:");
        println!("  (use \"ox restore --staged <file>...\" to unstage)");
        for change in &status.staged {
            match change {
                StagedChange::New(p) => println!("\tnew file:   {}", p),
                StagedChange::Modified(p) => println!("\tmodified:   {}", p),
                StagedChange::Deleted(p) => println!("\tdeleted:    {}", p),
                StagedChange::Renamed { from, to } => println!("\trenamed:    {} -> {}", from, to),
            }
        }
        println!();
    }

    if !status.unstaged.is_empty() {
        clean = false;
        println!("Changes not staged for commit:");
        println!("  (use \"ox add <file>...\" to update what will be committed)");
        for change in &status.unstaged {
            match change {
                UnstagedChange::Modified(p) => println!("\tmodified:   {}", p),
                UnstagedChange::Deleted(p) => println!("\tdeleted:    {}", p),
            }
        }
        println!();
    }

    if !status.untracked.is_empty() {
        clean = false;
        println!("Untracked files:");
        println!("  (use \"ox add <file>...\" to include in what will be committed)");
        for file in &status.untracked {
            println!("\t{}", file);
        }
        println!();
    }

    if clean {
        println!("nothing to commit, working tree clean");
    }

    Ok(())
}

fn cmd_ls_files(stage: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    for entry in index.entries() {
        if stage {
            println!(
                "{:06o} {} {}\t{}",
                entry.mode, entry.oid, entry.stage, entry.path
            );
        } else {
            println!("{}", entry.path);
        }
    }
    Ok(())
}

fn cmd_write_tree() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    let tree_oid = write_tree(&index, &store)?;
    println!("{}", tree_oid);
    Ok(())
}

fn cmd_update_index(
    add: bool,
    chmod: Option<String>,
    index_version: Option<u32>,
    files: Vec<String>,
) -> Result<()> {
    let ctx = RepoContext::discover(Path::new("."))?;
    let repo_root = ctx
        .worktree
        .as_deref()
        .context("cannot update index in bare repository")?;
    let git_dir = &ctx.git_dir;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    if let Some(ver) = index_version {
        if ver != 2 && ver != 3 && ver != 4 {
            bail!("unsupported index version {}", ver);
        }
        index.version = ver;
    }

    let cur_dir = std::env::current_dir()?;
    let cur_dir = strip_verbatim_prefix(&cur_dir);

    for file_str in files {
        if file_str.is_empty() {
            continue;
        }
        let p = Path::new(&file_str);
        let abs = if p.is_relative() {
            cur_dir.join(p)
        } else {
            p.to_path_buf()
        };

        let rel = match abs.strip_prefix(repo_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => bail!("path '{}' is outside repository root", abs.display()),
        };

        if let Some(ref ch) = chmod {
            if let Some(entry) = index.entries.iter_mut().find(|e| e.path == rel) {
                if ch == "+x" {
                    entry.mode = 0o100755;
                } else if ch == "-x" {
                    entry.mode = 0o100644;
                }
            }
        }

        if abs.exists() {
            let data = std::fs::read(&abs)?;
            let blob = Object::Blob(Blob::new(data));
            let oid = store.write_object(&blob)?;
            let meta = std::fs::metadata(&abs)?;
            let mut entry = IndexEntry::from_fs_metadata(rel.clone(), oid, &meta, 0);
            if let Some(ref ch) = chmod {
                if ch == "+x" {
                    entry.mode = 0o100755;
                } else if ch == "-x" {
                    entry.mode = 0o100644;
                }
            }
            index.add_entry(entry);
        } else if !add && chmod.is_none() {
            bail!("cannot find file {}", file_str);
        }
    }

    index.write_to(&index_path)?;
    Ok(())
}

fn cmd_commit(message: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    // 1. Reject unmerged paths
    let mut unmerged = Vec::new();
    for entry in index.entries() {
        if entry.stage != 0 {
            unmerged.push(entry.path.clone());
        }
    }
    if !unmerged.is_empty() {
        unmerged.sort();
        unmerged.dedup();
        bail!(
            "cannot commit: you have unmerged files ({})",
            unmerged.join(", ")
        );
    }

    // 2. Build tree from index
    let tree_oid = write_tree(&index, &store)?;
    let (branch_name, head_commit_oid) = ref_store.resolve_head()?;

    // 3. Check for MERGE_HEAD
    let merge_head_file = git_dir.join("MERGE_HEAD");
    let merge_head_oid = if merge_head_file.exists() {
        let content = std::fs::read_to_string(&merge_head_file)?;
        let hex = content.trim();
        if hex.is_empty() {
            None
        } else {
            Some(hex.parse::<ObjectId>().context("invalid MERGE_HEAD OID")?)
        }
    } else {
        None
    };

    // 4. Check if working tree / index is clean relative to HEAD
    if let Some(ref head_oid) = head_commit_oid {
        if let Ok(Object::Commit(head_commit)) = store.read_object(head_oid) {
            if head_commit.tree == tree_oid && merge_head_oid.is_none() {
                println!("On branch {}", branch_name);
                println!("nothing to commit, working tree clean");
                return Ok(());
            }
        }
    } else if index.entries().is_empty() {
        bail!("nothing to commit (create/copy files and use \"ox add\" to track)");
    }

    // 5. Determine commit message
    let msg_str = if let Some(m) = message {
        m
    } else if let Ok(merge_msg) = std::fs::read_to_string(git_dir.join("MERGE_MSG")) {
        merge_msg
    } else {
        bail!("Aborting commit due to empty commit message.");
    };

    if msg_str.trim().is_empty() {
        bail!("Aborting commit due to empty commit message.");
    }

    let sig = get_default_signature(Some(&git_dir));
    let mut parents = Vec::new();
    if let Some(p) = head_commit_oid {
        parents.push(p);
    }
    if let Some(mp) = merge_head_oid {
        if !parents.contains(&mp) {
            parents.push(mp);
        }
    }

    let commit = Commit {
        tree: tree_oid,
        parents: parents.clone(),
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", msg_str.trim()),
    };

    let commit_oid = store.write_object(&Object::Commit(commit))?;
    let first_line = msg_str.lines().next().unwrap_or("").trim();
    let ref_msg = format!("commit: {}", first_line);
    ref_store.update_ref(
        &branch_name,
        &commit_oid,
        head_commit_oid.as_ref(),
        &ref_msg,
    )?;

    // Clean up merge state files
    remove_file_if_exists(&git_dir.join("MERGE_HEAD"))?;
    remove_file_if_exists(&git_dir.join("MERGE_MSG"))?;
    remove_file_if_exists(&git_dir.join("MERGE_MODE"))?;

    let short_sha = &commit_oid.to_string()[..7];
    if parents.is_empty() {
        println!(
            "[{} (root-commit) {}] {}",
            branch_name, short_sha, first_line
        );
    } else {
        println!("[{} {}] {}", branch_name, short_sha, first_line);
    }

    Ok(())
}

fn cmd_log(max_count: Option<usize>, oneline: bool, graph: bool, tui: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    if tui {
        return Ok(oxidize_tui::run_tui(&git_dir)?);
    }
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let (branch_name, head_oid_opt) = ref_store.resolve_head()?;
    let start_oid = match head_oid_opt {
        Some(oid) => oid,
        None => {
            println!(
                "fatal: your current branch '{}' does not have any commits yet",
                branch_name
            );
            return Ok(());
        }
    };

    let mut curr_oid = Some(start_oid);
    let mut count = 0;
    let limit = max_count.unwrap_or(usize::MAX);

    while let Some(oid) = curr_oid {
        if count >= limit {
            break;
        }

        let obj = store.read_object(&oid)?;
        let commit = match obj {
            Object::Commit(c) => c,
            _ => bail!("object {} is not a commit", oid),
        };

        if oneline {
            let short_sha = &oid.to_string()[..7];
            let first_line = commit.message.lines().next().unwrap_or("");
            if graph {
                println!("* {} {}", short_sha, first_line);
            } else {
                println!("{} {}", short_sha, first_line);
            }
        } else {
            let ts = chrono::DateTime::from_timestamp(commit.author.time_seconds, 0)
                .map(|dt| dt.to_rfc2822())
                .unwrap_or_else(|| commit.author.time_seconds.to_string());

            println!("commit {}", oid);
            println!("Author: {} <{}>", commit.author.name, commit.author.email);
            println!("Date:   {}", ts);
            println!();
            for line in commit.message.lines() {
                println!("    {}", line);
            }
            println!();
        }

        count += 1;
        curr_oid = commit.parents.first().copied();
    }

    Ok(())
}

fn cmd_diff(staged: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    if staged {
        let (_branch, head_oid_opt) = ref_store.resolve_head()?;
        let head_tree_oid = match head_oid_opt {
            Some(oid) => {
                if let Object::Commit(commit) = store.read_object(&oid)? {
                    Some(commit.tree)
                } else {
                    None
                }
            }
            None => None,
        };

        let head_map = if let Some(ref tree_oid) = head_tree_oid {
            flatten_tree(&store, tree_oid, "")?
        } else {
            std::collections::BTreeMap::new()
        };

        // Detect staged renames: match deleted HEAD files with identical OID in new index entries
        // Index deleted HEAD files by OID for O(N + M) matching
        let index_paths: std::collections::HashSet<&str> =
            index.entries().iter().map(|e| e.path.as_str()).collect();
        let mut deleted_by_oid: std::collections::HashMap<&ObjectId, Vec<&str>> =
            std::collections::HashMap::new();
        for (p, (_mode, head_oid)) in &head_map {
            if !index_paths.contains(p.as_str()) {
                deleted_by_oid.entry(head_oid).or_default().push(p.as_str());
            }
        }

        let mut matched_deleted = std::collections::HashSet::new();
        let mut matched_new = std::collections::HashSet::new();

        for (i, entry) in index.entries().iter().enumerate() {
            if !head_map.contains_key(&entry.path) {
                if let Some(candidates) = deleted_by_oid.get_mut(&entry.oid) {
                    if let Some(del_path) = candidates.pop() {
                        matched_deleted.insert(del_path.to_string());
                        matched_new.insert(i);
                        println!("diff --git a/{} b/{}", del_path, entry.path);
                        println!("similarity index 100%");
                        println!("rename from {}", del_path);
                        println!("rename to {}", entry.path);
                    }
                }
            }
        }

        for (i, entry) in index.entries().iter().enumerate() {
            if matched_new.contains(&i) {
                continue;
            }

            let index_blob = match store.read_object(&entry.oid)? {
                Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                _ => String::new(),
            };

            match head_map.get(&entry.path) {
                None => {
                    // New file staged
                    if let Some(diff) =
                        format_unified_diff(&entry.path, &entry.path, "", &index_blob, 3)
                    {
                        print!("{}", diff);
                    }
                }
                Some((_mode, head_oid)) => {
                    if &entry.oid != head_oid {
                        let head_blob = match store.read_object(head_oid)? {
                            Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                            _ => String::new(),
                        };
                        if let Some(diff) = format_unified_diff(
                            &entry.path,
                            &entry.path,
                            &head_blob,
                            &index_blob,
                            3,
                        ) {
                            print!("{}", diff);
                        }
                    }
                }
            }
        }

        // Deleted files
        for (path, (_mode, head_oid)) in &head_map {
            if matched_deleted.contains(path) {
                continue;
            }
            if index.find_entry(path).is_none() {
                let head_blob = match store.read_object(head_oid)? {
                    Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                    _ => String::new(),
                };
                if let Some(diff) = format_unified_diff(path, path, &head_blob, "", 3) {
                    print!("{}", diff);
                }
            }
        }
    } else {
        // Working directory vs index
        for entry in index.entries() {
            let full_path = repo_root.join(&entry.path);
            let index_blob = match store.read_object(&entry.oid)? {
                Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                _ => String::new(),
            };

            if full_path.exists() {
                if let Ok(data) = std::fs::read(&full_path) {
                    let work_content = String::from_utf8_lossy(&data).to_string();
                    if work_content != index_blob {
                        if let Some(diff) = format_unified_diff(
                            &entry.path,
                            &entry.path,
                            &index_blob,
                            &work_content,
                            3,
                        ) {
                            print!("{}", diff);
                        }
                    }
                }
            } else {
                // File deleted in working directory
                if let Some(diff) =
                    format_unified_diff(&entry.path, &entry.path, &index_blob, "", 3)
                {
                    print!("{}", diff);
                }
            }
        }
    }

    Ok(())
}

fn cmd_rev_parse(verify: bool, args: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    if verify && args.len() != 1 {
        bail!("--verify requires exactly one parameter");
    }

    for arg in args {
        let oid = ref_store.resolve_rev(&arg, &store)?;
        println!("{}", oid);
    }
    Ok(())
}

fn cmd_rev_list(commit_str: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let start_oid = ref_store.resolve_rev(&commit_str, &store)?;

    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(start_oid);
    visited.insert(start_oid);

    while let Some(oid) = queue.pop_front() {
        println!("{}", oid);
        if let Ok(Object::Commit(commit)) = store.read_object(&oid) {
            for parent in commit.parents {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }
    }
    Ok(())
}

fn cmd_symbolic_ref(name: String, target: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let ref_path = if name.starts_with("refs/") || name == "HEAD" {
        git_dir.join(&name)
    } else {
        git_dir.join("refs").join(&name)
    };

    if let Some(new_target) = target {
        if !new_target.starts_with("refs/") {
            bail!("Refusing to point symbolic ref to non-ref: {}", new_target);
        }
        if let Some(parent) = ref_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&ref_path, format!("ref: {}\n", new_target))?;
    } else {
        if !ref_path.exists() {
            bail!("fatal: ref {} is not a symbolic ref", name);
        }
        let content = std::fs::read_to_string(&ref_path)?;
        let trimmed = content.trim();
        if let Some(sym) = trimmed.strip_prefix("ref: ") {
            println!("{}", sym);
        } else {
            bail!("fatal: ref {} is not a symbolic ref", name);
        }
    }
    Ok(())
}

fn cmd_update_ref(ref_name: String, new_value: String, old_value: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let new_oid = ref_store.resolve_rev(&new_value, &store)?;
    let old_oid = if let Some(ref old_val) = old_value {
        Some(ref_store.resolve_rev(old_val, &store)?)
    } else {
        None
    };

    ref_store.update_ref(&ref_name, &new_oid, old_oid.as_ref(), "update-ref")?;
    Ok(())
}

fn cmd_show_ref(quiet: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let mut refs = std::collections::BTreeMap::new();

    // 1. Packed refs
    let packed_path = git_dir.join("packed-refs");
    if packed_path.exists() {
        if let Ok(content) = std::fs::read_to_string(packed_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                    continue;
                }
                if let Some((sha, name)) = line.split_once(' ') {
                    refs.insert(name.trim().to_string(), sha.trim().to_string());
                }
            }
        }
    }

    // 2. Loose refs under .git/refs/
    let refs_dir = git_dir.join("refs");
    if refs_dir.is_dir() {
        fn collect_loose_refs(
            dir: &Path,
            prefix: &str,
            refs: &mut std::collections::BTreeMap<String, String>,
        ) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    let ref_name = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", prefix, name)
                    };
                    if path.is_dir() {
                        collect_loose_refs(&path, &ref_name, refs);
                    } else if path.is_file() {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let sha = content.trim().to_string();
                            if sha.len() == 40 {
                                refs.insert(format!("refs/{}", ref_name), sha);
                            }
                        }
                    }
                }
            }
        }
        collect_loose_refs(&refs_dir, "", &mut refs);
    }

    if refs.is_empty() {
        bail!("fatal: no references found");
    }

    if !quiet {
        for (name, sha) in refs {
            println!("{} {}", sha, name);
        }
    }
    Ok(())
}

fn cmd_read_tree(tree_ish: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let oid = ref_store.resolve_rev(&tree_ish, &store)?;
    let obj = store.read_object(&oid)?;
    let tree_oid = match obj {
        Object::Tree(_) => oid,
        Object::Commit(c) => c.tree,
        _ => bail!("fatal: not a tree-ish: {}", tree_ish),
    };

    let flat = flatten_tree(&store, &tree_oid, "")?;
    let mut index = Index::new();
    for (path, (mode, oid)) in flat {
        let full_path = repo_root.join(&path);
        let entry = if let Ok(meta) = std::fs::metadata(&full_path) {
            IndexEntry::from_fs_metadata(path, oid, &meta, 0)
        } else {
            IndexEntry::new(path, oid, mode.0)
        };
        index.add_entry(entry);
    }

    let index_path = git_dir.join("index");
    index.write_to(&index_path)?;
    Ok(())
}

fn cmd_clean(force: bool, directories: bool, dry_run: bool) -> Result<()> {
    if !force && !dry_run {
        bail!("fatal: clean.requireForce defaults to true and neither -i, -n, nor -f given; refusing to clean");
    }

    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    let (_branch, head_oid_opt) = ref_store.resolve_head()?;
    let head_tree_oid = match head_oid_opt {
        Some(oid) => {
            if let Object::Commit(commit) = store.read_object(&oid)? {
                Some(commit.tree)
            } else {
                None
            }
        }
        None => None,
    };

    let gitignore = GitIgnore::load_from_dir(repo_root)?;
    let ignore_fn = |rel: &str, is_dir: bool| gitignore.is_ignored(rel, is_dir);

    let status = compute_status_with_ignore(
        repo_root,
        &index,
        head_tree_oid.as_ref(),
        &store,
        Some(&ignore_fn),
    )?;

    for file in &status.untracked {
        let full_path = repo_root.join(file);
        if full_path.is_file() {
            if dry_run {
                println!("Would remove {}", file);
            } else {
                std::fs::remove_file(&full_path)?;
                println!("Removing {}", file);
            }
        } else if full_path.is_dir() && directories {
            if dry_run {
                println!("Would remove {}/", file);
            } else {
                std::fs::remove_dir_all(&full_path)?;
                println!("Removing {}/", file);
            }
        }
    }

    Ok(())
}

fn cmd_config(
    global: bool,
    list: bool,
    get: Option<String>,
    unset: Option<String>,
    key: Option<String>,
    value: Option<String>,
) -> Result<()> {
    let config_path = if global {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .context("could not determine home directory")?;
        home.join(".gitconfig")
    } else {
        let git_dir = find_git_dir(Path::new("."))?;
        git_dir.join("config")
    };

    let mut config = if config_path.exists() {
        GitConfig::load_from_file(&config_path)?
    } else {
        GitConfig::new()
    };

    if list {
        for (k, v) in config.list_all() {
            println!("{}={}", k, v);
        }
        return Ok(());
    }

    if let Some(ref target_key) = unset {
        if config.unset_by_name(target_key) {
            config.save_to_file(&config_path)?;
        }
        return Ok(());
    }

    if let Some(ref target_key) = get {
        if let Some(val) = config.get_by_name(target_key) {
            println!("{}", val);
            return Ok(());
        } else {
            std::process::exit(1);
        }
    }

    match (key, value) {
        (Some(k), Some(v)) => {
            config.set_by_name(&k, &v);
            config.save_to_file(&config_path)?;
        }
        (Some(k), None) => {
            if let Some(val) = config.get_by_name(&k) {
                println!("{}", val);
            } else {
                std::process::exit(1);
            }
        }
        (None, _) => {
            bail!("fatal: no key specified");
        }
    }

    Ok(())
}

fn cmd_show(object_ref: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let target = object_ref.unwrap_or_else(|| "HEAD".to_string());
    let oid = ref_store.resolve_rev(&target, &store)?;
    let obj = store.read_object(&oid)?;

    match obj {
        Object::Commit(commit) => {
            let ts = chrono::DateTime::from_timestamp(commit.author.time_seconds, 0)
                .map(|dt| dt.to_rfc2822())
                .unwrap_or_else(|| commit.author.time_seconds.to_string());

            println!("commit {}", oid);
            if commit.parents.len() > 1 {
                print!("Merge:");
                for p in &commit.parents {
                    print!(" {}", &p.to_string()[..7]);
                }
                println!();
            }
            println!("Author: {} <{}>", commit.author.name, commit.author.email);
            println!("Date:   {}", ts);
            println!();
            for line in commit.message.lines() {
                println!("    {}", line);
            }
            println!();

            let parent_map = if let Some(parent_oid) = commit.parents.first() {
                if let Ok(Object::Commit(p_commit)) = store.read_object(parent_oid) {
                    flatten_tree(&store, &p_commit.tree, "")?
                } else {
                    std::collections::BTreeMap::new()
                }
            } else {
                std::collections::BTreeMap::new()
            };

            let current_map = flatten_tree(&store, &commit.tree, "")?;

            for (path, (_mode, curr_blob_oid)) in &current_map {
                let curr_data = match store.read_object(curr_blob_oid)? {
                    Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                    _ => String::new(),
                };

                if let Some((_p_mode, p_blob_oid)) = parent_map.get(path) {
                    if p_blob_oid != curr_blob_oid {
                        let p_data = match store.read_object(p_blob_oid)? {
                            Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                            _ => String::new(),
                        };
                        if let Some(diff) = format_unified_diff(path, path, &p_data, &curr_data, 3)
                        {
                            print!("{}", diff);
                        }
                    }
                } else {
                    if let Some(diff) = format_unified_diff(path, path, "", &curr_data, 3) {
                        print!("{}", diff);
                    }
                }
            }

            for (path, (_p_mode, p_blob_oid)) in &parent_map {
                if !current_map.contains_key(path) {
                    let p_data = match store.read_object(p_blob_oid)? {
                        Object::Blob(b) => String::from_utf8_lossy(&b.data).to_string(),
                        _ => String::new(),
                    };
                    if let Some(diff) = format_unified_diff(path, path, &p_data, "", 3) {
                        print!("{}", diff);
                    }
                }
            }
        }
        Object::Blob(blob) => {
            io::stdout().write_all(&blob.data)?;
        }
        Object::Tree(tree) => {
            for entry in tree.entries {
                println!(
                    "{} {} {}\t{}",
                    entry.mode.display_str(),
                    entry.mode.object_type().as_str(),
                    entry.id,
                    entry.name
                );
            }
        }
        Object::Tag(_) => {
            io::stdout().write_all(&obj.serialize_content())?;
        }
    }
    Ok(())
}

fn cmd_merge_base(commit1: String, commit2: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let oid1 = ref_store.resolve_rev(&commit1, &store)?;
    let oid2 = ref_store.resolve_rev(&commit2, &store)?;

    if let Some(base) = ref_store.find_merge_base(&store, &oid1, &oid2)? {
        println!("{}", base);
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn cmd_commit_tree(tree: String, parents: Vec<String>, message: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let tree_oid: ObjectId = tree.parse()?;

    let mut parent_oids = Vec::new();
    for p in parents {
        parent_oids.push(p.parse::<ObjectId>()?);
    }

    let sig = get_default_signature(Some(&git_dir));
    let commit = Commit {
        tree: tree_oid,
        parents: parent_oids,
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: format!("{}\n", message.trim()),
    };

    let commit_oid = store.write_object(&Object::Commit(commit))?;
    println!("{}", commit_oid);
    Ok(())
}

fn checkout_tree_and_update_index(
    repo_root: &Path,
    store: &impl oxidize_core::ObjectReader,
    index: &mut Index,
    head_tree_oid: Option<&ObjectId>,
    target_tree_oid: &ObjectId,
    force: bool,
) -> Result<()> {
    let target_map = flatten_tree(store, target_tree_oid, "")?;

    // Preflight: validate every target path and index path before mutating anything
    for path in target_map.keys() {
        oxidize_core::safe_join(repo_root, path)?;
    }
    for entry in index.entries() {
        oxidize_core::safe_join(repo_root, &entry.path)?;
    }

    let head_map = match head_tree_oid {
        Some(oid) => flatten_tree(store, oid, "")?,
        None => BTreeMap::new(),
    };

    if !force {
        let mut dirty_paths = Vec::new();
        let mut untracked_collisions = Vec::new();

        // Collect all paths touched between HEAD and target
        let mut touched_paths = BTreeSet::new();
        for (p, target_val) in &target_map {
            if head_map.get(p) != Some(target_val) {
                touched_paths.insert(p.clone());
            }
        }
        for p in head_map.keys() {
            if !target_map.contains_key(p) {
                touched_paths.insert(p.clone());
            }
        }

        for path in &touched_paths {
            let full_path = oxidize_core::safe_join(repo_root, path)?;
            if let Some(entry) = index.get_entry(path) {
                if full_path.is_file() {
                    let data = std::fs::read(&full_path)?;
                    let wt_oid = ObjectId::hash_blob(&data);
                    if wt_oid != entry.oid {
                        dirty_paths.push(path.clone());
                        continue;
                    }
                } else if !full_path.exists() && entry.stage == 0 {
                    dirty_paths.push(path.clone());
                    continue;
                }

                // Check staged changes vs HEAD
                let head_val = head_map.get(path);
                if head_val.map(|(_, oid)| *oid) != Some(entry.oid) {
                    dirty_paths.push(path.clone());
                }
            } else if full_path.exists() {
                // Untracked collision
                untracked_collisions.push(path.clone());
            }
        }

        if !dirty_paths.is_empty() {
            bail!(
                "error: Your local changes to the following files would be overwritten by checkout:\n\t{}\nPlease commit your changes or stash them before you switch branches.\nAborting",
                dirty_paths.join("\n\t")
            );
        }

        if !untracked_collisions.is_empty() {
            bail!(
                "error: The following untracked working tree files would be overwritten by checkout:\n\t{}\nPlease move or remove them before you switch branches.\nAborting",
                untracked_collisions.join("\n\t")
            );
        }

        // Apply changes: only update paths that changed between HEAD and target
        for path in &touched_paths {
            let full_path = oxidize_core::safe_join(repo_root, path)?;
            if let Some((_mode, oid)) = target_map.get(path) {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let blob = match store.read_object(oid)? {
                    Object::Blob(blob) => blob,
                    _ => bail!("checkout expected blob for '{}'", path),
                };
                std::fs::write(&full_path, &blob.data)?;
                let meta = std::fs::metadata(&full_path)?;
                let entry = IndexEntry::from_fs_metadata(path.clone(), *oid, &meta, 0);
                index.add_entry(entry);
            } else {
                // Removed in target
                if full_path.exists() {
                    std::fs::remove_file(&full_path)?;
                }
                index.remove_entry(path);
                let mut parent = full_path.parent();
                while let Some(p) = parent {
                    if p == repo_root || !p.starts_with(repo_root) {
                        break;
                    }
                    if std::fs::remove_dir(p).is_err() {
                        break;
                    }
                    parent = p.parent();
                }
            }
        }
    } else {
        // Force checkout (e.g. clone or reset --hard)
        // 1. Remove files from working tree that are in old index but not in new target tree
        for entry in index.entries() {
            if !target_map.contains_key(&entry.path) {
                let full_path = oxidize_core::safe_join(repo_root, &entry.path)?;
                if full_path.exists() {
                    std::fs::remove_file(&full_path)?;
                }
            }
        }

        // 2. Write new files to working tree and create new index entries
        index.entries.clear();
        for (path, (_mode, oid)) in target_map {
            let full_path = oxidize_core::safe_join(repo_root, &path)?;
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let blob = match store.read_object(&oid)? {
                Object::Blob(blob) => blob,
                _ => bail!("checkout expected blob for '{}'", path),
            };
            std::fs::write(&full_path, &blob.data)?;
            let meta = std::fs::metadata(&full_path)?;
            let entry = IndexEntry::from_fs_metadata(path, oid, &meta, 0);
            index.add_entry(entry);
        }
    }

    Ok(())
}

fn cmd_branch(delete: bool, force_delete: bool, _all: bool, name: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let ref_store = RefStore::new(&git_dir);

    if delete || force_delete {
        let branch_name = name.context("branch name required to delete")?;
        let (active_branch, head_oid_opt) = ref_store.resolve_head()?;
        if active_branch == branch_name {
            bail!("cannot delete branch '{}' checked out", branch_name);
        }
        if !force_delete {
            let branch_ref = format!("refs/heads/{}", branch_name);
            let branch_oid = ref_store.read_ref(&branch_ref)?;
            let store = RepoObjectStore::open(&git_dir)?;
            let head_oid =
                head_oid_opt.context("cannot verify branch merge status: HEAD has no commits")?;
            if !is_ancestor(&store, &branch_oid, &head_oid)? {
                bail!("The branch '{}' is not fully merged.\nIf you are sure you want to delete it, run 'ox branch -D {}'.", branch_name, branch_name);
            }
        }
        ref_store.delete_branch(&branch_name)?;
        println!("Deleted branch {}.", branch_name);
        return Ok(());
    }

    if let Some(branch_name) = name {
        let (_, head_oid) = ref_store.resolve_head()?;
        let target_oid = head_oid.context("cannot create branch: HEAD has no commits")?;
        ref_store.create_branch(&branch_name, &target_oid)?;
        return Ok(());
    }

    // List branches
    let branches = ref_store.list_branches()?;
    let (active_branch, _) = ref_store.resolve_head()?;

    for (b_name, _) in branches {
        if b_name == active_branch {
            println!("* {}", b_name);
        } else {
            println!("  {}", b_name);
        }
    }

    Ok(())
}

fn cmd_checkout(create_branch: Option<String>, target: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    if let Some(new_branch) = create_branch {
        let (_, head_oid) = ref_store.resolve_head()?;
        let target_oid = head_oid.context("cannot checkout new branch: HEAD has no commits")?;
        ref_store.create_branch(&new_branch, &target_oid)?;
        ref_store.set_head_symbolic(&new_branch)?;
        println!("Switched to a new branch '{}'", new_branch);
        return Ok(());
    }

    let target = target.context("branch or commit target required")?;

    let head_tree_oid = match ref_store.resolve_head()?.1 {
        Some(head_oid) => {
            if let Ok(Object::Commit(c)) = store.read_object(&head_oid) {
                Some(c.tree)
            } else {
                None
            }
        }
        None => None,
    };

    // Check if target is a branch name
    let branch_ref = format!("refs/heads/{}", target);
    if let Ok(commit_oid) = ref_store.read_ref(&branch_ref) {
        let obj = store.read_object(&commit_oid)?;
        let commit = match obj {
            Object::Commit(c) => c,
            _ => bail!("object {} is not a commit", commit_oid),
        };

        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            head_tree_oid.as_ref(),
            &commit.tree,
            false,
        )?;
        index.write_to(&index_path)?;
        ref_store.set_head_symbolic(&target)?;
        println!("Switched to branch '{}'", target);
        return Ok(());
    }

    // Otherwise try resolving revision as commit
    if let Ok(commit_oid) = ref_store.resolve_rev(&target, &store) {
        let obj = store.read_object(&commit_oid)?;
        let commit = match obj {
            Object::Commit(c) => c,
            _ => bail!("object {} is not a commit", commit_oid),
        };

        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            head_tree_oid.as_ref(),
            &commit.tree,
            false,
        )?;
        index.write_to(&index_path)?;
        ref_store.set_head_detached(&commit_oid)?;
        println!(
            "Note: switching to '{}' (detached HEAD)",
            &commit_oid.to_string()[..7]
        );
        return Ok(());
    }

    bail!(
        "pathspec or branch '{}' did not match any file(s) known to git",
        target
    );
}

fn cmd_switch(create: Option<String>, branch: Option<String>) -> Result<()> {
    if let Some(new_branch) = create {
        cmd_checkout(Some(new_branch), branch)
    } else {
        cmd_checkout(None, branch)
    }
}

fn cmd_merge(abort: bool, commit_opt: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    if abort {
        let merge_head_file = git_dir.join("MERGE_HEAD");
        if !merge_head_file.exists() {
            bail!("fatal: There is no merge to abort (MERGE_HEAD missing).");
        }
        let (_, our_oid_opt) = ref_store.resolve_head()?;
        let our_oid = our_oid_opt.context("cannot abort merge: HEAD has no commits")?;
        let our_commit = match store.read_object(&our_oid)? {
            Object::Commit(c) => c,
            _ => bail!("HEAD commit not found"),
        };
        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            None,
            &our_commit.tree,
            true,
        )?;
        index.write_to(&index_path)?;
        remove_file_if_exists(&git_dir.join("MERGE_HEAD"))?;
        remove_file_if_exists(&git_dir.join("MERGE_MSG"))?;
        remove_file_if_exists(&git_dir.join("MERGE_MODE"))?;
        println!("Merge aborted.");
        return Ok(());
    }

    let commit_arg = commit_opt.context("commit or branch name required to merge")?;

    let (our_branch, our_oid_opt) = ref_store.resolve_head()?;
    let our_oid = our_oid_opt.context("cannot merge: HEAD has no commits")?;
    let their_oid = ref_store.resolve_rev(&commit_arg, &store)?;

    let merge_base_opt = ref_store.find_merge_base(&store, &our_oid, &their_oid)?;
    let merge_base = match merge_base_opt {
        Some(base) => base,
        None => bail!("refusing to merge unrelated histories"),
    };

    if merge_base == their_oid {
        println!("Already up to date.");
        return Ok(());
    }

    let our_commit = match store.read_object(&our_oid)? {
        Object::Commit(c) => c,
        _ => bail!("our commit not found"),
    };
    let their_commit = match store.read_object(&their_oid)? {
        Object::Commit(c) => c,
        _ => bail!("their commit not found"),
    };

    if merge_base == our_oid {
        // Fast-forward merge!
        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            Some(&our_commit.tree),
            &their_commit.tree,
            false,
        )?;
        index.write_to(&index_path)?;
        ref_store.update_ref(
            &our_branch,
            &their_oid,
            Some(&our_oid),
            &format!("merge {}: Fast-forward", commit_arg),
        )?;
        println!(
            "Updating {}..{}",
            &our_oid.to_string()[..7],
            &their_oid.to_string()[..7]
        );
        println!("Fast-forward");
        return Ok(());
    }

    // 3-way merge
    let base_commit = match store.read_object(&merge_base)? {
        Object::Commit(c) => c,
        _ => bail!("base commit not found"),
    };

    let base_files = flatten_tree(&store, &base_commit.tree, "")?;
    let our_files = flatten_tree(&store, &our_commit.tree, "")?;
    let their_files = flatten_tree(&store, &their_commit.tree, "")?;

    let mut all_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for p in base_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in our_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in their_files.keys() {
        all_paths.insert(p.clone());
    }

    let mut had_conflicts = false;
    let mut conflict_paths = Vec::new();

    for path in all_paths {
        let base_entry = base_files.get(&path).copied();
        let our_entry = our_files.get(&path).copied();
        let their_entry = their_files.get(&path).copied();

        // 1. Identical on both sides
        if our_entry == their_entry {
            continue;
        }

        // 2. Changed only in theirs
        if base_entry == our_entry {
            let full_path = repo_root.join(&path);
            if let Some((_their_mode, their_oid)) = their_entry {
                let (_, data) = store.read_raw(&their_oid)?;
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&full_path, &data)?;
                let meta = std::fs::metadata(&full_path)?;
                index.add_entry(IndexEntry::from_fs_metadata(
                    path.clone(),
                    their_oid,
                    &meta,
                    0,
                ));
            } else {
                if full_path.exists() {
                    std::fs::remove_file(&full_path)?;
                }
                index.remove_entry(&path);
            }
            continue;
        }

        // 3. Changed only in ours
        if base_entry == their_entry {
            continue;
        }

        // 4. Both changed!
        let full_path = repo_root.join(&path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Check modify/delete conflict
        if our_entry.is_none() || their_entry.is_none() {
            had_conflicts = true;
            conflict_paths.push(path.clone());
            println!("CONFLICT (modify/delete): Merge conflict in {}", path);

            index.remove_entry(&path);
            if let Some((mode, oid)) = base_entry {
                let mut e = IndexEntry::new(path.clone(), oid, mode.0);
                e.stage = 1;
                index.add_entry(e);
            }
            if let Some((mode, oid)) = our_entry {
                let mut e = IndexEntry::new(path.clone(), oid, mode.0);
                e.stage = 2;
                index.add_entry(e);
            }
            if let Some((mode, oid)) = their_entry {
                let mut e = IndexEntry::new(path.clone(), oid, mode.0);
                e.stage = 3;
                index.add_entry(e);
            }
            continue;
        }

        let (our_mode, our_oid) = our_entry.unwrap();
        let (their_mode, their_oid) = their_entry.unwrap();

        let our_bytes = store.read_raw(&our_oid)?.1;
        let their_bytes = store.read_raw(&their_oid)?.1;
        let base_bytes = if let Some((_, base_oid)) = base_entry {
            store.read_raw(&base_oid)?.1
        } else {
            Vec::new()
        };

        // Binary check
        let is_bin = oxidize_diff::is_binary_content(&our_bytes)
            || oxidize_diff::is_binary_content(&their_bytes)
            || oxidize_diff::is_binary_content(&base_bytes);

        let our_str_res = std::str::from_utf8(&our_bytes);
        let their_str_res = std::str::from_utf8(&their_bytes);
        let base_str_res = std::str::from_utf8(&base_bytes);

        if is_bin || our_str_res.is_err() || their_str_res.is_err() || base_str_res.is_err() {
            had_conflicts = true;
            conflict_paths.push(path.clone());
            println!("CONFLICT (content): Merge conflict in {}", path);
            println!("warning: Cannot merge binary files: {}", path);

            // Write our version to worktree to avoid corrupting binary data
            std::fs::write(&full_path, &our_bytes)?;

            index.remove_entry(&path);
            if let Some((mode, oid)) = base_entry {
                let mut e = IndexEntry::new(path.clone(), oid, mode.0);
                e.stage = 1;
                index.add_entry(e);
            }
            let mut e2 = IndexEntry::new(path.clone(), our_oid, our_mode.0);
            e2.stage = 2;
            index.add_entry(e2);
            let mut e3 = IndexEntry::new(path.clone(), their_oid, their_mode.0);
            e3.stage = 3;
            index.add_entry(e3);
            continue;
        }

        let our_str = our_str_res.unwrap();
        let their_str = their_str_res.unwrap();
        let base_str = base_str_res.unwrap();

        let merged = three_way_merge(base_str, our_str, their_str, "HEAD", &commit_arg);
        std::fs::write(&full_path, merged.content.as_bytes())?;

        if merged.has_conflicts {
            had_conflicts = true;
            conflict_paths.push(path.clone());
            println!("CONFLICT (content): Merge conflict in {}", path);

            index.remove_entry(&path);
            if let Some((mode, oid)) = base_entry {
                let mut e = IndexEntry::new(path.clone(), oid, mode.0);
                e.stage = 1;
                index.add_entry(e);
            }
            let mut e2 = IndexEntry::new(path.clone(), our_oid, our_mode.0);
            e2.stage = 2;
            index.add_entry(e2);
            let mut e3 = IndexEntry::new(path.clone(), their_oid, their_mode.0);
            e3.stage = 3;
            index.add_entry(e3);
        } else {
            let blob = Object::Blob(Blob::new(merged.content.into_bytes()));
            let blob_oid = store.loose().write_object(&blob)?;
            let meta = std::fs::metadata(&full_path)?;
            index.add_entry(IndexEntry::from_fs_metadata(
                path.clone(),
                blob_oid,
                &meta,
                0,
            ));
        }
    }

    index.write_to(&index_path)?;

    if had_conflicts {
        std::fs::write(git_dir.join("MERGE_HEAD"), format!("{}\n", their_oid))?;
        let msg = format!(
            "Merge branch '{}'\n\n# Conflicts:\n#\t{}\n",
            commit_arg,
            conflict_paths.join("\n#\t")
        );
        std::fs::write(git_dir.join("MERGE_MSG"), msg)?;
        println!("Automatic merge failed; fix conflicts and then commit the result.");
        bail!("Automatic merge failed; fix conflicts and then commit the result.");
    } else {
        // Automatic merge commit
        let tree_oid = write_tree(&index, store.loose())?;
        let sig = get_default_signature(Some(&git_dir));
        let merge_commit = Commit {
            tree: tree_oid,
            parents: vec![our_oid, their_oid],
            author: sig.clone(),
            committer: sig,
            gpg_sig: None,
            message: format!("Merge branch '{}'\n", commit_arg),
        };
        let merge_oid = store.loose().write_object(&Object::Commit(merge_commit))?;
        ref_store.update_ref(
            &our_branch,
            &merge_oid,
            Some(&our_oid),
            &format!("merge {}", commit_arg),
        )?;
        println!("Merge made by the 'ort' strategy.");
    }

    Ok(())
}

fn get_blob_text(store: &impl oxidize_core::ObjectReader, oid_opt: Option<&ObjectId>) -> String {
    if let Some(oid) = oid_opt {
        if let Ok(Object::Blob(b)) = store.read_object(oid) {
            return String::from_utf8_lossy(&b.data).to_string();
        }
    }
    String::new()
}

fn cmd_reset(hard: bool, soft: bool, _mixed: bool, commit_arg: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let target_str = commit_arg.unwrap_or_else(|| "HEAD".to_string());
    let target_oid = ref_store.resolve_rev(&target_str, &store)?;
    let (active_branch, _) = ref_store.resolve_head()?;

    let target_commit = match store.read_object(&target_oid)? {
        Object::Commit(c) => c,
        _ => bail!("target is not a commit"),
    };

    if soft {
        ref_store.update_ref(
            &active_branch,
            &target_oid,
            None,
            &format!("reset: moving to {}", target_str),
        )?;
        return Ok(());
    }

    if hard {
        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            None,
            &target_commit.tree,
            true,
        )?;
        index.write_to(&index_path)?;
        ref_store.update_ref(
            &active_branch,
            &target_oid,
            None,
            &format!("reset: moving to {}", target_str),
        )?;
        println!(
            "HEAD is now at {} {}",
            &target_oid.to_string()[..7],
            target_commit.message.lines().next().unwrap_or("")
        );
        return Ok(());
    }

    // Default mixed: reset index to target commit tree
    let target_map = flatten_tree(&store, &target_commit.tree, "")?;
    index.entries.clear();
    for (path, (mode, oid)) in target_map {
        let full_path = repo_root.join(&path);
        let meta = std::fs::metadata(&full_path).ok();
        let file_size = meta.as_ref().map(|m| m.len() as u32).unwrap_or(0);
        index.add_entry(IndexEntry {
            ctime_sec: 0,
            ctime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode: mode.0,
            uid: 0,
            gid: 0,
            file_size,
            oid,
            stage: 0,
            assume_valid: false,
            path,
        });
    }
    index.write_to(&index_path)?;
    ref_store.update_ref(
        &active_branch,
        &target_oid,
        None,
        &format!("reset: moving to {}", target_str),
    )?;

    Ok(())
}

fn cmd_pack_objects(base_name: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;

    let mut oids = Vec::new();
    if !io::stdin().is_terminal() {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.len() >= 40 {
                if let Ok(oid) = trimmed[..40].parse::<ObjectId>() {
                    oids.push(oid);
                }
            }
        }
    }

    let raw_objects = if oids.is_empty() {
        store.collect_all_objects()?
    } else {
        let mut objs = Vec::new();
        for oid in oids {
            let (obj_type, data) = store.read_raw(&oid)?;
            objs.push(RawPackObject::new(oid, obj_type, data));
        }
        objs
    };

    let (pack_bytes, indexed_objs, pack_checksum) = write_pack(&raw_objects, true)?;

    let pack_path = format!("{}-{}.pack", base_name, pack_checksum);
    let idx_path = format!("{}-{}.idx", base_name, pack_checksum);

    if let Some(parent) = Path::new(&pack_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    std::fs::write(&pack_path, &pack_bytes)?;
    PackIndex::write_to(indexed_objs, &pack_checksum, &idx_path)?;

    println!("{}", pack_checksum);
    Ok(())
}

fn cmd_unpack_objects() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = LooseObjectStore::new(git_dir.join("objects"));

    let mut pack_bytes = Vec::new();
    io::stdin().read_to_end(&mut pack_bytes)?;

    if pack_bytes.is_empty() {
        bail!("empty input for unpack-objects");
    }

    let unpacked = unpack_packfile(&pack_bytes)?;
    for (_oid, obj_type, data) in unpacked {
        let obj = parse_object_from_content(obj_type, &data)?;
        store.write_object(&obj)?;
    }

    Ok(())
}

fn cmd_index_pack(file: String) -> Result<()> {
    let pack_bytes =
        std::fs::read(&file).with_context(|| format!("failed to read packfile '{}'", file))?;

    let (indexed_objects, pack_checksum) = index_packfile(&pack_bytes)?;

    let idx_path = if file.ends_with(".pack") {
        format!("{}.idx", &file[..file.len() - 5])
    } else {
        format!("{}.idx", file)
    };

    PackIndex::write_to(indexed_objects, &pack_checksum, &idx_path)?;
    println!("{}", pack_checksum);
    Ok(())
}

fn cmd_verify_pack(verbose: bool, files: Vec<String>) -> Result<()> {
    for file in files {
        let (idx_path, pack_path) = if file.ends_with(".idx") {
            let p = format!("{}.pack", &file[..file.len() - 4]);
            (file.clone(), p)
        } else if file.ends_with(".pack") {
            let idx = format!("{}.idx", &file[..file.len() - 5]);
            (idx, file.clone())
        } else {
            (format!("{}.idx", file), format!("{}.pack", file))
        };

        let index = PackIndex::read_from(&idx_path)
            .with_context(|| format!("failed to read index file '{}'", idx_path))?;
        let pack_bytes = std::fs::read(&pack_path)
            .with_context(|| format!("failed to read packfile '{}'", pack_path))?;

        if pack_bytes.len() < 32 {
            bail!("packfile '{}' is too short", pack_path);
        }

        // 1. Verify pack checksum
        let payload_len = pack_bytes.len() - 20;
        let mut hasher = Sha1::new();
        hasher.update(&pack_bytes[..payload_len]);
        let calculated_hash: [u8; 20] = hasher.finalize().into();
        let pack_checksum = ObjectId::from_bytes(calculated_hash);

        if pack_checksum != index.pack_checksum {
            bail!("pack checksum mismatch in index '{}'", idx_path);
        }
        if calculated_hash != pack_bytes[payload_len..] {
            bail!("pack checksum corruption in packfile '{}'", pack_path);
        }

        // 2. Verify each object's CRC32 and unpack
        for item in &index.objects {
            let (obj_type, data, packed_len, crc32) =
                read_pack_object_at(&pack_bytes, item.offset, None)?;

            if crc32 != item.crc32 {
                bail!(
                    "CRC32 mismatch for object {}: index has {:08x}, pack has {:08x}",
                    item.oid,
                    item.crc32,
                    crc32
                );
            }

            // Verify SHA-1
            let mut h = Sha1::new();
            h.update(format!("{} {}\0", obj_type.as_str(), data.len()).as_bytes());
            h.update(&data);
            let calculated_oid = ObjectId::from_bytes(h.finalize().into());
            if calculated_oid != item.oid {
                bail!(
                    "SHA-1 mismatch for object at offset {}: expected {}, computed {}",
                    item.offset,
                    item.oid,
                    calculated_oid
                );
            }

            if verbose {
                println!(
                    "{} {:<6} {:>7} {:>7} {}",
                    item.oid,
                    obj_type.as_str(),
                    data.len(),
                    packed_len,
                    item.offset
                );
            }
        }

        println!("{}: OK", idx_path);
    }
    Ok(())
}

fn cmd_gc() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let loose = LooseObjectStore::new(git_dir.join("objects"));

    // Collect all loose objects
    let mut loose_objects = Vec::new();
    let mut files_to_prune = Vec::new();

    if loose.root().is_dir() {
        for entry in std::fs::read_dir(loose.root())? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                if dir_name.len() == 2 && dir_name != "in" && dir_name != "pa" {
                    for sub in std::fs::read_dir(&path)? {
                        let sub = sub?;
                        let file_path = sub.path();
                        let file_name = sub.file_name().to_string_lossy().to_string();
                        let full_hex = format!("{}{}", dir_name, file_name);
                        if let Ok(oid) = full_hex.parse::<ObjectId>() {
                            if let Ok((obj_type, data)) = loose.read_raw(&oid) {
                                loose_objects.push(RawPackObject::new(oid, obj_type, data));
                                files_to_prune.push(file_path);
                            }
                        }
                    }
                }
            }
        }
    }

    if loose_objects.is_empty() {
        return Ok(());
    }

    let pack_dir = git_dir.join("objects").join("pack");
    std::fs::create_dir_all(&pack_dir)?;

    let (pack_bytes, indexed_objs, pack_checksum) = write_pack(&loose_objects, true)?;

    let pack_path = pack_dir.join(format!("pack-{}.pack", pack_checksum));
    let idx_path = pack_dir.join(format!("pack-{}.idx", pack_checksum));

    std::fs::write(&pack_path, &pack_bytes)?;
    PackIndex::write_to(indexed_objs, &pack_checksum, &idx_path)?;

    // Prune packed loose objects
    for file in files_to_prune {
        std::fs::remove_file(&file)
            .with_context(|| format!("failed to prune packed loose object '{}'", file.display()))?;
    }

    // Clean up empty directories
    if loose.root().is_dir() {
        for entry in (std::fs::read_dir(loose.root())?).flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.len() == 2 {
                    match std::fs::remove_dir(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
    }

    println!(
        "Packed {} objects into pack-{}.",
        loose_objects.len(),
        pack_checksum
    );
    Ok(())
}

fn cmd_fsck() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let all_objects = store.collect_all_objects()?;
    let mut reachable: std::collections::HashSet<ObjectId> = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<ObjectId> = std::collections::VecDeque::new();

    // 1. Gather roots from refs
    if let Ok((_head_ref, Some(head_oid))) = ref_store.resolve_head() {
        queue.push_back(head_oid);
        reachable.insert(head_oid);
    }
    if let Ok(branches) = ref_store.list_branches() {
        for (_branch, oid) in branches {
            if reachable.insert(oid) {
                queue.push_back(oid);
            }
        }
    }
    // Tags
    let tags_dir = git_dir.join("refs").join("tags");
    if tags_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(tags_dir) {
            for entry in entries.flatten() {
                if let Ok(oid_str) = std::fs::read_to_string(entry.path()) {
                    if let Ok(oid) = oid_str.trim().parse::<ObjectId>() {
                        if reachable.insert(oid) {
                            queue.push_back(oid);
                        }
                    }
                }
            }
        }
    }
    // Index
    let index_file = git_dir.join("index");
    if index_file.exists() {
        if let Ok(index) = Index::load_from(&index_file) {
            for entry in &index.entries {
                if reachable.insert(entry.oid) {
                    queue.push_back(entry.oid);
                }
            }
        }
    }

    // 2. BFS graph traversal to mark reachable objects
    while let Some(oid) = queue.pop_front() {
        if let Ok(obj) = store.read_object(&oid) {
            match obj {
                Object::Commit(commit) => {
                    if reachable.insert(commit.tree) {
                        queue.push_back(commit.tree);
                    }
                    for parent in commit.parents {
                        if reachable.insert(parent) {
                            queue.push_back(parent);
                        }
                    }
                }
                Object::Tree(tree) => {
                    for entry in tree.entries {
                        if reachable.insert(entry.id) {
                            queue.push_back(entry.id);
                        }
                    }
                }
                Object::Tag(tag) => {
                    if reachable.insert(tag.target) {
                        queue.push_back(tag.target);
                    }
                }
                Object::Blob(_) => {}
            }
        } else {
            eprintln!("missing object {}", oid);
        }
    }

    // 3. Find dangling objects
    let mut dangling = Vec::new();
    for obj in &all_objects {
        if !reachable.contains(&obj.oid) {
            dangling.push((obj.obj_type, obj.oid));
        }
    }
    dangling.sort_by_key(|d| d.1);

    for (t, oid) in dangling {
        println!("dangling {} {}", t.as_str(), oid);
    }

    Ok(())
}

fn cmd_clone(repository: String, directory: Option<String>) -> Result<()> {
    let target_dir_str = if let Some(dir) = directory {
        dir
    } else {
        let trimmed = repository.trim_end_matches('/').trim_end_matches(".git");
        let name = trimmed.rsplit(['/', '\\']).next().unwrap_or("repo");
        name.to_string()
    };

    let target_path = PathBuf::from(&target_dir_str);
    if target_path.exists() && target_path.read_dir()?.next().is_some() {
        bail!(
            "destination path '{}' already exists and is not an empty directory.",
            target_dir_str
        );
    }

    println!("Cloning into '{}'...", target_dir_str);
    std::fs::create_dir_all(&target_path)?;

    // 1. Initialize empty Git repository in target_path
    let git_dir = target_path.join(".git");
    cmd_init(Some(target_dir_str.clone()))?;

    // 2. Discover remote refs & fetch pack
    let (remote_refs, default_branch, pack_bytes) =
        if let Some(local_path) = resolve_local_path(&repository) {
            let (refs, def_branch) = discover_local_refs(&local_path)?;
            let wants: Vec<ObjectId> = refs.iter().map(|r| r.oid).collect();
            let pack = if wants.is_empty() {
                Vec::new()
            } else {
                fetch_local_pack(&local_path, &wants)?
            };
            (refs, def_branch, pack)
        } else if is_ssh_url(&repository) {
            let client = SshClient::new();
            let (refs, _caps, symref_head) = client.discover_upload_pack(&repository)?;
            let wants: Vec<ObjectId> = refs.iter().map(|r| r.oid).collect();
            let pack = if wants.is_empty() {
                Vec::new()
            } else {
                let (p, _progress) = client.fetch_pack(&repository, &wants, &[])?;
                p
            };
            (refs, symref_head, pack)
        } else {
            let client = SmartHttpClient::new();
            let (refs, _caps, symref_head) = client.discover_upload_pack(&repository)?;
            let wants: Vec<ObjectId> = refs.iter().map(|r| r.oid).collect();
            let pack = if wants.is_empty() {
                Vec::new()
            } else {
                let (p, _progress) = client.fetch_pack(&repository, &wants, &[])?;
                p
            };
            (refs, symref_head, pack)
        };

    // Configure remote in .git/config
    let config_path = git_dir.join("config");
    let mut config = GitConfig::load_from_file(&config_path)?;
    config.add_remote("origin", &repository);

    if remote_refs.is_empty() || pack_bytes.is_empty() {
        config.save_to_file(&config_path)?;
        println!("warning: You appear to have cloned an empty repository.");
        return Ok(());
    }

    // 3. Write packfile and index
    let (indexed_objs, pack_checksum) = index_packfile(&pack_bytes)?;
    let pack_dir = git_dir.join("objects").join("pack");
    std::fs::create_dir_all(&pack_dir)?;
    let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
    let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
    std::fs::write(&pack_file, &pack_bytes)?;
    PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)?;

    // 4. Determine default branch
    let head_oid_opt = remote_refs.iter().find(|r| r.name == "HEAD").map(|r| r.oid);
    let mut target_branch = "master".to_string();

    if let Some(ref sym) = default_branch {
        if let Some(b) = sym.strip_prefix("refs/heads/") {
            target_branch = b.to_string();
        }
    } else if let Some(head_oid) = head_oid_opt {
        for r in &remote_refs {
            if r.oid == head_oid && r.name != "HEAD" && r.name.starts_with("refs/heads/") {
                target_branch = r.name["refs/heads/".len()..].to_string();
                break;
            }
        }
    }

    // Set up tracking config
    config.set("branch", Some(&target_branch), "remote", "origin");
    config.set(
        "branch",
        Some(&target_branch),
        "merge",
        &format!("refs/heads/{}", target_branch),
    );
    config.save_to_file(&config_path)?;

    // 5. Update remote references and HEAD
    let ref_store = RefStore::new(&git_dir);
    for r in &remote_refs {
        if let Some(branch) = r.name.strip_prefix("refs/heads/") {
            let remote_ref_name = format!("refs/remotes/origin/{}", branch);
            ref_store.update_ref(&remote_ref_name, &r.oid, None, "clone: from remote")?;

            if branch == target_branch {
                let local_ref_name = format!("refs/heads/{}", branch);
                ref_store.update_ref(&local_ref_name, &r.oid, None, "clone: set local branch")?;
                ref_store.set_head_symbolic(branch)?;
            }
        } else if let Some(tag) = r.name.strip_prefix("refs/tags/") {
            let tag_ref_name = format!("refs/tags/{}", tag);
            ref_store.update_ref(&tag_ref_name, &r.oid, None, "clone: set tag")?;
        }
    }

    // 6. Checkout the default branch commit
    let store = RepoObjectStore::open(&git_dir)?;
    if let Ok(head_commit_oid) = ref_store.read_ref(&format!("refs/heads/{}", target_branch)) {
        let obj = store.read_object(&head_commit_oid)?;
        if let Object::Commit(commit) = obj {
            let index_path = git_dir.join("index");
            let mut index = Index::new();
            checkout_tree_and_update_index(
                &target_path,
                &store,
                &mut index,
                None,
                &commit.tree,
                true,
            )?;
            index.write_to(&index_path)?;
        }
    }

    Ok(())
}

fn cmd_fetch(remote_opt: Option<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let config_path = git_dir.join("config");
    let config = GitConfig::load_from_file(&config_path)?;

    let remote_name = remote_opt.unwrap_or_else(|| "origin".to_string());
    let url = config.get_remote_url(&remote_name).ok_or_else(|| {
        anyhow::anyhow!(
            "fatal: '{}' does not appear to be a git repository (no remote config found)",
            remote_name
        )
    })?;

    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let (remote_refs, pack_bytes) = if let Some(local_path) = resolve_local_path(url) {
        let (refs, _) = discover_local_refs(&local_path)?;
        let wants: Vec<ObjectId> = refs
            .iter()
            .filter(|r| !store.exists(&r.oid))
            .map(|r| r.oid)
            .collect();
        let pack = if wants.is_empty() {
            Vec::new()
        } else {
            fetch_local_pack(&local_path, &wants)?
        };
        (refs, pack)
    } else if is_ssh_url(url) {
        let client = SshClient::new();
        let (refs, _, _) = client.discover_upload_pack(url)?;
        let wants: Vec<ObjectId> = refs
            .iter()
            .filter(|r| !store.exists(&r.oid))
            .map(|r| r.oid)
            .collect();
        let pack = if wants.is_empty() {
            Vec::new()
        } else {
            let (p, _) = client.fetch_pack(url, &wants, &[])?;
            p
        };
        (refs, pack)
    } else {
        let client = SmartHttpClient::new();
        let (refs, _, _) = client.discover_upload_pack(url)?;
        let wants: Vec<ObjectId> = refs
            .iter()
            .filter(|r| !store.exists(&r.oid))
            .map(|r| r.oid)
            .collect();
        let pack = if wants.is_empty() {
            Vec::new()
        } else {
            let (p, _) = client.fetch_pack(url, &wants, &[])?;
            p
        };
        (refs, pack)
    };

    if !pack_bytes.is_empty() {
        let (indexed_objs, pack_checksum) = index_packfile(&pack_bytes)?;
        let pack_dir = git_dir.join("objects").join("pack");
        std::fs::create_dir_all(&pack_dir)?;
        let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
        let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
        std::fs::write(&pack_file, &pack_bytes)?;
        PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)?;
    }

    for r in &remote_refs {
        if let Some(branch) = r.name.strip_prefix("refs/heads/") {
            let remote_ref_name = format!("refs/remotes/{}/{}", remote_name, branch);
            ref_store.update_ref(
                &remote_ref_name,
                &r.oid,
                None,
                &format!("fetch: from {}", remote_name),
            )?;
            println!(
                "   {} -> {}/{}",
                &r.oid.to_string()[..7],
                remote_name,
                branch
            );
        }
    }

    Ok(())
}

fn cmd_pull(remote_opt: Option<String>, branch_opt: Option<String>) -> Result<()> {
    cmd_fetch(remote_opt.clone())?;

    let git_dir = find_git_dir(Path::new("."))?;
    let ref_store = RefStore::new(&git_dir);
    let (current_branch, _) = ref_store.resolve_head()?;

    let remote_name = remote_opt.unwrap_or_else(|| "origin".to_string());
    let branch_name = branch_opt.unwrap_or(current_branch);

    let target_ref = format!("refs/remotes/{}/{}", remote_name, branch_name);
    let target_oid = ref_store.read_ref(&target_ref)?;

    cmd_merge(false, Some(target_oid.to_string()))?;
    Ok(())
}

fn is_ancestor(
    store: &impl ObjectReader,
    ancestor: &ObjectId,
    descendant: &ObjectId,
) -> Result<bool> {
    if ancestor == descendant {
        return Ok(true);
    }
    use std::collections::{HashSet, VecDeque};
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();

    queue.push_back(*descendant);
    visited.insert(*descendant);

    while let Some(curr) = queue.pop_front() {
        if curr == *ancestor {
            return Ok(true);
        }
        if let Ok(Object::Commit(c)) = store.read_object(&curr) {
            for p in c.parents {
                if visited.insert(p) {
                    queue.push_back(p);
                }
            }
        }
    }

    Ok(false)
}

fn cmd_push(remote_opt: Option<String>, branch_opt: Option<String>, force: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let config_path = git_dir.join("config");
    let config = GitConfig::load_from_file(&config_path)?;

    let remote_name = remote_opt.unwrap_or_else(|| "origin".to_string());
    let url = config.get_remote_url(&remote_name).ok_or_else(|| {
        anyhow::anyhow!(
            "fatal: No configured push destination for remote '{}'",
            remote_name
        )
    })?;

    let ref_store = RefStore::new(&git_dir);
    let branch = if let Some(b) = branch_opt {
        b
    } else {
        let (curr, _) = ref_store.resolve_head()?;
        curr
    };

    let local_ref_name = format!("refs/heads/{}", branch);
    let local_oid = ref_store.read_ref(&local_ref_name)?;

    // Discover remote refs and capabilities
    let (remote_refs, server_caps, is_local, is_ssh) =
        if let Some(local_path) = resolve_local_path(url) {
            let (refs, _) = discover_local_refs(&local_path)?;
            (refs, Vec::new(), Some(local_path), false)
        } else if is_ssh_url(url) {
            let client = SshClient::new();
            let (refs, caps) = client.discover_receive_pack(url)?;
            (refs, caps, None, true)
        } else {
            let client = SmartHttpClient::new();
            let (refs, caps) = client.discover_receive_pack(url)?;
            (refs, caps, None, false)
        };

    let remote_target_name = format!("refs/heads/{}", branch);
    let remote_old_oid = remote_refs
        .iter()
        .find(|r| r.name == remote_target_name)
        .map(|r| r.oid)
        .unwrap_or(ObjectId::ZERO);

    // 1. Up-to-date check
    if !remote_old_oid.is_zero() && remote_old_oid == local_oid {
        println!("Everything up-to-date");
        return Ok(());
    }

    let store = RepoObjectStore::open(&git_dir)?;

    // 2. Fast-forward ancestry check
    let is_ff = if remote_old_oid.is_zero() {
        true
    } else {
        is_ancestor(&store, &remote_old_oid, &local_oid)?
    };

    if !is_ff && !force {
        anyhow::bail!(
            "fatal: Updates were rejected because the remote contains work that you do \
             not have locally. This is usually caused by another repository pushing to \
             the same ref. You may want to first integrate the remote changes before pushing again.\n\
             hint: See the 'Note about fast-forwards' in 'git push --help' for details.\n\
             hint: Use --force to overwrite."
        );
    }

    // 3. Checked-out branch protection on local non-bare destination
    if let Some(ref dest_path) = is_local {
        if dest_path.join(".git").is_dir() {
            let dest_git_dir = dest_path.join(".git");
            let dest_ref_store = RefStore::new(&dest_git_dir);
            if let Ok((dest_head, _)) = dest_ref_store.resolve_head() {
                let is_checked_out = dest_head == remote_target_name
                    || format!("refs/heads/{}", dest_head) == remote_target_name;
                if is_checked_out {
                    anyhow::bail!(
                        "fatal: refusing to update checked out branch: {}\n\
                         By default, updating the current branch in a non-bare repository is denied.",
                        remote_target_name
                    );
                }
            }
        }
    }

    // 4. Pack only reachable objects required from local_oid, excluding haves
    let haves = if remote_old_oid.is_zero() {
        Vec::new()
    } else {
        vec![remote_old_oid]
    };
    let objects = store.collect_reachable_objects(&[local_oid], &haves)?;
    let (pack_bytes, _, _) = write_pack(&objects, true)?;

    if let Some(dest_path) = is_local {
        // Local destination repository
        let dest_git_dir = if dest_path.join(".git").is_dir() {
            dest_path.join(".git")
        } else {
            dest_path.clone()
        };

        if !pack_bytes.is_empty() {
            // Write pack to destination
            let (indexed_objs, pack_checksum) = index_packfile(&pack_bytes)?;
            let pack_dir = dest_git_dir.join("objects").join("pack");
            std::fs::create_dir_all(&pack_dir)?;
            let pack_file = pack_dir.join(format!("pack-{}.pack", pack_checksum));
            let idx_file = pack_dir.join(format!("pack-{}.idx", pack_checksum));
            std::fs::write(&pack_file, &pack_bytes)?;
            PackIndex::write_to(indexed_objs, &pack_checksum, &idx_file)?;
        }

        // Update branch ref in destination with CAS
        let dest_ref_store = RefStore::new(&dest_git_dir);
        let expected_old = if force || remote_old_oid.is_zero() {
            None
        } else {
            Some(remote_old_oid)
        };
        dest_ref_store.update_ref(
            &remote_target_name,
            &local_oid,
            expected_old.as_ref(),
            "push: from local",
        )?;
    } else if is_ssh {
        let client = SshClient::new();
        let report = client.push_pack_with_caps(
            url,
            &[(&remote_old_oid, &local_oid, &remote_target_name)],
            &pack_bytes,
            &server_caps,
        )?;
        if !report.is_success() {
            let summary = report.display_summary();
            if !summary.is_empty() {
                eprintln!("{}", summary);
            }
            anyhow::bail!("fatal: push rejected by remote");
        }
    } else {
        let client = SmartHttpClient::new();
        let report = client.push_pack_with_caps(
            url,
            &[(&remote_old_oid, &local_oid, &remote_target_name)],
            &pack_bytes,
            &server_caps,
        )?;
        if !report.is_success() {
            let summary = report.display_summary();
            if !summary.is_empty() {
                eprintln!("{}", summary);
            }
            anyhow::bail!("fatal: push rejected by remote");
        }
    }

    // Update local remote tracking ref: refs/remotes/<remote>/<branch>
    let tracking_ref = format!("refs/remotes/{}/{}", remote_name, branch);
    ref_store
        .update_ref(
            &tracking_ref,
            &local_oid,
            None,
            &format!("push: update tracking ref {}", remote_name),
        )
        .with_context(|| {
            format!(
                "remote push succeeded, but local tracking ref '{}' could not be updated",
                tracking_ref
            )
        })?;

    let force_flag = if force && !remote_old_oid.is_zero() && !is_ff {
        "+"
    } else {
        " "
    };
    let old_short = if remote_old_oid.is_zero() {
        "[new branch]".to_string()
    } else {
        format!(
            "{}..{}",
            &remote_old_oid.to_string()[..7],
            &local_oid.to_string()[..7]
        )
    };
    println!("To {}", url);
    println!("  {}{} {} -> {}", force_flag, old_short, branch, branch);

    Ok(())
}

fn cmd_remote(subcommand_opt: Option<RemoteCommand>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let config_path = git_dir.join("config");
    let mut config = GitConfig::load_from_file(&config_path)?;

    match subcommand_opt {
        None => {
            for (name, _) in config.list_remotes() {
                println!("{}", name);
            }
        }
        Some(RemoteCommand::Add { name, url }) => {
            if config.get_remote_url(&name).is_some() {
                bail!("error: remote '{}' already exists", name);
            }
            config.add_remote(&name, &url);
            config.save_to_file(&config_path)?;
        }
        Some(RemoteCommand::Remove { name }) => {
            let original = std::fs::read(&config_path)?;
            if !config.remove_remote(&name) {
                bail!("error: No such remote: '{}'", name);
            }
            config.save_to_file(&config_path)?;
            let ref_store = RefStore::new(&git_dir);
            if let Err(error) = ref_store.remove_remote_refs(&name) {
                if let Err(rollback_error) = restore_file_atomic(&config_path, &original) {
                    bail!(
                        "failed to remove tracking refs for remote '{}': {}; restoring config also failed: {}",
                        name,
                        error,
                        rollback_error
                    );
                }
                return Err(error.into());
            }
        }
        Some(RemoteCommand::Rename { old, new }) => {
            if config.get_remote_url(&new).is_some() {
                bail!("error: remote '{}' already exists", new);
            }
            let original = std::fs::read(&config_path)?;
            if !config.rename_subsection("remote", &old, &new)? {
                bail!("error: No such remote: '{}'", old);
            }
            config.replace_in_values(
                "remote",
                Some(&new),
                "fetch",
                &format!("refs/remotes/{}/", old),
                &format!("refs/remotes/{}/", new),
            );
            for branch in config.subsections("branch") {
                if config.get("branch", Some(&branch), "remote") == Some(old.as_str()) {
                    config.set("branch", Some(&branch), "remote", &new);
                }
            }
            config.save_to_file(&config_path)?;
            let ref_store = RefStore::new(&git_dir);
            if let Err(error) = ref_store.rename_remote_refs(&old, &new) {
                if let Err(rollback_error) = restore_file_atomic(&config_path, &original) {
                    bail!(
                        "failed to rename tracking refs from '{}' to '{}': {}; restoring config also failed: {}",
                        old,
                        new,
                        error,
                        rollback_error
                    );
                }
                return Err(error.into());
            }
        }
    }
    Ok(())
}

fn restore_file_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let mut lock = LockFile::acquire(path)?;
    lock.write_all(content)?;
    lock.commit()?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cmd_rm(cached: bool, recursive: bool, force: bool, files: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    // Resolve HEAD tree to check staged changes vs HEAD
    let head_map = match ref_store.resolve_head()?.1 {
        Some(head_oid) => match store.read_object(&head_oid)? {
            Object::Commit(commit) => flatten_tree(&store, &commit.tree, "")?,
            _ => bail!("HEAD does not point to a commit"),
        },
        None => BTreeMap::new(),
    };

    // 1. Preflight: Resolve all file pathspecs to matched index entries
    let mut files_to_remove: Vec<String> = Vec::new();
    let current_dir = std::env::current_dir()?;

    for file_arg in &files {
        let p = Path::new(file_arg);
        let abs_path = if p.is_relative() {
            current_dir.join(p)
        } else {
            p.to_path_buf()
        };

        let rel_path = match abs_path.strip_prefix(repo_root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => bail!("path '{}' is outside repository root", file_arg),
        };

        let dir_prefix = format!("{}/", rel_path);
        let mut matched = false;

        if index.get_entry(&rel_path).is_some() {
            if !files_to_remove.contains(&rel_path) {
                files_to_remove.push(rel_path.clone());
            }
            matched = true;
        } else if recursive {
            let matched_paths: Vec<String> = index
                .entries()
                .iter()
                .filter(|e| e.path.starts_with(&dir_prefix))
                .map(|e| e.path.clone())
                .collect();
            if !matched_paths.is_empty() {
                for mp in matched_paths {
                    if !files_to_remove.contains(&mp) {
                        files_to_remove.push(mp);
                    }
                }
                matched = true;
            }
        }

        if !matched {
            bail!("fatal: pathspec '{}' did not match any files", file_arg);
        }
    }

    // 2. Preflight dirty checks (if !force)
    if !force {
        let mut dirty_files = Vec::new();

        for rel_path in &files_to_remove {
            let entry = index.get_entry(rel_path).unwrap();
            let full_path = oxidize_core::safe_join(repo_root, rel_path)?;

            // Case A: Worktree modified compared to index (if not cached)
            if !cached && full_path.is_file() {
                let data = std::fs::read(&full_path)?;
                let wt_oid = ObjectId::hash_blob(&data);
                if wt_oid != entry.oid {
                    dirty_files.push(rel_path.clone());
                    continue;
                }
            }

            // Case B: Staged changes compared to HEAD
            let head_entry = head_map.get(rel_path);
            let head_oid = head_entry.map(|(_, oid)| *oid);
            if head_oid != Some(entry.oid) {
                if !cached {
                    dirty_files.push(rel_path.clone());
                } else if full_path.is_file() {
                    let data = std::fs::read(&full_path)?;
                    let wt_oid = ObjectId::hash_blob(&data);
                    if wt_oid != entry.oid {
                        dirty_files.push(rel_path.clone());
                    }
                }
            }
        }

        if !dirty_files.is_empty() {
            bail!(
                "error: the following file has local modifications:\n    {}\n(use --cached to keep the file, or -f to force removal)",
                dirty_files.join("\n    ")
            );
        }
    }

    // 3. Execution: remove from index, and remove from worktree if !cached
    for rel_path in &files_to_remove {
        index.remove_entry(rel_path);
        if !cached {
            let full_path = oxidize_core::safe_join(repo_root, rel_path)?;
            if full_path.exists() {
                std::fs::remove_file(&full_path)?;
            }
            // Clean up empty directories only
            let mut parent = full_path.parent();
            while let Some(p) = parent {
                if p == repo_root || !p.starts_with(repo_root) {
                    break;
                }
                if std::fs::remove_dir(p).is_err() {
                    break; // Directory is not empty, stop climbing
                }
                parent = p.parent();
            }
        }
        println!("rm '{}'", rel_path);
    }

    index.write_to(&index_path)?;
    Ok(())
}

fn cmd_mv(source: String, destination: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let src_p = Path::new(&source);
    let src_abs = if src_p.is_relative() {
        std::env::current_dir()?.join(src_p)
    } else {
        src_p.to_path_buf()
    };

    let dest_p = Path::new(&destination);
    let dest_abs = if dest_p.is_relative() {
        std::env::current_dir()?.join(dest_p)
    } else {
        dest_p.to_path_buf()
    };

    let src_rel = src_abs
        .strip_prefix(repo_root)
        .with_context(|| format!("source '{}' is outside repository root", source))?
        .to_string_lossy()
        .replace('\\', "/");

    let dest_rel = dest_abs
        .strip_prefix(repo_root)
        .with_context(|| format!("destination '{}' is outside repository root", destination))?
        .to_string_lossy()
        .replace('\\', "/");

    if let Some(parent) = dest_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&src_abs, &dest_abs)?;

    if let Some(entry) = index.get_entry(&src_rel).cloned() {
        index.remove_entry(&src_rel);
        let mut new_entry = entry;
        new_entry.path = dest_rel;
        if let Ok(meta) = std::fs::metadata(&dest_abs) {
            new_entry.file_size = meta.len() as u32;
        }
        index.add_entry(new_entry);
    } else {
        let dir_prefix = format!("{}/", src_rel);
        let entries_to_rename: Vec<IndexEntry> = index
            .entries()
            .iter()
            .filter(|e| e.path.starts_with(&dir_prefix))
            .cloned()
            .collect();

        if entries_to_rename.is_empty() {
            bail!("fatal: not under version control: {}", source);
        }

        for mut entry in entries_to_rename {
            index.remove_entry(&entry.path);
            let sub = &entry.path[dir_prefix.len()..];
            entry.path = format!("{}/{}", dest_rel, sub);
            index.add_entry(entry);
        }
    }

    index.write_to(&index_path)?;
    Ok(())
}

fn cmd_restore(staged: bool, files: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let head_oid_opt = ref_store.resolve_head()?.1;
    let head_tree_map = if let Some(head_oid) = head_oid_opt {
        let head_commit = match store.read_object(&head_oid)? {
            Object::Commit(c) => c,
            _ => bail!("HEAD is not a commit"),
        };
        flatten_tree(&store, &head_commit.tree, "")?
    } else {
        std::collections::BTreeMap::new()
    };

    for file_arg in files {
        let p = Path::new(&file_arg);
        let abs = if p.is_relative() {
            std::env::current_dir()?.join(p)
        } else {
            p.to_path_buf()
        };

        let rel_path = abs
            .strip_prefix(repo_root)
            .with_context(|| format!("path '{}' is outside repository root", file_arg))?
            .to_string_lossy()
            .replace('\\', "/");

        if staged {
            if let Some((_mode, head_oid)) = head_tree_map.get(&rel_path) {
                let full_path = repo_root.join(&rel_path);
                if let Ok(meta) = std::fs::metadata(&full_path) {
                    index.add_entry(IndexEntry::from_fs_metadata(rel_path, *head_oid, &meta, 0));
                } else {
                    let (_, raw_bytes) = store.read_raw(head_oid)?;
                    index.add_entry(IndexEntry {
                        ctime_sec: 0,
                        ctime_nsec: 0,
                        mtime_sec: 0,
                        mtime_nsec: 0,
                        dev: 0,
                        ino: 0,
                        mode: 0o100644,
                        uid: 0,
                        gid: 0,
                        file_size: raw_bytes.len() as u32,
                        oid: *head_oid,
                        stage: 0,
                        assume_valid: false,
                        path: rel_path,
                    });
                }
            } else {
                index.remove_entry(&rel_path);
            }
        } else {
            let entry = index.get_entry(&rel_path).ok_or_else(|| {
                anyhow::anyhow!(
                    "pathspec '{}' did not match any file(s) known to git",
                    file_arg
                )
            })?;

            let (obj_type, data) = store.read_raw(&entry.oid)?;
            if obj_type != ObjectType::Blob {
                bail!("object {} is not a blob", entry.oid);
            }

            let full_path = repo_root.join(&rel_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, &data)?;
            if let Ok(meta) = std::fs::metadata(&full_path) {
                index.add_entry(IndexEntry::from_fs_metadata(rel_path, entry.oid, &meta, 0));
            }
        }
    }

    index.write_to(&index_path)?;
    Ok(())
}

fn cmd_tag(
    delete: bool,
    annotate: bool,
    message: Option<String>,
    name_opt: Option<String>,
    target_opt: Option<String>,
) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let tags_dir = git_dir.join("refs").join("tags");

    if delete {
        let name = name_opt.context("tag name required for deletion")?;
        let tag_path = tags_dir.join(&name);
        if !tag_path.exists() {
            bail!("tag '{}' not found", name);
        }
        std::fs::remove_file(tag_path)?;
        println!("Deleted tag '{}'", name);
        return Ok(());
    }

    let name = match name_opt {
        Some(n) => n,
        None => {
            let mut tags = Vec::new();
            if tags_dir.is_dir() {
                for entry in std::fs::read_dir(tags_dir)? {
                    let entry = entry?;
                    tags.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            tags.sort();
            for t in tags {
                println!("{}", t);
            }
            return Ok(());
        }
    };

    let store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(&git_dir);
    let target_str = target_opt.unwrap_or_else(|| "HEAD".to_string());
    let target_oid = ref_store.resolve_rev(&target_str, &store)?;

    std::fs::create_dir_all(&tags_dir)?;
    let tag_file = tags_dir.join(&name);

    if annotate || message.is_some() {
        let sig = get_default_signature(Some(&git_dir));
        let tag_obj = Object::Tag(CoreTag {
            target: target_oid,
            target_type: ObjectType::Commit,
            name: name.clone(),
            tagger: Some(sig),
            message: message.unwrap_or_default(),
        });
        let tag_oid = store.write_object(&tag_obj)?;
        std::fs::write(tag_file, format!("{}\n", tag_oid))?;
    } else {
        std::fs::write(tag_file, format!("{}\n", target_oid))?;
    }

    Ok(())
}

fn cmd_stash(subcommand_opt: Option<StashCommand>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let ref_store = RefStore::new(&git_dir);

    match subcommand_opt {
        None | Some(StashCommand::Push { .. }) => {
            let custom_msg = if let Some(StashCommand::Push { message }) = subcommand_opt {
                message
            } else {
                None
            };
            let msg = custom_msg.as_deref().unwrap_or("");
            let _stash_oid = oxidize_tui::ops::stash_save(repo_root, &git_dir, msg)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!(
                "Saved working directory and index state {}",
                if msg.is_empty() { "WIP" } else { msg }
            );
        }
        Some(StashCommand::List) => {
            let list = ref_store.stash_list()?;
            for (idx, entry) in list.iter().enumerate() {
                println!("stash@{{{}}}: {}", idx, entry.message);
            }
        }
        Some(StashCommand::Pop) => {
            let idx = 0;
            let list = ref_store.stash_list()?;
            let entry = list.get(idx).context("No stash entries found.")?;
            let stash_oid = entry.new_oid;

            let clean = oxidize_tui::ops::pop_stash(repo_root, &git_dir, idx, &stash_oid)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            if clean {
                println!("Dropped refs/stash@{{{}}} ({})", idx, stash_oid);
            } else {
                println!("CONFLICT: Merge conflict in stashed files.");
                println!("The stash entry is kept in case you need it again.");
                bail!("merge conflict when popping stash");
            }
        }
        Some(StashCommand::Drop { index }) => {
            let idx = index.unwrap_or(0);
            let dropped_oid = oxidize_tui::ops::drop_stash(&git_dir, idx)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Dropped refs/stash@{{{}}} ({})", idx, dropped_oid);
        }
    }

    Ok(())
}

fn cmd_rebase(upstream_arg: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let (active_branch, head_oid_opt) = ref_store.resolve_head()?;
    let head_oid = head_oid_opt.context("cannot rebase: HEAD has no commits")?;
    let upstream_oid = ref_store.resolve_rev(&upstream_arg, &store)?;

    let merge_base = ref_store
        .find_merge_base(&store, &head_oid, &upstream_oid)?
        .context("no common ancestor found between HEAD and upstream")?;

    if merge_base == head_oid {
        let up_commit = match store.read_object(&upstream_oid)? {
            Object::Commit(c) => c,
            _ => bail!("upstream is not a commit"),
        };
        checkout_tree_and_update_index(
            repo_root,
            &store,
            &mut index,
            None,
            &up_commit.tree,
            false,
        )?;
        index.write_to(&index_path)?;
        ref_store.update_ref(
            &format!("refs/heads/{}", active_branch),
            &upstream_oid,
            Some(&head_oid),
            &format!("rebase: fast-forward to {}", upstream_arg),
        )?;
        println!("Current branch {} is up to date.", active_branch);
        return Ok(());
    }

    if merge_base == upstream_oid {
        println!("Current branch {} is up to date.", active_branch);
        return Ok(());
    }

    let mut commits_to_replay = Vec::new();
    let mut curr = head_oid;
    while curr != merge_base {
        let commit = match store.read_object(&curr)? {
            Object::Commit(c) => c,
            _ => bail!("object {} is not a commit", curr),
        };
        commits_to_replay.push((curr, commit.clone()));
        if commit.parents.is_empty() {
            break;
        }
        curr = commit.parents[0];
    }
    commits_to_replay.reverse();

    println!("First, rewinding head to replay your work on top of it...");

    let mut current_tip = upstream_oid;
    let up_commit = match store.read_object(&upstream_oid)? {
        Object::Commit(c) => c,
        _ => bail!("upstream is not a commit"),
    };

    checkout_tree_and_update_index(repo_root, &store, &mut index, None, &up_commit.tree, true)?;

    for (_orig_oid, commit) in commits_to_replay {
        let base_oid = commit.parents[0];
        let base_commit = match store.read_object(&base_oid)? {
            Object::Commit(c) => c,
            _ => bail!("base commit not found"),
        };
        let tip_commit = match store.read_object(&current_tip)? {
            Object::Commit(c) => c,
            _ => bail!("tip commit not found"),
        };

        let base_files = flatten_tree(&store, &base_commit.tree, "")?;
        let our_files = flatten_tree(&store, &tip_commit.tree, "")?;
        let their_files = flatten_tree(&store, &commit.tree, "")?;

        let mut all_paths = std::collections::BTreeSet::new();
        for p in base_files.keys() {
            all_paths.insert(p.clone());
        }
        for p in our_files.keys() {
            all_paths.insert(p.clone());
        }
        for p in their_files.keys() {
            all_paths.insert(p.clone());
        }

        let mut had_conflicts = false;
        for path in all_paths {
            let base_text = get_blob_text(&store, base_files.get(&path).map(|(_, id)| id));
            let our_text = get_blob_text(&store, our_files.get(&path).map(|(_, id)| id));
            let their_text = get_blob_text(&store, their_files.get(&path).map(|(_, id)| id));

            let merged = three_way_merge(&base_text, &our_text, &their_text, "HEAD", &upstream_arg);
            let full_path = repo_root.join(&path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, merged.content.as_bytes())?;

            let blob = Object::Blob(Blob::new(merged.content.into_bytes()));
            let blob_oid = store.write_object(&blob)?;
            if let Ok(meta) = std::fs::metadata(&full_path) {
                let stage = if merged.has_conflicts { 1 } else { 0 };
                index.add_entry(IndexEntry::from_fs_metadata(
                    path.clone(),
                    blob_oid,
                    &meta,
                    stage,
                ));
            }
            if merged.has_conflicts {
                had_conflicts = true;
                println!("CONFLICT (content): Merge conflict in {}", path);
            }
        }

        if had_conflicts {
            index.write_to(&index_path)?;
            bail!("could not apply commit: merge conflict. Fix conflicts and commit.");
        }

        let new_tree_oid = write_tree(&index, store.loose())?;
        let sig = get_default_signature(Some(&git_dir));
        let new_commit = Commit {
            tree: new_tree_oid,
            parents: vec![current_tip],
            author: commit.author,
            committer: sig,
            gpg_sig: None,
            message: commit.message,
        };
        current_tip = store.write_object(&Object::Commit(new_commit))?;
    }

    index.write_to(&index_path)?;
    ref_store.update_ref(
        &format!("refs/heads/{}", active_branch),
        &current_tip,
        Some(&head_oid),
        &format!("rebase finished: returning to refs/heads/{}", active_branch),
    )?;

    println!(
        "Successfully rebased and updated refs/heads/{}.",
        active_branch
    );
    Ok(())
}

fn cmd_cherry_pick(commit_arg: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let (active_branch, head_oid_opt) = ref_store.resolve_head()?;
    let head_oid = head_oid_opt.context("HEAD has no commits")?;
    let target_oid = ref_store.resolve_rev(&commit_arg, store.loose())?;

    let target_commit = match store.read_object(&target_oid)? {
        Object::Commit(c) => c,
        _ => bail!("target is not a commit"),
    };
    let head_commit = match store.read_object(&head_oid)? {
        Object::Commit(c) => c,
        _ => bail!("HEAD is not a commit"),
    };

    let base_files = if !target_commit.parents.is_empty() {
        let p_commit = match store.read_object(&target_commit.parents[0])? {
            Object::Commit(c) => c,
            _ => bail!("parent is not a commit"),
        };
        flatten_tree(&store, &p_commit.tree, "")?
    } else {
        std::collections::BTreeMap::new()
    };

    let our_files = flatten_tree(&store, &head_commit.tree, "")?;
    let their_files = flatten_tree(&store, &target_commit.tree, "")?;

    let mut all_paths = std::collections::BTreeSet::new();
    for p in base_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in our_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in their_files.keys() {
        all_paths.insert(p.clone());
    }

    let mut had_conflicts = false;
    for path in all_paths {
        let base_text = get_blob_text(&store, base_files.get(&path).map(|(_, id)| id));
        let our_text = get_blob_text(&store, our_files.get(&path).map(|(_, id)| id));
        let their_text = get_blob_text(&store, their_files.get(&path).map(|(_, id)| id));

        let merged = three_way_merge(&base_text, &our_text, &their_text, "HEAD", &commit_arg);
        let full_path = repo_root.join(&path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, merged.content.as_bytes())?;

        let blob = Object::Blob(Blob::new(merged.content.into_bytes()));
        let blob_oid = store.write_object(&blob)?;
        if let Ok(meta) = std::fs::metadata(&full_path) {
            let stage = if merged.has_conflicts { 1 } else { 0 };
            index.add_entry(IndexEntry::from_fs_metadata(
                path.clone(),
                blob_oid,
                &meta,
                stage,
            ));
        }
        if merged.has_conflicts {
            had_conflicts = true;
            println!("CONFLICT (content): Merge conflict in {}", path);
        }
    }

    index.write_to(&index_path)?;
    if had_conflicts {
        bail!("could not apply {}; merge conflict", target_oid);
    }

    let tree_oid = write_tree(&index, store.loose())?;
    let sig = get_default_signature(Some(&git_dir));
    let new_commit = Commit {
        tree: tree_oid,
        parents: vec![head_oid],
        author: target_commit.author,
        committer: sig,
        gpg_sig: None,
        message: target_commit.message.clone(),
    };
    let new_oid = store.write_object(&Object::Commit(new_commit))?;
    ref_store.update_ref(
        &format!("refs/heads/{}", active_branch),
        &new_oid,
        Some(&head_oid),
        &format!(
            "cherry-pick: {}",
            target_commit.message.lines().next().unwrap_or("")
        ),
    )?;

    println!(
        "[{}] {}",
        &new_oid.to_string()[..7],
        target_commit.message.lines().next().unwrap_or("")
    );
    Ok(())
}

fn cmd_revert(commit_arg: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    let (active_branch, head_oid_opt) = ref_store.resolve_head()?;
    let head_oid = head_oid_opt.context("HEAD has no commits")?;
    let target_oid = ref_store.resolve_rev(&commit_arg, store.loose())?;

    let target_commit = match store.read_object(&target_oid)? {
        Object::Commit(c) => c,
        _ => bail!("target is not a commit"),
    };
    let head_commit = match store.read_object(&head_oid)? {
        Object::Commit(c) => c,
        _ => bail!("HEAD is not a commit"),
    };

    let parent_files = if !target_commit.parents.is_empty() {
        let p_commit = match store.read_object(&target_commit.parents[0])? {
            Object::Commit(c) => c,
            _ => bail!("parent is not a commit"),
        };
        flatten_tree(&store, &p_commit.tree, "")?
    } else {
        std::collections::BTreeMap::new()
    };

    let base_files = flatten_tree(&store, &target_commit.tree, "")?;
    let our_files = flatten_tree(&store, &head_commit.tree, "")?;
    let their_files = parent_files;

    let mut all_paths = std::collections::BTreeSet::new();
    for p in base_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in our_files.keys() {
        all_paths.insert(p.clone());
    }
    for p in their_files.keys() {
        all_paths.insert(p.clone());
    }

    let mut had_conflicts = false;
    for path in all_paths {
        let base_text = get_blob_text(&store, base_files.get(&path).map(|(_, id)| id));
        let our_text = get_blob_text(&store, our_files.get(&path).map(|(_, id)| id));
        let their_text = get_blob_text(&store, their_files.get(&path).map(|(_, id)| id));

        let merged = three_way_merge(
            &base_text,
            &our_text,
            &their_text,
            "HEAD",
            &format!("parent of {}", commit_arg),
        );
        let full_path = repo_root.join(&path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, merged.content.as_bytes())?;

        let blob = Object::Blob(Blob::new(merged.content.into_bytes()));
        let blob_oid = store.write_object(&blob)?;
        if let Ok(meta) = std::fs::metadata(&full_path) {
            let stage = if merged.has_conflicts { 1 } else { 0 };
            index.add_entry(IndexEntry::from_fs_metadata(
                path.clone(),
                blob_oid,
                &meta,
                stage,
            ));
        }
        if merged.has_conflicts {
            had_conflicts = true;
            println!("CONFLICT (content): Merge conflict in {}", path);
        }
    }

    index.write_to(&index_path)?;
    if had_conflicts {
        bail!("could not revert {}; merge conflict", target_oid);
    }

    let tree_oid = write_tree(&index, store.loose())?;
    let sig = get_default_signature(Some(&git_dir));
    let first_line = target_commit.message.lines().next().unwrap_or("");
    let revert_msg = format!(
        "Revert \"{}\"\n\nThis reverts commit {}.\n",
        first_line, target_oid
    );

    let new_commit = Commit {
        tree: tree_oid,
        parents: vec![head_oid],
        author: sig.clone(),
        committer: sig,
        gpg_sig: None,
        message: revert_msg,
    };
    let new_oid = store.write_object(&Object::Commit(new_commit))?;
    ref_store.update_ref(
        &format!("refs/heads/{}", active_branch),
        &new_oid,
        Some(&head_oid),
        &format!("revert: {}", first_line),
    )?;

    println!("[{}] Revert \"{}\"", &new_oid.to_string()[..7], first_line);
    Ok(())
}

fn cmd_blame(file_arg: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    let head_oid = ref_store.resolve_rev("HEAD", store.loose())?;
    let head_commit = match store.read_object(&head_oid)? {
        Object::Commit(c) => c,
        _ => bail!("HEAD is not a commit"),
    };

    let files = flatten_tree(&store, &head_commit.tree, "")?;
    let (_, blob_oid) = files
        .get(&file_arg)
        .ok_or_else(|| anyhow::anyhow!("fatal: no such path '{}' in HEAD", file_arg))?;

    let (_, data) = store.read_raw(blob_oid)?;
    let text = String::from_utf8_lossy(&data).to_string();
    let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    let mut attribution: Vec<(ObjectId, Signature, usize)> = lines
        .iter()
        .enumerate()
        .map(|(idx, _)| (head_oid, head_commit.author.clone(), idx + 1))
        .collect();

    let mut queue: std::collections::VecDeque<(ObjectId, Vec<String>)> =
        std::collections::VecDeque::new();
    queue.push_back((head_oid, lines.clone()));

    let mut visited = std::collections::HashSet::new();
    visited.insert(head_oid);

    while let Some((c_oid, c_lines)) = queue.pop_front() {
        if let Ok(Object::Commit(c)) = store.read_object(&c_oid) {
            for parent_oid in c.parents {
                if visited.insert(parent_oid) {
                    if let Ok(Object::Commit(p_commit)) = store.read_object(&parent_oid) {
                        let p_files = flatten_tree(&store, &p_commit.tree, "").unwrap_or_default();
                        if let Some((_, p_blob_oid)) = p_files.get(&file_arg) {
                            if let Ok((_, p_data)) = store.read_raw(p_blob_oid) {
                                let p_text = String::from_utf8_lossy(&p_data).to_string();
                                let p_lines: Vec<String> =
                                    p_text.lines().map(|s| s.to_string()).collect();

                                let p_slices: Vec<&str> =
                                    p_lines.iter().map(|s| s.as_str()).collect();
                                let c_slices: Vec<&str> =
                                    c_lines.iter().map(|s| s.as_str()).collect();
                                let diff = oxidize_diff::myers_diff(&p_slices, &c_slices);
                                let mut p_idx = 0;
                                let mut c_idx = 0;

                                for op in diff {
                                    match op {
                                        oxidize_diff::DiffOp::Keep(_) => {
                                            for item in &mut attribution {
                                                if item.0 == c_oid && item.2 == c_idx + 1 {
                                                    item.0 = parent_oid;
                                                    item.1 = p_commit.author.clone();
                                                    item.2 = p_idx + 1;
                                                }
                                            }
                                            p_idx += 1;
                                            c_idx += 1;
                                        }
                                        oxidize_diff::DiffOp::Delete(_) => {
                                            p_idx += 1;
                                        }
                                        oxidize_diff::DiffOp::Insert(_) => {
                                            c_idx += 1;
                                        }
                                    }
                                }

                                queue.push_back((parent_oid, p_lines));
                            }
                        }
                    }
                }
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        let (oid, sig, orig_line) = &attribution[idx];
        let short_oid = &oid.to_string()[..8];
        println!(
            "{} ({:<15} {} {:>2}) {}",
            short_oid, sig.name, sig.time_seconds, orig_line, line
        );
    }

    Ok(())
}

fn cmd_bisect(args: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let store = RepoObjectStore::open(&git_dir)?;
    let ref_store = RefStore::new(&git_dir);

    if args.is_empty() {
        println!("usage: ox bisect <subcommand> [<options>]");
        println!("   subcommands: start, bad, good, reset");
        return Ok(());
    }

    let sub = args[0].as_str();
    match sub {
        "start" => {
            let (active_branch, _) = ref_store.resolve_head()?;
            std::fs::write(git_dir.join("BISECT_START"), active_branch)?;
            if args.len() > 1 {
                let bad_rev = &args[1];
                let oid = ref_store.resolve_rev(bad_rev, store.loose())?;
                std::fs::write(git_dir.join("BISECT_BAD"), format!("{}\n", oid))?;
            }
            if args.len() > 2 {
                let good_rev = &args[2];
                let oid = ref_store.resolve_rev(good_rev, store.loose())?;
                std::fs::write(git_dir.join("BISECT_GOOD"), format!("{}\n", oid))?;
            }
            step_bisect(&git_dir, &store, &ref_store)?;
        }
        "bad" => {
            let rev = if args.len() > 1 {
                args[1].clone()
            } else {
                "HEAD".to_string()
            };
            let oid = ref_store.resolve_rev(&rev, store.loose())?;
            std::fs::write(git_dir.join("BISECT_BAD"), format!("{}\n", oid))?;
            step_bisect(&git_dir, &store, &ref_store)?;
        }
        "good" => {
            let rev = if args.len() > 1 {
                args[1].clone()
            } else {
                "HEAD".to_string()
            };
            let oid = ref_store.resolve_rev(&rev, store.loose())?;
            std::fs::write(git_dir.join("BISECT_GOOD"), format!("{}\n", oid))?;
            step_bisect(&git_dir, &store, &ref_store)?;
        }
        "reset" => {
            let start_file = git_dir.join("BISECT_START");
            if start_file.exists() {
                let orig_branch = std::fs::read_to_string(&start_file)?.trim().to_string();
                cmd_checkout(None, Some(orig_branch))?;
                remove_file_if_exists(&start_file)?;
                remove_file_if_exists(&git_dir.join("BISECT_BAD"))?;
                remove_file_if_exists(&git_dir.join("BISECT_GOOD"))?;
            } else {
                println!("We are not bisecting.");
            }
        }
        other => {
            bail!("unknown bisect subcommand: {}", other);
        }
    }

    Ok(())
}

fn step_bisect(git_dir: &Path, store: &RepoObjectStore, _ref_store: &RefStore) -> Result<()> {
    let bad_file = git_dir.join("BISECT_BAD");
    let good_file = git_dir.join("BISECT_GOOD");

    if !bad_file.exists() || !good_file.exists() {
        return Ok(());
    }

    let bad_oid: ObjectId = std::fs::read_to_string(bad_file)?.trim().parse()?;
    let good_oid: ObjectId = std::fs::read_to_string(good_file)?.trim().parse()?;

    let mut good_ancestors = std::collections::HashSet::new();
    let mut q = std::collections::VecDeque::new();
    q.push_back(good_oid);
    good_ancestors.insert(good_oid);

    while let Some(c) = q.pop_front() {
        if let Ok(Object::Commit(commit)) = store.read_object(&c) {
            for p in commit.parents {
                if good_ancestors.insert(p) {
                    q.push_back(p);
                }
            }
        }
    }

    let mut candidates = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut q2 = std::collections::VecDeque::new();
    q2.push_back(bad_oid);
    visited.insert(bad_oid);

    while let Some(c) = q2.pop_front() {
        if !good_ancestors.contains(&c) {
            candidates.push(c);
            if let Ok(Object::Commit(commit)) = store.read_object(&c) {
                for p in commit.parents {
                    if visited.insert(p) {
                        q2.push_back(p);
                    }
                }
            }
        }
    }

    if candidates.len() <= 1 {
        let first_bad = candidates.first().copied().unwrap_or(bad_oid);
        println!("{} is the first bad commit", first_bad);
        if let Ok(Object::Commit(commit)) = store.read_object(&first_bad) {
            println!("Author: {}", commit.author);
            println!("\n    {}", commit.message.trim());
        }
        return Ok(());
    }

    let mid_idx = candidates.len() / 2;
    let next_test = candidates[mid_idx];
    let steps = (candidates.len() as f64).log2().ceil() as usize;

    println!(
        "Bisecting: {} revisions left to test after this (roughly {} step{})",
        candidates.len(),
        steps,
        if steps == 1 { "" } else { "s" }
    );

    cmd_checkout(None, Some(next_test.to_string()))?;
    Ok(())
}

fn cmd_reflog() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let ref_store = RefStore::new(&git_dir);
    let entries = ref_store.read_reflog("HEAD")?;

    if entries.is_empty() {
        return Ok(());
    }

    for (idx, entry) in entries.iter().rev().enumerate() {
        let short_oid = &entry.new_oid.to_string()[..7];
        println!("{} HEAD@{{{}}}: {}", short_oid, idx, entry.message);
    }

    Ok(())
}
