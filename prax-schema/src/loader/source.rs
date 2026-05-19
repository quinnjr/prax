//! Source provenance tracking for multi-file schemas.

use std::path::{Path, PathBuf};

/// Opaque, dense identifier for a source file in a [`SourceMap`].
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct SourceId(pub u32);

/// A single source file (path + content) loaded into the schema.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    pub content: String,
}

/// Map of [`SourceId`] -> [`SourceFile`].
///
/// Built incrementally during loading. Empty by default.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new source file and return its [`SourceId`].
    pub fn insert(&mut self, path: impl Into<PathBuf>, content: impl Into<String>) -> SourceId {
        let id = SourceId(self.files.len() as u32);
        self.files.push(SourceFile {
            path: path.into(),
            content: content.into(),
        });
        id
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (SourceId, &SourceFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (SourceId(i as u32), f))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Convenience: path for a given id.
    pub fn path_of(&self, id: SourceId) -> Option<&Path> {
        self.get(id).map(|f| f.path.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_assigns_monotonic_ids() {
        let mut map = SourceMap::new();
        let a = map.insert("a.prax", "model A {}");
        let b = map.insert("b.prax", "model B {}");
        assert_eq!(a, SourceId(0));
        assert_eq!(b, SourceId(1));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn get_returns_inserted_file() {
        let mut map = SourceMap::new();
        let id = map.insert("/tmp/x.prax", "content");
        let f = map.get(id).unwrap();
        assert_eq!(f.path.to_str().unwrap(), "/tmp/x.prax");
        assert_eq!(f.content, "content");
    }

    #[test]
    fn get_unknown_id_returns_none() {
        let map = SourceMap::new();
        assert!(map.get(SourceId(42)).is_none());
    }
}
