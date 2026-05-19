//! Multi-file schema loader.

mod discovery;
pub(crate) mod merge;
mod source;

use std::path::Path;

pub use discovery::{Discovered, discover};
pub use merge::MergeConflict;
pub use source::{SourceFile, SourceId, SourceLoc, SourceMap};

use crate::ast::Schema;
use crate::error::SchemaError;
use crate::parser::parse_schema;
use crate::validator::Validator;

/// A successfully loaded multi-file (or single-file) schema, paired with the
/// source map needed for downstream diagnostics rendering.
#[derive(Debug, Clone)]
pub struct LoadedSchema {
    pub schema: Schema,
    pub sources: SourceMap,
}

/// Error returned by [`load`], carrying the partial source map built up to the
/// point of failure so the renderer can resolve [`SourceId`]s back to file
/// content.
#[derive(Debug)]
pub struct LoadError {
    pub error: SchemaError,
    pub sources: SourceMap,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Load a schema from a file or directory.
///
/// - If `path` is a file: parse the single file.
/// - If `path` is a directory: recursively find `*.prax`, parse each, merge
///   with collision detection, then validate the merged AST.
pub fn load(path: impl AsRef<Path>) -> Result<LoadedSchema, LoadError> {
    let path = path.as_ref();
    let meta = std::fs::metadata(path).map_err(|e| LoadError {
        error: SchemaError::IoError {
            path: path.display().to_string(),
            source: e,
        },
        sources: SourceMap::new(),
    })?;

    if meta.is_file() {
        load_single(path)
    } else if meta.is_dir() {
        load_directory(path)
    } else {
        Err(LoadError {
            error: SchemaError::ConfigError {
                message: format!(
                    "schema path `{}` is neither a file nor a directory",
                    path.display()
                ),
            },
            sources: SourceMap::new(),
        })
    }
}

fn load_single(path: &Path) -> Result<LoadedSchema, LoadError> {
    let mut sources = SourceMap::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return Err(LoadError {
                error: SchemaError::IoError {
                    path: path.display().to_string(),
                    source: e,
                },
                sources,
            });
        }
    };

    let mut schema = match parse_schema(&content) {
        Ok(s) => s,
        Err(e) => {
            // Insert into the map before returning so the renderer can resolve
            // SourceId(0) back to file content.
            sources.insert(path.to_path_buf(), content);
            return Err(LoadError { error: e, sources });
        }
    };
    let sid = sources.insert(path.to_path_buf(), content);
    stamp_source(&mut schema, sid);

    let validated = match Validator::new().validate(schema) {
        Ok(s) => s,
        Err(e) => return Err(LoadError { error: e, sources }),
    };

    Ok(LoadedSchema {
        schema: validated,
        sources,
    })
}

fn load_directory(root: &Path) -> Result<LoadedSchema, LoadError> {
    let mut sources = SourceMap::new();

    let files = match discovery::discover(root) {
        Ok(v) => v,
        Err(e) => return Err(LoadError { error: e, sources }),
    };

    if files.is_empty() {
        return Err(LoadError {
            error: SchemaError::EmptySchemaDirectory {
                path: root.to_path_buf(),
            },
            sources,
        });
    }

    let mut per_file: Vec<(SourceId, Schema)> = Vec::with_capacity(files.len());
    for f in files {
        let content = match std::fs::read_to_string(&f.absolute) {
            Ok(c) => c,
            Err(e) => {
                return Err(LoadError {
                    error: SchemaError::IoError {
                        path: f.absolute.display().to_string(),
                        source: e,
                    },
                    sources,
                });
            }
        };
        let sid = sources.insert(f.absolute, content);
        // Borrow content back through the map; per-file syntax errors are
        // fail-fast (no useful partial schema if file N of M is malformed).
        let file_content = &sources.get(sid).expect("just inserted").content;
        let mut schema_i = match parse_schema(file_content) {
            Ok(s) => s,
            Err(inner) => {
                return Err(LoadError {
                    error: SchemaError::ParseInFile {
                        source: sid,
                        inner: Box::new(inner),
                    },
                    sources,
                });
            }
        };
        stamp_source(&mut schema_i, sid);
        per_file.push((sid, schema_i));
    }

    let mut merged = Schema::new();
    let mut all_conflicts: Vec<MergeConflict> = Vec::new();
    for (_, schema_i) in per_file {
        if let Err(conflicts) = merged.try_merge(schema_i) {
            all_conflicts.extend(conflicts);
        }
    }

    if !all_conflicts.is_empty() {
        return Err(LoadError {
            error: from_conflicts(all_conflicts),
            sources,
        });
    }

    let validated = match Validator::new().validate(merged) {
        Ok(s) => s,
        Err(e) => return Err(LoadError { error: e, sources }),
    };

    Ok(LoadedSchema {
        schema: validated,
        sources,
    })
}

