//! Multi-file Prisma schema discovery, parse, and merge.
//!
//! Mirrors the design in `docs/superpowers/specs/2026-05-19-multi-file-schema-design.md`.
//! Prisma's `prismaSchemaFolder` mode places multiple `*.prisma` files in one
//! directory and treats them as a single logical schema. This module walks
//! such a directory, parses each file, stamps provenance, and merges into a
//! single [`PrismaSchema`] with conflict detection.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::parser::parse_prisma_schema;
use super::types::{PrismaSchema, PrismaSourceId};
use crate::error::{ImportError, ImportResult};

/// A discovered `.prisma` file with absolute + relative paths.
#[derive(Debug, Clone)]
pub struct PrismaFile {
    pub absolute: PathBuf,
    pub relative: PathBuf,
}

/// Map of [`PrismaSourceId`] -> [`PrismaFile`], built during multi-file import.
#[derive(Debug, Default)]
pub struct PrismaSourceMap {
    files: Vec<PrismaFile>,
}

impl PrismaSourceMap {
    pub fn get(&self, id: PrismaSourceId) -> Option<&PrismaFile> {
        self.files.get(id.0 as usize)
    }
    pub fn iter(&self) -> impl Iterator<Item = (PrismaSourceId, &PrismaFile)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, f)| (PrismaSourceId(i as u32), f))
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Discover all `*.prisma` files under `root`, sorted lexicographically by
/// relative path.
///
/// Skips hidden entries, symlinks, and `node_modules/` directories. (We
/// deliberately do not reuse the loader's `target/` skip — the importer's
/// skip set is Prisma-ecosystem-specific.)
pub fn discover_prisma(root: &Path) -> ImportResult<Vec<PrismaFile>> {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for entry in WalkDir::new(&canonical)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped(e))
    {
        let entry = entry.map_err(|e| {
            ImportError::IoError(
                e.into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("walkdir error")),
            )
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("prisma") {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&canonical)
            .unwrap_or(entry.path())
            .to_path_buf();
        out.push(PrismaFile {
            absolute: entry.path().to_path_buf(),
            relative,
        });
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

fn is_skipped(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if let Some(name) = entry.file_name().to_str() {
        if name.starts_with('.') {
            return true;
        }
        if entry.file_type().is_dir() && name == "node_modules" {
            return true;
        }
    }
    if entry.file_type().is_symlink() {
        return true;
    }
    false
}

/// Parse every `.prisma` file under `root`, stamp source provenance, and
/// merge into one [`PrismaSchema`]. Returns the merged schema and the source
/// map; on conflict, returns an error.
pub fn parse_and_merge_directory(root: &Path) -> ImportResult<(PrismaSchema, PrismaSourceMap)> {
    let files = discover_prisma(root)?;
    if files.is_empty() {
        return Err(ImportError::EmptyPrismaDirectory {
            path: root.to_path_buf(),
        });
    }

    let mut sources = PrismaSourceMap {
        files: files.clone(),
    };
    let mut merged = PrismaSchema::default();

    for (idx, f) in files.iter().enumerate() {
        let sid = PrismaSourceId(idx as u32);
        let content = std::fs::read_to_string(&f.absolute)?;
        let mut per_file = parse_prisma_schema(&content)?;
        stamp_prisma(&mut per_file, sid);
        try_merge_prisma(&mut merged, per_file, sid)?;
    }

    let _ = &mut sources; // sources is built in-place above; suppress unused-mut if any
    Ok((merged, sources))
}

fn stamp_prisma(s: &mut PrismaSchema, sid: PrismaSourceId) {
    for m in &mut s.models {
        m.source_id = Some(sid);
    }
    for e in &mut s.enums {
        e.source_id = Some(sid);
    }
    if let Some(ds) = &mut s.datasource {
        ds.source_id = Some(sid);
    }
}

fn try_merge_prisma(
    into: &mut PrismaSchema,
    other: PrismaSchema,
    incoming_sid: PrismaSourceId,
) -> ImportResult<()> {
    for m in other.models {
        if let Some(existing) = into.models.iter().find(|x| x.name == m.name) {
            return Err(ImportError::DuplicatePrismaModel {
                name: m.name.clone(),
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.models.push(m);
    }
    for e in other.enums {
        if let Some(existing) = into.enums.iter().find(|x| x.name == e.name) {
            return Err(ImportError::DuplicatePrismaEnum {
                name: e.name.clone(),
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.enums.push(e);
    }
    if let Some(incoming) = other.datasource {
        if let Some(existing) = &into.datasource {
            return Err(ImportError::MultiplePrismaDatasource {
                first: existing.source_id.unwrap_or(PrismaSourceId(0)),
                second: incoming_sid,
            });
        }
        into.datasource = Some(incoming);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_and_merges_directory() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.prisma"),
            r#"datasource db { provider = "postgresql" url = env("DB_URL") }

model A {
  id Int @id @default(autoincrement())
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.prisma"),
            "model B { id Int @id @default(autoincrement()) }",
        )
        .unwrap();

        let (merged, sources) = parse_and_merge_directory(dir.path()).unwrap();
        assert_eq!(merged.models.len(), 2);
        assert!(merged.datasource.is_some());
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn duplicate_models_error() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.prisma"),
            "model X { id Int @id @default(autoincrement()) }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.prisma"),
            "model X { id Int @id @default(autoincrement()) }",
        )
        .unwrap();

        let err = parse_and_merge_directory(dir.path()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("duplicate") && msg.contains("X"), "got: {msg}");
    }

    #[test]
    fn empty_dir_errors() {
        let dir = tempdir().unwrap();
        let err = parse_and_merge_directory(dir.path()).unwrap_err();
        assert!(matches!(err, ImportError::EmptyPrismaDirectory { .. }));
    }
}
