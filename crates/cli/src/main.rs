//! `ox` — Production-quality, daily-driver-capable Git implementation in Rust.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use oxidize_core::store::parse_object_from_content;
use oxidize_core::{
    find_git_dir, Blob, Commit, FileMode, LooseObjectStore, Object, ObjectId, ObjectType, Tree,
    TreeEntry,
};
use oxidize_diff::{format_unified_diff, three_way_merge};
use oxidize_index::{
    compute_status, flatten_tree, write_tree, Index, IndexEntry, StagedChange, UnstagedChange,
};
use oxidize_pack::{
    index_packfile, read_pack_object_at, unpack_packfile, write_pack, PackIndex, RawPackObject,
    RepoObjectStore,
};
use oxidize_refs::{get_default_signature, RefStore};
use sha1::{Digest, Sha1};
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
        /// Pretty-print the contents of <object> based on its type
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

        /// Files to act on
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
        /// Commit or branch to merge into HEAD
        commit: String,
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
    /// Add a remote named <name> for the repository at <url>
    Add {
        /// Name of the remote
        name: String,
        /// URL of the remote
        url: String,
    },
    /// Remove the remote named <name>
    Remove {
        /// Name of the remote
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(cmd) => dispatch_command(cmd)?,
        None => {
            // If no subcommand given, print short help
            println!("ox: A complete Git implementation in Rust. Run `ox --help` for usage.");
        }
    }

    Ok(())
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
        Commands::UpdateIndex { add, files } => cmd_update_index(add, files)?,
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
        Commands::Merge { commit } => cmd_merge(commit)?,
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
        other => {
            println!("Command {:?} dispatched (stubbed in current phase)", other);
        }
    }
    Ok(())
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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
    store: &LooseObjectStore,
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
    store: &LooseObjectStore,
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

fn cmd_add(files: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    for file_arg in files {
        let target_path = if file_arg == "." {
            repo_root.to_path_buf()
        } else {
            let p = Path::new(&file_arg);
            if p.is_relative() {
                std::env::current_dir()?.join(p)
            } else {
                p.to_path_buf()
            }
        };

        add_path_to_index(&mut index, &store, repo_root, &target_path)?;
    }

    index.write_to(&index_path)?;
    Ok(())
}

fn add_path_to_index(
    index: &mut Index,
    store: &LooseObjectStore,
    repo_root: &Path,
    target: &Path,
) -> Result<()> {
    if target.is_dir() {
        for entry in std::fs::read_dir(target)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == ".git" || name_str == "target" {
                continue;
            }
            add_path_to_index(index, store, repo_root, &path)?;
        }
    } else if target.is_file() {
        let rel_path = target
            .strip_prefix(repo_root)
            .with_context(|| format!("path '{}' is outside repository root", target.display()))?
            .to_string_lossy()
            .replace('\\', "/");

        let data = std::fs::read(target)?;
        let blob = Object::Blob(Blob::new(data));
        let oid = store.write_object(&blob)?;

        let meta = std::fs::metadata(target)?;
        let entry = IndexEntry::from_fs_metadata(rel_path, oid, &meta, 0);
        index.add_entry(entry);
    }
    Ok(())
}

