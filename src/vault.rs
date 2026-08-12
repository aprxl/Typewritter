//! The vault: note files on disk, walked lazily.
//!
//! Nothing is read recursively. Only the entries of expanded directories
//! are loaded, one `read_dir` per folder per first expansion, so opening a
//! vault costs one directory listing no matter how deep it is.

use std::fs;
use std::path::{Path, PathBuf};

/// One directory entry.
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    expanded: bool,
    loaded: bool,
    children: Vec<Node>,
}

/// A visible row of the tree, in display order (depth-first, expanded
/// folders above their children).
pub struct Row<'a> {
    pub name: &'a str,
    pub path: &'a Path,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultFile {
    pub name: String,
    pub path: PathBuf,
}

pub struct Vault {
    root: PathBuf,
    children: Vec<Node>,
}

impl Vault {
    /// Opens the vault and reads its top level. `None` when the directory
    /// is missing or unreadable — the shell shows a hint instead of a tree.
    pub fn open(root: &Path) -> Option<Vault> {
        let children = read_dir(root)?;
        Some(Vault {
            root: root.to_path_buf(),
            children,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Expands or collapses `dir`, reading its children on first expansion.
    pub fn toggle(&mut self, dir: &Path) {
        let Some(node) = find_mut(&mut self.children, dir) else {
            return;
        };
        if !node.is_dir {
            return;
        }
        if !node.loaded {
            node.children = read_dir(dir).unwrap_or_default();
            node.loaded = true;
        }
        node.expanded = !node.expanded;
    }

    /// Rows in display order. Allocated per call; the tree is small.
    pub fn visible(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        walk(&self.children, 0, &mut rows);
        rows
    }

    /// Returns every visible regular file, recursively, without changing tree
    /// expansion state. Source order is the same directory-first alphabetical
    /// order used by the tree.
    pub fn files(&self) -> Vec<VaultFile> {
        let mut files = Vec::new();
        collect_files(&self.root, &self.root, &mut files);
        files
    }

    /// Removes `path` (file or folder) from the tree. Returns whether it
    /// was found.
    pub fn remove(&mut self, path: &Path) -> bool {
        remove_from(&mut self.children, path)
    }

    /// Re-reads the tree preserving expansion state: expanded folders stay
    /// expanded and their children are re-read too; collapsed folders stay
    /// lazy (children not loaded).
    pub fn refresh(&mut self) {
        if let Some(mut children) = read_dir(&self.root) {
            merge(&mut children, &self.children);
            self.children = children;
        }
    }

    /// Expands every ancestor of `path` inside the vault so a newly created
    /// file or folder is visible in the tree. Loads children on the way down.
    pub fn reveal(&mut self, path: &Path) {
        let relative = match path.strip_prefix(&self.root) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut current = self.root.clone();
        let mut nodes = &mut self.children;
        for component in relative.components() {
            let next = current.join(component);
            // Find the node at this level that matches.
            let Some(pos) = nodes.iter().position(|n| n.path == next) else {
                return;
            };
            let node = &mut nodes[pos];
            if node.is_dir {
                if !node.loaded {
                    node.children = read_dir(&node.path).unwrap_or_default();
                    node.loaded = true;
                }
                node.expanded = true;
                current = next;
                nodes = &mut node.children;
            } else {
                // File — stop descending.
                break;
            }
        }
    }
}

/// Re-reads children of every fresh node whose matching old node was
/// expanded, preserving expanded/loaded state recursively.
fn merge(fresh: &mut [Node], old: &[Node]) {
    for f in fresh.iter_mut() {
        if let Some(o) = old.iter().find(|o| o.path == f.path)
            && o.expanded
        {
            f.expanded = true;
            f.loaded = true;
            if let Some(mut children) = read_dir(&f.path) {
                merge(&mut children, &o.children);
                f.children = children;
            }
        }
    }
}

fn remove_from(nodes: &mut Vec<Node>, path: &Path) -> bool {
    if let Some(i) = nodes.iter().position(|n| n.path == path) {
        nodes.remove(i);
        return true;
    }
    for node in nodes.iter_mut() {
        if remove_from(&mut node.children, path) {
            return true;
        }
    }
    false
}

fn walk<'a>(nodes: &'a [Node], depth: usize, rows: &mut Vec<Row<'a>>) {
    for node in nodes {
        rows.push(Row {
            name: &node.name,
            path: &node.path,
            depth,
            is_dir: node.is_dir,
            expanded: node.expanded,
        });
        if node.expanded {
            walk(&node.children, depth + 1, rows);
        }
    }
}

fn find_mut<'a>(nodes: &'a mut [Node], path: &Path) -> Option<&'a mut Node> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if let Some(found) = find_mut(&mut node.children, path) {
            return Some(found);
        }
    }
    None
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<VaultFile>) {
    for node in read_dir(dir).unwrap_or_default() {
        if node.is_dir {
            collect_files(root, &node.path, files);
        } else if let Ok(relative) = node.path.strip_prefix(root) {
            files.push(VaultFile {
                name: relative.to_string_lossy().into_owned(),
                path: node.path,
            });
        }
    }
}

