//! Core Git object model, content-addressable storage, and SHA-1 identity.

pub mod error;
pub mod id;
pub mod object;
pub mod store;

pub use error::CoreError;
pub use id::ObjectId;
pub use object::{Blob, Commit, FileMode, Object, ObjectType, Signature, Tag, Tree, TreeEntry};
pub use store::{find_git_dir, LooseObjectStore, ObjectReader};
