//! Multi-file schema loader.
//!
//! See `docs/superpowers/specs/2026-05-19-multi-file-schema-design.md`.

pub mod source;

pub use source::{SourceFile, SourceId, SourceMap};

use crate::ast::Schema;

/// Stamp every top-level item in `schema` with `source`.
///
/// Called by the loader right after parsing a per-file [`Schema`], before merging.
#[allow(dead_code)] // wired into load() in a later task
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
