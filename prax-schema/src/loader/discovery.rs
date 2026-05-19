//! Recursive `*.prax` discovery for multi-file schema directories.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{SchemaError, SchemaResult};

/// A discovered `*.prax` file with its absolute and relative paths.
#[derive(Debug, Clone)]
pub struct Discovered {
    /// Absolute path on disk.
    pub absolute: PathBuf,
    /// Path relative to the discovery root (used for sort order + emit mirroring).
    pub relative: PathBuf,
}

/// Recursively find all `*.prax` files under `root`, sorted lexicographically
/// by the relative path.
///
/// Skipped:
/// - Hidden entries (filename starts with `.`)
/// - Symlinks (not followed)
/// - Any directory named `target`
pub fn discover(root: impl AsRef<Path>) -> SchemaResult<Vec<Discovered>> {
    let root = root.as_ref();
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut out = Vec::new();
    for entry in WalkDir::new(&canonical_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let entry = entry.map_err(|e| SchemaError::IoError {
            path: e
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            source: e
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("walkdir error")),
        })?;

        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("prax") {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(&canonical_root)
            .unwrap_or(entry.path())
            .to_path_buf();

        out.push(Discovered {
            absolute: entry.path().to_path_buf(),
            relative,
        });
    }

    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    // Always allow the root itself.
    if entry.depth() == 0 {
        return false;
    }
    if let Some(name) = entry.file_name().to_str() {
        if name.starts_with('.') {
            return true;
        }
        if entry.file_type().is_dir() && name == "target" {
            return true;
        }
    }
    if entry.file_type().is_symlink() {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, content: &str) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    #[test]
    fn flat_directory_returns_sorted_prax_files() {
        let dir = tempdir().unwrap();
        write(dir.path(), "b.prax", "// b");
        write(dir.path(), "a.prax", "// a");
        write(dir.path(), "c.prax", "// c");

        let found = discover(dir.path()).unwrap();
        let names: Vec<_> = found.iter().map(|d| d.relative.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a.prax", "b.prax", "c.prax"]);
    }

    #[test]
    fn recursive_descent_finds_nested_files() {
        let dir = tempdir().unwrap();
        write(dir.path(), "schema.prax", "// root");
        write(dir.path(), "models/user.prax", "model U {}");
        write(dir.path(), "models/post.prax", "model P {}");
        write(dir.path(), "enums/role.prax", "enum R {}");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 4);
    }

    #[test]
    fn hidden_dirs_are_skipped() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), ".git/HEAD", "// not prax");
        write(dir.path(), ".cache/bad.prax", "// skipped");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relative.to_str().unwrap(), "ok.prax");
    }

    #[test]
    fn target_directory_is_skipped() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), "target/build.prax", "// skipped");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn non_prax_files_ignored() {
        let dir = tempdir().unwrap();
        write(dir.path(), "ok.prax", "// ok");
        write(dir.path(), "README.md", "# readme");
        write(dir.path(), "schema.prisma", "// wrong ext");

        let found = discover(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
    }
}