/// Lists a directory's entries: directories first, then files, each
/// alphabetically. Hidden entries (dotfiles, `.git`) are skipped.
fn read_dir(path: &Path) -> Option<Vec<Node>> {
    let mut nodes: Vec<Node> = fs::read_dir(path)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let kind = entry.file_type().ok()?;
            if !(kind.is_dir() || kind.is_file()) {
                return None;
            }
            Some(Node {
                name,
                path: entry.path(),
                is_dir: kind.is_dir(),
                expanded: false,
                loaded: false,
                children: Vec::new(),
            })
        })
        .collect();
    nodes.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Some(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tw-vault-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("math")).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("alpha.md"), "").unwrap();
        fs::write(root.join("zeta.md"), "").unwrap();
        fs::write(root.join(".hidden.md"), "").unwrap();
        fs::write(root.join("math/frac.md"), "").unwrap();
        fs::write(root.join("math/delta.md"), "").unwrap();
        root
    }

    #[test]
    fn open_reads_top_level_only_dirs_first_alphabetical() {
        let root = temp_vault("top");
        let vault = Vault::open(&root).unwrap();

        let names: Vec<&str> = vault.visible().iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            vec!["math", "notes", "alpha.md", "zeta.md"],
            "hidden skipped, dirs before files, each alphabetical"
        );
        assert!(
            vault.visible().iter().all(|r| r.depth == 0 && !r.expanded),
            "nothing expanded, nothing loaded deeper"
        );
    }

    #[test]
    fn expanding_loads_children_once_and_collapsing_hides_them() {
        let root = temp_vault("expand");
        let mut vault = Vault::open(&root).unwrap();
        let math = root.join("math");

        vault.toggle(&math);
        let first: Vec<&str> = vault.visible().iter().map(|r| r.name).collect();
        assert_eq!(
            first,
            vec![
                "math", "delta.md", "frac.md", "notes", "alpha.md", "zeta.md"
            ],
            "children appear under the expanded folder, depth-first"
        );
        let depths: Vec<usize> = vault.visible().iter().map(|r| r.depth).collect();
        assert_eq!(depths[1], 1);
        assert_eq!(depths[2], 1);

        vault.toggle(&math);
        assert_eq!(vault.visible().len(), 4, "collapsing hides the children");

        vault.toggle(&math);
        assert_eq!(
            vault.visible().len(),
            6,
            "re-expanding shows the children still loaded, not re-read"
        );
    }

    #[test]
    fn toggling_a_file_does_nothing() {
        let root = temp_vault("file");
        let mut vault = Vault::open(&root).unwrap();
        let before = vault.visible().len();

        vault.toggle(&root.join("alpha.md"));
        assert_eq!(vault.visible().len(), before);
    }

    #[test]
    fn remove_prunes_depth_first_and_refresh_sees_new_files() {
        let root = temp_vault("rm");
        let mut vault = Vault::open(&root).unwrap();
        vault.toggle(&root.join("math"));

        assert!(vault.remove(&root.join("math/frac.md")));
        assert!(!vault.remove(&root.join("math/frac.md")), "already gone");
        let names: Vec<&str> = vault.visible().iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            vec!["math", "delta.md", "notes", "alpha.md", "zeta.md"]
        );

        // A pile-up only happens if the internal forest drifts from disk,
        // which a refresh is exactly for.
        fs::write(root.join("fresh.md"), "").unwrap();
        vault.refresh();
        assert!(
            vault.visible().iter().any(|r| r.name == "fresh.md"),
            "refresh picks up newly created files"
        );
    }

    #[test]
    fn files_lists_regular_files_recursively_and_skips_hidden_entries() {
        let root = temp_vault("files");
        fs::write(root.join("math/.hidden.md"), "").unwrap();
        let vault = Vault::open(&root).unwrap();
        let files = vault.files();
        let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["math/delta.md", "math/frac.md", "alpha.md", "zeta.md"]
        );
    }

    #[test]
    fn refresh_keeps_expanded_folders_open() {
        let root = temp_vault("refresh-expand");
        let mut vault = Vault::open(&root).unwrap();
        vault.toggle(&root.join("math"));
        assert!(vault.visible().iter().any(|r| r.name == "delta.md"));

        fs::write(root.join("math/new.md"), "").unwrap();
        vault.refresh();
        let names: Vec<&str> = vault.visible().iter().map(|r| r.name).collect();
        assert!(names.contains(&"math"), "math folder still visible");
        assert!(names.contains(&"new.md"), "new file appears");
        assert!(names.contains(&"delta.md"), "existing child preserved");
    }

    #[test]
    fn reveal_expands_ancestors() {
        let root = temp_vault("reveal");
        let mut vault = Vault::open(&root).unwrap();
        fs::create_dir_all(root.join("notes/deep")).unwrap();
        fs::write(root.join("notes/deep/x.md"), "").unwrap();
        vault.refresh();

        vault.reveal(&root.join("notes/deep/x.md"));
        let names: Vec<&str> = vault.visible().iter().map(|r| r.name).collect();
        assert!(names.contains(&"deep"), "deep folder visible");
        assert!(names.contains(&"x.md"), "file visible");
    }
}