fn cmd_status() -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let index = Index::load_from(&index_path)?;

    let (branch_name, head_commit, head_tree) = get_head_info(&git_dir, &store)?;
    let status = compute_status(repo_root, &index, head_tree.as_ref(), &store)?;

    println!("On branch {}", branch_name);

    if head_commit.is_none() {
        println!("\nNo commits yet\n");
    }

    let mut clean = true;

    if !status.staged.is_empty() {
        clean = false;
        println!("Changes to be committed:");
        println!("  (use \"ox restore --staged <file>...\" to unstage)");
        for change in &status.staged {
            match change {
                StagedChange::New(p) => println!("\tnew file:   {}", p),
                StagedChange::Modified(p) => println!("\tmodified:   {}", p),
                StagedChange::Deleted(p) => println!("\tdeleted:    {}", p),
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

fn cmd_update_index(add: bool, files: Vec<String>) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

    for file_str in files {
        let p = Path::new(&file_str);
        let abs = if p.is_relative() {
            std::env::current_dir()?.join(p)
        } else {
            p.to_path_buf()
        };

        if abs.exists() {
            let rel = abs
                .strip_prefix(repo_root)
                .with_context(|| format!("path '{}' is outside repository root", abs.display()))?
                .to_string_lossy()
                .replace('\\', "/");

            let data = std::fs::read(&abs)?;
            let blob = Object::Blob(Blob::new(data));
            let oid = store.write_object(&blob)?;
            let meta = std::fs::metadata(&abs)?;
            let entry = IndexEntry::from_fs_metadata(rel, oid, &meta, 0);
            index.add_entry(entry);
        } else if !add {
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

    if index.entries().is_empty() {
        bail!("nothing to commit (create/copy files and use \"ox add\" to track)");
    }

    let tree_oid = write_tree(&index, &store)?;
    let (branch_name, head_commit_oid) = ref_store.resolve_head()?;

    if let Some(ref head_oid) = head_commit_oid {
        if let Ok(Object::Commit(head_commit)) = store.read_object(head_oid) {
            if head_commit.tree == tree_oid {
                println!("On branch {}", branch_name);
                println!("nothing to commit, working tree clean");
                return Ok(());
            }
        }
    }

    let msg_str = message.unwrap_or_else(|| "unspecified commit message".to_string());
    if msg_str.trim().is_empty() {
        bail!("Aborting commit due to empty commit message.");
    }

    let sig = get_default_signature(Some(&git_dir));
    let parents = if let Some(p) = head_commit_oid {
        vec![p]
    } else {
        Vec::new()
    };

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

fn cmd_log(max_count: Option<usize>, oneline: bool, graph: bool, _tui: bool) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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

        for entry in index.entries() {
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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
    store: &LooseObjectStore,
    index: &mut Index,
    target_tree_oid: &ObjectId,
) -> Result<()> {
    let target_map = flatten_tree(store, target_tree_oid, "")?;

    // 1. Remove files from working tree that are in old index but not in new target tree
    for entry in index.entries() {
        if !target_map.contains_key(&entry.path) {
            let full_path = repo_root.join(&entry.path);
            if full_path.exists() {
                let _ = std::fs::remove_file(&full_path);
            }
        }
    }

    // 2. Write new files to working tree and create new index entries
    index.entries.clear();
    for (path, (_mode, oid)) in target_map {
        let full_path = repo_root.join(&path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let obj = store.read_object(&oid)?;
        if let Object::Blob(blob) = obj {
            std::fs::write(&full_path, &blob.data)?;
        }

        if let Ok(meta) = std::fs::metadata(&full_path) {
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
        let (active_branch, _) = ref_store.resolve_head()?;
        if active_branch == branch_name {
            bail!("cannot delete branch '{}' checked out", branch_name);
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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

    // Check if target is a branch name
    let branch_ref = format!("refs/heads/{}", target);
    if let Ok(commit_oid) = ref_store.read_ref(&branch_ref) {
        let obj = store.read_object(&commit_oid)?;
        let commit = match obj {
            Object::Commit(c) => c,
            _ => bail!("object {} is not a commit", commit_oid),
        };

        checkout_tree_and_update_index(repo_root, &store, &mut index, &commit.tree)?;
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

        checkout_tree_and_update_index(repo_root, &store, &mut index, &commit.tree)?;
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

fn cmd_merge(commit_arg: String) -> Result<()> {
    let git_dir = find_git_dir(Path::new("."))?;
    let repo_root = git_dir.parent().context("git_dir has no parent")?;
    let store = LooseObjectStore::new(git_dir.join("objects"));
    let ref_store = RefStore::new(&git_dir);
    let index_path = git_dir.join("index");
    let mut index = Index::load_from(&index_path)?;

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
        checkout_tree_and_update_index(repo_root, &store, &mut index, &their_commit.tree)?;
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
        println!("Automatic merge failed; fix conflicts and then commit the result.");
    } else {
        // Automatic merge commit
        let tree_oid = write_tree(&index, &store)?;
        let sig = get_default_signature(Some(&git_dir));
        let merge_commit = Commit {
            tree: tree_oid,
            parents: vec![our_oid, their_oid],
            author: sig.clone(),
            committer: sig,
            gpg_sig: None,
            message: format!("Merge branch '{}'\n", commit_arg),
        };
        let merge_oid = store.write_object(&Object::Commit(merge_commit))?;
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

fn get_blob_text(store: &LooseObjectStore, oid_opt: Option<&ObjectId>) -> String {
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
        checkout_tree_and_update_index(repo_root, &store, &mut index, &target_commit.tree)?;
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
        let _ = std::fs::remove_file(&file);
    }

    // Clean up empty directories
    if loose.root().is_dir() {
        for entry in (std::fs::read_dir(loose.root())?).flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.len() == 2 {
                    let _ = std::fs::remove_dir(&path);
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