/// Bundle a batch of [`MergeConflict`]s into a single [`SchemaError`].
fn from_conflicts(conflicts: Vec<MergeConflict>) -> SchemaError {
    let mut errors: Vec<SchemaError> = conflicts.into_iter().map(conflict_to_error).collect();
    if errors.len() == 1 {
        errors.remove(0)
    } else {
        SchemaError::ValidationFailed {
            count: errors.len(),
            errors,
        }
    }
}

fn conflict_to_error(c: MergeConflict) -> SchemaError {
    use crate::error::DuplicateKind;

    macro_rules! dispatch {
        ($($variant:ident => $kind:ident),+ $(,)?) => {
            match c {
                $(
                    MergeConflict::$variant { name, existing, incoming } => {
                        SchemaError::DuplicateAcrossFiles {
                            kind: DuplicateKind::$kind,
                            name: name.to_string(),
                            first: existing,
                            second: incoming,
                        }
                    }
                ),+,
                MergeConflict::MultipleDatasource { existing, incoming } => {
                    SchemaError::MultipleDatasource {
                        first: existing,
                        second: incoming,
                    }
                }
            }
        };
    }

    dispatch! {
        DuplicateModel => Model,
        DuplicateEnum => Enum,
        DuplicateType => Type,
        DuplicateView => View,
        DuplicateServerGroup => ServerGroup,
        DuplicatePolicy => Policy,
        DuplicateGenerator => Generator,
        DuplicateRawSql => RawSql,
    }
}

/// Stamp every top-level item in `schema` with `source`.
///
/// Called by [`load`] right after parsing a per-file [`Schema`], before merging.
pub(crate) fn stamp_source(schema: &mut Schema, source: SourceId) {
    for m in schema.models.values_mut() {
        m.source_id = Some(source);
    }
    for e in schema.enums.values_mut() {
        e.source_id = Some(source);
    }
    for t in schema.types.values_mut() {
        t.source_id = Some(source);
    }
    for v in schema.views.values_mut() {
        v.source_id = Some(source);
    }
    for sg in schema.server_groups.values_mut() {
        sg.source_id = Some(source);
    }
    for p in &mut schema.policies {
        p.source_id = Some(source);
    }
    for g in schema.generators.values_mut() {
        g.source_id = Some(source);
    }
    if let Some(ds) = &mut schema.datasource {
        ds.source_id = Some(source);
    }
    for r in &mut schema.raw_sql {
        r.source_id = Some(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_schema;

    #[test]
    fn load_directory_merges_files_and_resolves_cross_file_relations() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("datasource.prax"),
            r#"datasource db { provider = "postgresql" url = "x" }"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(
            dir.path().join("models/user.prax"),
            "model User { id Int @id @auto email String @unique posts Post[] }",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("models/post.prax"),
            "model Post { id Int @id @auto author_id Int author User @relation(fields: [author_id], references: [id]) }",
        )
        .unwrap();

        let loaded = load(dir.path()).expect("load should succeed");
        assert!(loaded.schema.get_model("User").is_some());
        assert!(loaded.schema.get_model("Post").is_some());
        assert!(loaded.schema.datasource.is_some());
        assert_eq!(loaded.sources.len(), 3);
    }

    #[test]
    fn load_directory_duplicate_model_errors() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.prax"), "model User { id Int @id @auto }").unwrap();
        std::fs::write(dir.path().join("b.prax"), "model User { id Int @id @auto }").unwrap();

        let err = load(dir.path()).unwrap_err();
        let msg = format!("{}", err.error);
        assert!(msg.contains("duplicate model"), "got: {msg}");
        assert_eq!(err.sources.len(), 2);
    }

    #[test]
    fn load_empty_directory_errors() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(matches!(
            err.error,
            crate::error::SchemaError::EmptySchemaDirectory { .. }
        ));
    }

    #[test]
    fn stamp_marks_all_items() {
        let mut schema = parse_schema(
            r#"
            datasource db { provider = "postgresql" url = "x" }
            generator client { provider = "prax-client" }
            enum Role { User Admin }
            model User { id Int @id @auto role Role }
            "#,
        )
        .unwrap();
        stamp_source(&mut schema, SourceId(7));
        assert_eq!(schema.models["User"].source_id, Some(SourceId(7)));
        assert_eq!(schema.enums["Role"].source_id, Some(SourceId(7)));
        assert_eq!(schema.datasource.unwrap().source_id, Some(SourceId(7)));
        assert_eq!(schema.generators["client"].source_id, Some(SourceId(7)));
    }
}
