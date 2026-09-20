//! Finding the folders that hold Markdown.
//!
//! baca looks in the places notes usually live, plus whatever folders the
//! reader has added. It deliberately does not sweep the whole home directory:
//! on a developer's machine that is tens of thousands of folders, most of them
//! source trees rather than anything to read.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Folder names under `$HOME` that usually hold notes. `Downloads` is here
/// because that is where documents land, whatever anyone intends.
const NOTE_HOMES: [&str; 9] = [
    "Documents",
    "Downloads",
    "Notes",
    "notes",
    "Obsidian",
    "vault",
    "Vault",
    "wiki",
    "zettelkasten",
];

/// Names never worth walking into.
const SKIP: [&str; 8] = [
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".git",
    "__pycache__",
    "Trash",
];

/// How deep below a root to look. Deep enough for a nested vault, shallow
/// enough that one stray symlink cannot take the scan on a tour of the disk.
const MAX_DEPTH: usize = 8;

/// An upper bound, so a pathological tree cannot fill the gallery or memory.
const MAX_COLLECTIONS: usize = 500;

/// A folder holding Markdown, as the gallery shows it.
#[derive(Clone)]
pub struct Collection {
    pub path: PathBuf,
    pub name: String,
    pub count: usize,
    /// The most recent edit among its documents, used to order the gallery.
    pub updated: Option<SystemTime>,
}

/// Where baca looks by default: the note folders that exist under `$HOME`.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    NOTE_HOMES
        .iter()
        .map(|name| home.join(name))
        .filter(|p| p.is_dir())
        .collect()
}

/// Every folder under these roots that directly contains Markdown.
///
/// Nesting is kept: a vault with `journal/` and `refs/` shows up as three
/// collections, because that is how its author filed things.
pub fn discover(roots: &[PathBuf]) -> Vec<Collection> {
    let mut found: Vec<Collection> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for root in roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        if seen.iter().any(|s| root.starts_with(s)) {
            continue;
        }
        seen.push(root.clone());
        walk(&root, 0, &mut found);
    }
    // Most recently touched first: the folder you were last working in is
    // almost always the one you want next.
    found.sort_by(|a, b| {
        b.updated
            .cmp(&a.updated)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    found.truncate(MAX_COLLECTIONS);
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<Collection>) {
    if depth > MAX_DEPTH || found.len() >= MAX_COLLECTIONS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut count = 0;
    let mut updated: Option<SystemTime> = None;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || SKIP.contains(&name.as_str()) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            subdirs.push(path);
        } else if crate::is_markdown(&path) {
            count += 1;
            let touched = entry.metadata().and_then(|m| m.modified()).ok();
            updated = updated.max(touched);
        }
    }
    if count > 0 {
        found.push(Collection {
            name: label(dir),
            path: dir.to_path_buf(),
            count,
            updated,
        });
    }
    for sub in subdirs {
        walk(&sub, depth + 1, found);
    }
}

/// A folder's display name. The last component is usually enough, but a bare
/// `docs` says nothing without knowing whose docs they are. A folder actually
/// called `notes` is left alone: that name means something on its own.
fn label(dir: &Path) -> String {
    let last = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string());
    if matches!(
        last.to_lowercase().as_str(),
        "docs" | "doc" | "src" | "content"
    ) {
        if let Some(parent) = dir.parent().and_then(|p| p.file_name()) {
            return format!("{}/{last}", parent.to_string_lossy());
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory tree, removed when the test ends.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("baca-test-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Tree(root)
        }

        fn file(&self, path: &str) -> &Self {
            let full = self.0.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, "# note\n").unwrap();
            self
        }

        fn dir(&self, path: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(path)).unwrap();
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(found: &[Collection]) -> Vec<String> {
        let mut names: Vec<String> = found.iter().map(|c| c.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn every_folder_holding_markdown_becomes_a_collection() {
        let tree = Tree::new("nested");
        tree.file("a.md")
            .file("journal/b.md")
            .file("journal/deep/c.md");
        let found = discover(&[tree.0.clone()]);
        assert_eq!(found.len(), 3, "each level files its own notes");
        assert!(names(&found).contains(&"journal".to_string()));
    }

    #[test]
    fn folders_without_markdown_are_left_out() {
        let tree = Tree::new("empty");
        tree.file("notes/a.md").dir("pictures").dir("empty");
        let found = discover(&[tree.0.clone()]);
        assert_eq!(names(&found), vec!["notes"]);
    }

    #[test]
    fn noise_directories_are_never_walked() {
        let tree = Tree::new("noise");
        tree.file("real/a.md")
            .file("node_modules/pkg/readme.md")
            .file("target/doc/x.md")
            .file(".git/notes.md");
        let found = discover(&[tree.0.clone()]);
        assert_eq!(names(&found), vec!["real"]);
    }

    #[test]
    fn a_root_inside_another_root_is_not_scanned_twice() {
        let tree = Tree::new("overlap");
        tree.file("vault/a.md");
        let found = discover(&[tree.0.clone(), tree.0.join("vault")]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_generic_folder_name_is_qualified_by_its_parent() {
        assert_eq!(label(Path::new("/home/x/omarchy/docs")), "omarchy/docs");
        assert_eq!(label(Path::new("/home/x/omarchy/Docs")), "omarchy/Docs");
        // A folder named for what it holds stands on its own.
        assert_eq!(label(Path::new("/home/x/Notes")), "Notes");
        assert_eq!(label(Path::new("/home/x/notes")), "notes");
    }

    #[test]
    fn collections_are_counted() {
        let tree = Tree::new("count");
        tree.file("notes/a.md")
            .file("notes/b.md")
            .file("notes/c.md");
        let found = discover(&[tree.0.clone()]);
        assert_eq!(found[0].count, 3);
    }
}
