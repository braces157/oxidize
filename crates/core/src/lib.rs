//! Core Git object model, content-addressable storage, and SHA-1 identity.

pub mod error;
pub mod id;
pub mod lock;
pub mod object;
pub mod path;
pub mod store;

pub use error::CoreError;
pub use id::ObjectId;
pub use lock::LockFile;
pub use object::{Blob, Commit, FileMode, Object, ObjectType, Signature, Tag, Tree, TreeEntry};
pub use path::{
    safe_join, strip_verbatim_prefix, validate_branch_name, validate_ref_name, validate_repo_path,
    validate_tree_component, RepoPath,
};
pub use store::{find_git_dir, LooseObjectStore, ObjectReader, RepoContext};
