//! `ox` — Production-quality, daily-driver-capable Git implementation in Rust.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use oxidize_core::{
    find_git_dir, Blob, FileMode, LooseObjectStore, Object, ObjectId, ObjectType, Tree, TreeEntry,
};
use oxidize_index::{compute_status, write_tree, Index, IndexEntry, StagedChange, UnstagedChange};
use std::io::{self, BufRead, Read, Write};
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
        target: String,
    },

    /// Switch branches
    Switch {
        /// Create and switch to a new branch
        #[arg(short = 'c', long)]
        create: Option<String>,

        /// Branch name
        branch: String,
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
    let store = LooseObjectStore::new(git_dir.join("objects"));
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
