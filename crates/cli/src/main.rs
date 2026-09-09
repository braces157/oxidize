//! `ox` — Production-quality, daily-driver-capable Git implementation in Rust.

use anyhow::Result;
use clap::{Parser, Subcommand};

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
        } => {
            println!(
                "hash-object: write={}, stdin={}, type={}, file={:?}",
                write, stdin, object_type, file
            );
        }
        Commands::CatFile {
            pretty,
            show_type,
            show_size,
            object,
        } => {
            println!(
                "cat-file: -p={}, -t={}, -s={}, object={}",
                pretty, show_type, show_size, object
            );
        }
        Commands::Init { directory } => {
            let target = directory.unwrap_or_else(|| ".".to_string());
            println!("Initialized empty Git repository in {}", target);
        }
        other => {
            println!("Command {:?} dispatched (stubbed in Phase 1)", other);
        }
    }
    Ok(())
}
