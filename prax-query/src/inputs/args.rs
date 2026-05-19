//! Per-operation argument containers.
//!
//! Each struct is the layer-2 "explicit form" of an operation request:
//! the macro DSL (phase 3+) expands to a `*Args { ... }` literal that
//! the operation builder consumes via `.with_args(args)`. Direct
//! construction by hand is fully supported.
//!
//! Generic parameters:
//! - `M` — the model
//! - `W` — `WhereInput` impl for that model
//! - `I` — `IncludeInput` impl
//! - `S` — `SelectInput` impl
//! - `D` — `CreateInput::Data` / `UpdateInput::Data` payload (operation-specific)
//! - `O` — `OrderByInput` impl
//!
//! Phase 1 keeps the bounds open so hand-construction works even before
//! codegen lands. Phase 2 narrows them when the per-model types exist.

use core::marker::PhantomData;

/// Args for `find_unique`. `where` must identify at most one row.
#[derive(Debug, Clone)]
pub struct FindUniqueArgs<M, W, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Phantom marker for the model type.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, W: Default, I, S> Default for FindUniqueArgs<M, W, I, S> {
    fn default() -> Self {
        Self {
            r#where: W::default(),
            include: None,
            select: None,
            _model: PhantomData,
        }
    }
}

impl<M, W, I, S> FindUniqueArgs<M, W, I, S> {
    /// Construct with the given unique WHERE.
    pub fn new(r#where: W) -> Self {
        Self {
            r#where,
            include: None,
            select: None,
            _model: PhantomData,
        }
    }
}

/// Args for `find_first`.
#[derive(Debug, Clone)]
pub struct FindFirstArgs<M, W, I, S, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Optional order-by shape (single or vec).
    pub order_by: Option<Vec<O>>,
    /// Optional cursor value.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, W, I, S, O, C> Default for FindFirstArgs<M, W, I, S, O, C> {
    fn default() -> Self {
        Self {
            r#where: None,
            include: None,
            select: None,
            order_by: None,
            cursor: None,
            skip: None,
            take: None,
            _model: PhantomData,
        }
    }
}

/// Args for `find_many`.
#[derive(Debug, Clone)]
pub struct FindManyArgs<M, W, I, S, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Optional order-by shape (single or vec).
    pub order_by: Option<Vec<O>>,
    /// Optional cursor value.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Distinct columns.
    pub distinct: Option<Vec<String>>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, W, I, S, O, C> Default for FindManyArgs<M, W, I, S, O, C> {
    fn default() -> Self {
        Self {
            r#where: None,
            include: None,
            select: None,
            order_by: None,
            cursor: None,
            skip: None,
            take: None,
            distinct: None,
            _model: PhantomData,
        }
    }
}

/// Args for `create`.
#[derive(Debug, Clone)]
pub struct CreateArgs<M, D, I, S> {
    /// Create-data payload.
    pub data: D,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, D: Default, I, S> Default for CreateArgs<M, D, I, S> {
    fn default() -> Self {
        Self {
            data: D::default(),
            include: None,
            select: None,
            _model: PhantomData,
        }
    }
}

/// Args for `create_many`.
#[derive(Debug, Clone)]
pub struct CreateManyArgs<M, D> {
    /// Create-data payloads.
    pub data: Vec<D>,
    /// Skip rows that would violate a unique constraint (instead of erroring).
    pub skip_duplicates: Option<bool>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, D> Default for CreateManyArgs<M, D> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            skip_duplicates: None,
            _model: PhantomData,
        }
    }
}

/// Args for `update`.
#[derive(Debug, Clone)]
pub struct UpdateArgs<M, W, U, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Update-data payload.
    pub data: U,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `update_many`.
#[derive(Debug, Clone)]
pub struct UpdateManyArgs<M, W, U> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Update-data payload.
    pub data: U,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, W, U: Default> Default for UpdateManyArgs<M, W, U> {
    fn default() -> Self {
        Self {
            r#where: None,
            data: U::default(),
            _model: PhantomData,
        }
    }
}

/// Args for `upsert`.
#[derive(Debug, Clone)]
pub struct UpsertArgs<M, W, C, U, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Create-data payload (used if no row matched).
    pub create: C,
    /// Update-data payload (used if a row matched).
    pub update: U,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `delete`.
#[derive(Debug, Clone)]
pub struct DeleteArgs<M, W, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `delete_many`.
#[derive(Debug, Clone, Default)]
pub struct DeleteManyArgs<M, W> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `count`.
#[derive(Debug, Clone, Default)]
pub struct CountArgs<M, W, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Optional cursor.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `aggregate`. The aggregate spec parameter is filled in by phase 6.
#[derive(Debug, Clone, Default)]
pub struct AggregateArgs<M, W, A, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Aggregate spec (`_count` / `_avg` / `_sum` / `_min` / `_max`).
    pub aggregate: Option<A>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Optional cursor.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `group_by`. The grouping spec parameter is filled in by phase 6.
#[derive(Debug, Clone, Default)]
pub struct GroupByArgs<M, W, A, G, H = (), O = ()> {
    /// Group by these field names.
    pub by: Vec<G>,
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional HAVING input.
    pub having: Option<H>,
    /// Aggregate spec.
    pub aggregate: Option<A>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}
