//! Traits implemented by per-model generated input types.
//!
//! Each trait has one method, `into_ir`, that lowers the input to the
//! runtime IR that the SQL builders already consume. The associated
//! `Model` type keeps generic bounds tight: a `FindManyOperation<E, User>`
//! can only accept a `WhereInput<Model = User>`, never a `PostWhereInput`.

use crate::filter::Filter;
use crate::pagination::Pagination;
use crate::relations::Include;
use crate::traits::Model;
use crate::types::{OrderBy, Select};

/// A typed shape that lowers to a runtime [`Filter`].
///
/// Implemented by per-model `UserWhereInput`, `PostWhereInput`, ...
///
/// # Warning: `Default::default()` lowers to `Filter::None`
///
/// A `*WhereInput` constructed via `Default::default()` (no fields set)
/// produces `Filter::None`, which lowers to `WHERE TRUE` — i.e. matches
/// every row. Passing such a filter to `delete_many` or `update_many`
/// affects every row in the table. Codegen never refuses this at
/// compile time; if a `delete_many` / `update_many` call site needs a
/// non-empty filter, it is the caller's responsibility to verify the
/// `Filter::None` case before invoking `.exec()`.
pub trait WhereInput {
    /// The model this WHERE shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    ///
    /// Returns `Filter::None` when no fields are set. See the trait-level
    /// note about the match-all behavior of `Filter::None`.
    fn into_ir(self) -> Filter;
}

/// A WHERE shape constrained to a unique key (PK or `@unique` column).
///
/// Used by `find_unique` / `update` / `upsert` / `delete` where the
/// operation requires the filter to identify at most one row.
pub trait WhereUniqueInput {
    /// The model this WHERE shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Filter;
}

/// A typed shape that lowers to an [`Include`] specification.
pub trait IncludeInput {
    /// The model this include shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Include;
}

/// A typed shape that lowers to a [`Select`] specification.
pub trait SelectInput {
    /// The model this select shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Select;
}

/// A typed shape that lowers to an [`OrderBy`] specification.
pub trait OrderByInput {
    /// The model this order shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> OrderBy;
}

/// A typed shape that lowers to the `Data` payload for a `create`.
///
/// The associated `Data` type is the existing `<Model as CreateData>::Data`
/// from `prax_query::traits::CreateData` — phase 5 will introduce a
/// `NestedWritePlan` lowering path; phase 1 keeps the lowering simple.
pub trait CreateInput {
    /// The model this create input applies to.
    type Model: Model;
    /// The runtime payload type.
    type Data: Send + Sync;
    /// Lower this input to the runtime payload.
    fn into_ir(self) -> Self::Data;
}

/// A typed shape that lowers to the `Data` payload for an `update`.
pub trait UpdateInput {
    /// The model this update input applies to.
    type Model: Model;
    /// The runtime payload type.
    type Data: Send + Sync;
    /// Lower this input to the runtime payload.
    fn into_ir(self) -> Self::Data;
}

/// A typed shape that lowers to a `_count` aggregate selection.
pub trait CountSelect {
    /// The model this count selection applies to.
    type Model: Model;
    /// Concrete representation as a list of relation names to count.
    fn into_relation_names(self) -> Vec<String>;
}

/// A typed shape that lowers to an aggregate spec
/// (`_count` / `_avg` / `_sum` / `_min` / `_max`).
///
/// The IR target for this trait is finalized in phase 6 when aggregate
/// macros are wired up. For phase 1 the trait only carries the `Model`
/// associated type.
pub trait AggregateInput {
    /// The model this aggregate spec applies to.
    type Model: Model;
}

/// A typed shape that lowers to a group-by spec.
///
/// As with [`AggregateInput`], the IR target is finalized in phase 6.
pub trait GroupByInput {
    /// The model this group-by spec applies to.
    type Model: Model;
}

/// Pagination fragment shared by every read input.
///
/// Phase 1 keeps pagination on the operation itself (matching the
/// current builder API). This struct exists so phase 3+ macros can
/// surface `skip`/`take`/`cursor` inside the input AST without having
/// to construct an entire `*Args`.
#[derive(Debug, Clone, Default)]
pub struct PaginationInput {
    /// Number of rows to skip.
    pub skip: Option<u64>,
    /// Number of rows to take.
    pub take: Option<u64>,
}

impl From<PaginationInput> for Pagination {
    fn from(p: PaginationInput) -> Self {
        let mut out = Pagination::new();
        if let Some(n) = p.skip {
            out = out.skip(n);
        }
        if let Some(n) = p.take {
            out = out.take(n);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestModel;
    impl Model for TestModel {
        const MODEL_NAME: &'static str = "TestModel";
        const TABLE_NAME: &'static str = "test_models";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id"];
    }

    struct TestWhere;
    impl WhereInput for TestWhere {
        type Model = TestModel;
        fn into_ir(self) -> Filter {
            Filter::None
        }
    }

    #[test]
    fn where_input_lowers_to_filter_none() {
        assert!(matches!(TestWhere.into_ir(), Filter::None));
    }

    #[test]
    fn pagination_input_roundtrip() {
        let p = PaginationInput {
            skip: Some(5),
            take: Some(10),
        };
        let raw: Pagination = p.into();
        assert_eq!(raw.skip, Some(5));
        assert_eq!(raw.take, Some(10));
    }
}
