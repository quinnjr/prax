//! Schema merging with cross-file collision detection.

use smol_str::SmolStr;

use super::source::{SourceId, SourceLoc};
use crate::ast::Span;

/// A single conflict found while merging two [`Schema`](crate::ast::Schema)s.
///
/// Collected without short-circuiting so the loader can report every duplicate
/// in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeConflict {
    DuplicateModel {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateEnum {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateType {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateView {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateServerGroup {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicatePolicy {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateGenerator {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    DuplicateRawSql {
        name: SmolStr,
        existing: SourceLoc,
        incoming: SourceLoc,
    },
    MultipleDatasource {
        existing: SourceLoc,
        incoming: SourceLoc,
    },
}

/// Build a SourceLoc from any item that has `source_id: Option<SourceId>` and `span: Span`.
///
/// Items whose `source_id` is `None` (i.e., constructed outside the loader,
/// such as in unit tests) get `SourceId(u32::MAX)`.
pub(crate) fn loc(source_id: Option<SourceId>, span: Span) -> SourceLoc {
    SourceLoc::new(source_id.unwrap_or(SourceId(u32::MAX)), span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Schema;
    use crate::loader::{SourceId, stamp_source};
    use crate::parser::parse_schema;

    fn stamped(input: &str, sid: u32) -> Schema {
        let mut s = parse_schema(input).unwrap();
        stamp_source(&mut s, SourceId(sid));
        s
    }

    #[test]
    fn merge_distinct_models_succeeds() {
        let mut a = stamped("model A { id Int @id @auto }", 0);
        let b = stamped("model B { id Int @id @auto }", 1);
        assert!(a.try_merge(b).is_ok());
        assert!(a.get_model("A").is_some());
        assert!(a.get_model("B").is_some());
        assert_eq!(a.get_model("B").unwrap().source_id, Some(SourceId(1)));
    }

    #[test]
    fn merge_duplicate_models_reports_both_locations() {
        let mut a = stamped("model User { id Int @id @auto }", 0);
        let b = stamped("model User { id Int @id @auto }", 1);
        let err = a.try_merge(b).unwrap_err();
        assert_eq!(err.len(), 1);
        match &err[0] {
            MergeConflict::DuplicateModel {
                name,
                existing,
                incoming,
            } => {
                assert_eq!(name.as_str(), "User");
                assert_eq!(existing.source, SourceId(0));
                assert_eq!(incoming.source, SourceId(1));
            }
            other => panic!("unexpected conflict: {other:?}"),
        }
    }

    #[test]
    fn merge_collects_all_conflicts_without_short_circuit() {
        let mut a = stamped(
            "model A { id Int @id @auto } model B { id Int @id @auto }",
            0,
        );
        let b = stamped(
            "model A { id Int @id @auto } model B { id Int @id @auto }",
            1,
        );
        let err = a.try_merge(b).unwrap_err();
        assert_eq!(err.len(), 2);
    }

    #[test]
    fn merge_two_datasources_errors() {
        let mut a = stamped(r#"datasource db { provider = "postgresql" url = "x" }"#, 0);
        let b = stamped(r#"datasource db { provider = "postgresql" url = "y" }"#, 1);
        let err = a.try_merge(b).unwrap_err();
        assert!(matches!(err[0], MergeConflict::MultipleDatasource { .. }));
    }
}
