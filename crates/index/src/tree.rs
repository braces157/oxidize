//! Conversion between Git Index entries and hierarchical Tree objects (`write-tree`).

use crate::index::Index;
use crate::IndexError;
use oxidize_core::id::ObjectId;
use oxidize_core::object::{FileMode, Object, Tree, TreeEntry};
use oxidize_core::store::LooseObjectStore;
use std::collections::BTreeMap;

#[derive(Default)]
struct TreeNode {
    mode: Option<FileMode>,
    oid: Option<ObjectId>,
    children: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    fn insert(&mut self, path_parts: &[&str], mode: FileMode, oid: ObjectId) {
        if path_parts.is_empty() {
            return;
        }

        if path_parts.len() == 1 {
            let name = path_parts[0].to_string();
            let leaf = TreeNode {
                mode: Some(mode),
                oid: Some(oid),
                children: BTreeMap::new(),
            };
            self.children.insert(name, leaf);
        } else {
            let dir_name = path_parts[0];
            let dir_node = self.children.entry(dir_name.to_string()).or_default();
            dir_node.insert(&path_parts[1..], mode, oid);
        }
    }

    fn write_to_store(&self, store: &LooseObjectStore) -> Result<ObjectId, IndexError> {
        let mut entries = Vec::new();

        for (name, child) in &self.children {
            if child.children.is_empty() {
                // File entry
                let mode = child.mode.unwrap_or(FileMode::REGULAR);
                let oid = child.oid.unwrap_or(ObjectId::ZERO);
                entries.push(TreeEntry {
                    mode,
                    name: name.clone(),
                    id: oid,
                });
            } else {
                // Subdirectory subtree entry
                let subtree_oid = child.write_to_store(store)?;
                entries.push(TreeEntry {
                    mode: FileMode::TREE,
                    name: name.clone(),
                    id: subtree_oid,
                });
            }
        }

        let tree = Tree::new(entries);
        let obj = Object::Tree(tree);
        let oid = store.write_object(&obj)?;
        Ok(oid)
    }
}

/// Builds hierarchical Tree objects from the current index and writes them to the object store.
/// Returns the root `ObjectId`.
pub fn write_tree(index: &Index, store: &LooseObjectStore) -> Result<ObjectId, IndexError> {
    let mut root = TreeNode::default();

    for entry in index.entries() {
        let parts: Vec<&str> = entry.path.split('/').filter(|p| !p.is_empty()).collect();
        root.insert(&parts, FileMode(entry.mode), entry.oid);
    }

    root.write_to_store(store)
}
