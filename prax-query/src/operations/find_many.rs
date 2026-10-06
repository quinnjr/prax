//! FindMany operation for querying multiple records.

use std::marker::PhantomData;

use smallvec::SmallVec;

use crate::advanced::RowLock;
use crate::capabilities::{SupportsRowLocks, SupportsScalarSubqueryInSelect};
use crate::error::QueryResult;
use crate::filter::Filter;
use crate::pagination::Pagination;
use crate::projection::ScalarProjection;
use crate::relations::IncludeSpec;
use crate::traits::{Model, ModelRelationLoader, QueryEngine};
use crate::types::{OrderBy, Select};

/// A query operation that finds multiple records.
///
/// # Example
///
/// ```rust,ignore
/// let users = client
///     .user()
///     .find_many()
///     .r#where(user::email::contains("@example.com"))
///     .order_by(user::created_at::desc())
///     .skip(0)
///     .take(10)
///     .exec()
///     .await?;
/// ```
pub struct FindManyOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    order_by: OrderBy,
    pagination: Pagination,
    select: Select,
    distinct: Option<Vec<String>>,
    /// Relations to eager-load after the main query returns. Each
    /// spec drives one follow-up SELECT via the model's
    /// [`ModelRelationLoader`] impl. Inlined for up to two specs
    /// (the typical 0-2 case) to avoid a heap allocation on the hot
    /// builder path.
    includes: SmallVec<[IncludeSpec; 2]>,
    /// Extra scalar-subquery columns appended to the SELECT clause.
    /// Used by relation-aggregate virtual fields (`@count`, `@sum`, …).
    pub extra_projections: Vec<ScalarProjection>,
    /// Row lock appended to the statement (`FOR UPDATE`, MSSQL hints, ...).
    lock: Option<RowLock>,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> FindManyOperation<E, M> {
    /// Create a new FindMany operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            order_by: OrderBy::none(),
            pagination: Pagination::new(),
            select: Select::All,
            distinct: None,
            includes: SmallVec::new(),
            extra_projections: Vec::new(),
            lock: None,
            _model: PhantomData,
        }
    }

    /// Eager-load a relation alongside the main query.
    ///
    /// Each `.include()` call appends one follow-up SELECT that
    /// fetches the target rows for every parent returned by this
    /// find. Children get stitched onto the parent slice by the
    /// [`ModelRelationLoader`] impl emitted by `#[derive(Model)]`.
    pub fn include(mut self, spec: IncludeSpec) -> Self {
        self.includes.push(spec);
        self
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        let new_filter = filter.into();
        self.filter = self.filter.and_then(new_filter);
        self
    }

    /// Set the order by clause.
    pub fn order_by(mut self, order: impl Into<OrderBy>) -> Self {
        self.order_by = order.into();
        self
    }

    /// Skip a number of records.
    pub fn skip(mut self, n: u64) -> Self {
        self.pagination = self.pagination.skip(n);
        self
    }

    /// Take a limited number of records.
    pub fn take(mut self, n: u64) -> Self {
        self.pagination = self.pagination.take(n);
        self
    }

    /// Select specific fields.
    pub fn select(mut self, select: impl Into<Select>) -> Self {
        self.select = select.into();
        self
    }

    /// Make the query distinct.
    ///
    /// Emits `SELECT DISTINCT ON (cols)` on dialects where
    /// [`crate::dialect::SqlDialect::supports_distinct_on`] holds
    /// (Postgres). Dialects without support (MySQL, SQLite, MSSQL) fall
    /// back to plain `SELECT DISTINCT` — note the semantics change:
    /// plain `DISTINCT` deduplicates whole rows, not per-column groups.
    pub fn distinct(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.distinct = Some(columns.into_iter().map(Into::into).collect());
        self
    }

    /// Set cursor for cursor-based pagination.
    ///
    /// Emits a keyset predicate (`"col" > $n` / `"col" < $n`) AND-composed
    /// with any filter. Keyset pagination is only deterministic with a
    /// matching row order, so when no explicit [`order_by`](Self::order_by)
    /// is set the query falls back to ordering by the cursor column in the
    /// pagination direction (`After` → `ASC`, `Before` → `DESC`).
    pub fn cursor(mut self, cursor: crate::pagination::Cursor) -> Self {
        self.pagination = self.pagination.cursor(cursor);
        self
    }

    /// Apply a typed `WhereInput`. AND-composes with any previously set
    /// filter — same semantics as calling `.r#where(...)` again.
    pub fn with_where_input<W: crate::inputs::WhereInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `IncludeInput`. Merges into any previously set
    /// includes (later wins on conflicting relation names).
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    /// Apply a typed `OrderByInput` (replaces current).
    pub fn with_order_by_input<O: crate::inputs::OrderByInput<Model = M>>(mut self, o: O) -> Self {
        self.order_by = o.into_ir();
        self
    }

    /// Doc-hidden accessor for the current filter — needed for unit
    /// tests that don't have a running engine to issue queries against.
    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
}

impl<E, M> FindManyOperation<E, M>
where
    E: QueryEngine + SupportsScalarSubqueryInSelect,
    M: Model + crate::row::FromRow,
{
    /// Append a scalar-subquery projection to the SELECT list.
    ///
    /// Available only on engines that implement
    /// [`SupportsScalarSubqueryInSelect`]. SQL backends (Postgres, MySQL,
    /// SQLite, MSSQL, DuckDB, SQLx) all satisfy this bound; MongoDB,
    /// ScyllaDB, and Cassandra do not — calling this method on those
    /// engines is a **compile-time error**.
    pub fn with_scalar_projection(mut self, proj: ScalarProjection) -> Self {
        self.extra_projections.push(proj);
        self
    }
}

impl<E, M> FindManyOperation<E, M>
where
    E: QueryEngine + SupportsRowLocks,
    M: Model + crate::row::FromRow,
{
    /// Lock the selected rows (`SELECT ... FOR UPDATE` and friends).
    ///
    /// Locks are held until the surrounding transaction ends. Outside a
    /// transaction they are released as soon as the statement finishes, so
    /// they protect nothing; run locking queries inside
    /// `PraxClient::transaction`. With `include`, only the root query is
    /// locked; eagerly loaded relations are read without locks.
    ///
    /// Unsupported combinations (for example `SKIP LOCKED` with a shared
    /// lock on MSSQL) and use together with `distinct` fail at `exec`.
    pub fn lock(mut self, lock: impl Into<RowLock>) -> Self {
        self.lock = Some(lock.into());
        self
    }

    /// Shorthand for `.lock(RowLock::for_update())`.
    pub fn for_update(self) -> Self {
        self.lock(RowLock::for_update())
    }

    /// Shorthand for `.lock(RowLock::for_share())`.
    pub fn for_share(self) -> Self {
        self.lock(RowLock::for_share())
    }
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> FindManyOperation<E, M> {
    /// Build the SQL query.
    ///
    /// A row lock the dialect cannot express (see [`lock`](Self::lock)) is
    /// left out of the SQL built here; only `exec` validates the lock and
    /// reports the error.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<crate::filter::FilterValue>) {
        // Projection params come first; WHERE params are offset by the
        // number of params already consumed by the extra projections so
        // that all dialect placeholders form a single contiguous sequence.
        let proj_param_count: usize = self.extra_projections.iter().map(|p| p.params.len()).sum();
        let (where_sql, where_params) = self.filter.to_sql(proj_param_count, dialect);

        let cursor = self.pagination.cursor.as_ref();
        let mut params: Vec<crate::filter::FilterValue> = Vec::with_capacity(
            proj_param_count + where_params.len() + usize::from(cursor.is_some()),
        );

        // Pre-size for the fixed clauses plus the filter fragment; the
        // ORDER BY / LIMIT / DISTINCT tail grows amortized on top.
        let mut sql = String::with_capacity(64 + M::TABLE_NAME.len() + where_sql.len());

        // SELECT clause
        sql.push_str("SELECT ");
        if let Some(ref cols) = self.distinct {
            if dialect.supports_distinct_on() {
                sql.push_str("DISTINCT ON (");
                sql.push_str(&cols.join(", "));
                sql.push_str(") ");
            } else {
                // `DISTINCT ON` is Postgres-only; degrade to plain
                // DISTINCT (whole-row dedup) on dialects without support.
                sql.push_str("DISTINCT ");
            }
        }
        self.select.write_sql(&mut sql);

        // Extra scalar-subquery projections
        let mut proj_offset = 0usize;
        for proj in &self.extra_projections {
            sql.push_str(", ");
            let frag = proj.to_sql(proj_offset, dialect, &mut params);
            sql.push('(');
            sql.push_str(&frag);
            sql.push_str(") AS \"");
            sql.push_str(proj.alias);
            sql.push('"');
            proj_offset += proj.params.len();
        }

        // FROM clause
        sql.push_str(" FROM ");
        sql.push_str(M::TABLE_NAME);
        let lock_sql = super::lock::render(self.lock.as_ref(), dialect);
        if let Some(hint) = &lock_sql.table_hint {
            sql.push(' ');
            sql.push_str(hint);
        }

        // WHERE clause — a stored cursor AND-composes a keyset predicate
        // (`"col" > $n` / `"col" < $n`) after the filter; its value binds
        // last so the placeholder sequence stays dense.
        if !self.filter.is_none() || cursor.is_some() {
            sql.push_str(" WHERE ");
            let mut conjunct = false;
            if !self.filter.is_none() {
                sql.push_str(&where_sql);
                conjunct = true;
            }
            if let Some(cursor) = cursor {
                if conjunct {
                    sql.push_str(" AND ");
                }
                sql.push_str(&dialect.quote_ident(&cursor.column));
                sql.push(' ');
                sql.push_str(cursor.operator());
                sql.push(' ');
                sql.push_str(&dialect.placeholder(proj_param_count + where_params.len() + 1));
            }
        }
        params.extend(where_params);
        if let Some(cursor) = cursor {
            params.push(match &cursor.value {
                crate::pagination::CursorValue::Int(v) => crate::filter::FilterValue::Int(*v),
                crate::pagination::CursorValue::String(s) => {
                    crate::filter::FilterValue::String(s.clone())
                }
            });
        }

        // ORDER BY clause. A stored cursor with no explicit `order_by`
        // falls back to ordering by the cursor column in the pagination
        // direction — without a deterministic row order the keyset
        // predicate can skip or re-see rows between pages.
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            self.order_by.write_sql(&mut sql);
        } else if let Some(cursor) = cursor {
            sql.push_str(" ORDER BY ");
            sql.push_str(&dialect.quote_ident(&cursor.column));
            sql.push_str(match cursor.direction {
                crate::pagination::CursorDirection::After => " ASC",
                crate::pagination::CursorDirection::Before => " DESC",
            });
        }

        // LIMIT/OFFSET clause (the cursor predicate lives in WHERE).
        if self.pagination.take.is_some() || self.pagination.skip.is_some() {
            sql.push(' ');
            self.pagination.write_sql(&mut sql);
        }

        if let Some(suffix) = &lock_sql.suffix {
            sql.push(' ');
            sql.push_str(suffix);
        }

        (sql, params)
    }

    /// Execute the query.
    ///
    /// After the main SELECT hydrates the parent rows, any pending
    /// `.include()` specs are dispatched through
    /// [`ModelRelationLoader::load_relation`] which issues one
    /// additional SELECT per relation and stitches the children onto
    /// the parent slice.
    pub async fn exec(self) -> QueryResult<Vec<M>>
    where
        M: Send + 'static + ModelRelationLoader<E>,
    {
        let dialect = self.engine.dialect();
        super::lock::validate(self.lock.as_ref(), dialect, self.distinct.is_some())?;
        let (sql, params) = self.build_sql(dialect);
        let mut parents = self.engine.query_many::<M>(&sql, params).await?;
        for spec in &self.includes {
            <M as ModelRelationLoader<E>>::load_relation(&self.engine, &mut parents, spec).await?;
        }
        Ok(parents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;
    use crate::filter::FilterValue;
    use crate::pagination::{Cursor, CursorDirection, CursorValue};
    use crate::types::OrderByField;

    struct TestModel;

    impl Model for TestModel {
        const MODEL_NAME: &'static str = "TestModel";
        const TABLE_NAME: &'static str = "test_models";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "name", "email"];
    }

    impl crate::row::FromRow for TestModel {
        fn from_row(_row: &impl crate::row::RowRef) -> Result<Self, crate::row::RowError> {
            Ok(TestModel)
        }
    }

    // Minimal `ModelRelationLoader` impl for the mock — real models
    // get one from codegen. Errors on any include name (the tests
    // never register an include).
    impl crate::traits::ModelRelationLoader<MockEngine> for TestModel {
        fn load_relation<'a>(
            _engine: &'a MockEngine,
            _parents: &'a mut [Self],
            spec: &'a crate::relations::IncludeSpec,
        ) -> crate::traits::BoxFuture<'a, QueryResult<()>> {
            let name = spec.relation_name.clone();
            Box::pin(async move {
                Err(QueryError::internal(format!(
                    "unknown relation '{name}' on TestModel (mock)",
                )))
            })
        }
    }

    #[derive(Clone)]
    struct MockEngine;

    impl QueryEngine for MockEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

        fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }

        fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    // ========== Construction Tests ==========

    #[test]
    fn test_find_many_new() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM test_models"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_find_many_basic() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(sql, "SELECT * FROM test_models");
        assert!(params.is_empty());
    }

    // ========== Filter Tests ==========

    #[test]
    fn test_find_many_with_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "Alice".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""name" = $1"#));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_many_with_compound_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("active".to_string()),
            ))
            .r#where(Filter::Gte("age".into(), FilterValue::Int(18)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_many_with_or_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::or([
            Filter::Equals("role".into(), FilterValue::String("admin".to_string())),
            Filter::Equals("role".into(), FilterValue::String("moderator".to_string())),
        ]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("OR"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_many_with_in_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::In(
            "status".into(),
            vec![
                FilterValue::String("pending".to_string()),
                FilterValue::String("processing".to_string()),
            ],
        ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_many_without_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
        assert!(params.is_empty());
    }

    // ========== Order By Tests ==========

    #[test]
    fn test_find_many_with_order() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .order_by(OrderByField::desc("created_at"));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY created_at DESC"));
    }

    #[test]
    fn test_find_many_with_asc_order() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .order_by(OrderByField::asc("name"));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY name ASC"));
    }

    #[test]
    fn test_find_many_without_order() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("ORDER BY"));
    }

    #[test]
    fn test_find_many_order_replaces() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .order_by(OrderByField::asc("name"))
            .order_by(OrderByField::desc("created_at"));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(!sql.contains("ORDER BY name"));
    }

    // ========== Pagination Tests ==========

    #[test]
    fn test_find_many_with_pagination() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .skip(10)
            .take(20);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIMIT 20"));
        assert!(sql.contains("OFFSET 10"));
    }

    #[test]
    fn test_find_many_with_skip_only() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).skip(5);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("OFFSET 5"));
    }

    #[test]
    fn test_find_many_with_take_only() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).take(100);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIMIT 100"));
    }

    #[test]
    fn test_find_many_with_cursor() {
        let cursor = Cursor::new("id", CursorValue::Int(100), CursorDirection::After);
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .cursor(cursor)
            .take(10);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // Cursor pagination emits a keyset predicate and binds the value.
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains(r#"WHERE "id" > $1"#), "got: {sql}");
        assert_eq!(params, vec![FilterValue::Int(100)]);
    }

    #[test]
    fn test_find_many_cursor_exact_sql() {
        let cursor = Cursor::new("id", CursorValue::Int(100), CursorDirection::After);
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()))
            .cursor(cursor)
            .take(10);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(
            sql,
            r#"SELECT * FROM test_models WHERE "name" = $1 AND "id" > $2 ORDER BY "id" ASC LIMIT 10"#
        );
        assert_eq!(
            params,
            vec![FilterValue::String("a".to_string()), FilterValue::Int(100)]
        );
    }

    /// A cursor with no explicit `order_by` falls back to ordering by the
    /// cursor column in the pagination direction — keyset pagination is
    /// only deterministic with a matching row order.
    #[test]
    fn test_find_many_cursor_orders_by_cursor_column() {
        let after = Cursor::new("id", CursorValue::Int(100), CursorDirection::After);
        let (sql, _) = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .cursor(after)
            .build_sql(&crate::dialect::Postgres);
        assert!(
            sql.contains(r#"ORDER BY "id" ASC"#),
            "After cursor must order ASC, got: {sql}"
        );

        let before = Cursor::new("id", CursorValue::Int(100), CursorDirection::Before);
        let (sql, _) = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .cursor(before)
            .build_sql(&crate::dialect::Postgres);
        assert!(
            sql.contains(r#"WHERE "id" < $1"#) && sql.contains(r#"ORDER BY "id" DESC"#),
            "Before cursor must order DESC, got: {sql}"
        );
    }

    /// An explicit `order_by` wins over the cursor-derived fallback.
    #[test]
    fn test_find_many_cursor_explicit_order_by_wins() {
        let cursor = Cursor::new("id", CursorValue::Int(100), CursorDirection::After);
        let (sql, _) = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .cursor(cursor)
            .order_by(OrderByField::desc("created_at"))
            .build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY created_at DESC"), "got: {sql}");
        assert!(!sql.contains(r#"ORDER BY "id""#), "got: {sql}");
    }

    // ========== Select Tests ==========

    #[test]
    fn test_find_many_with_select() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id, name FROM"));
        assert!(!sql.contains("SELECT *"));
    }

    #[test]
    fn test_find_many_select_single_field() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .select(Select::fields(["id"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id FROM"));
    }

    #[test]
    fn test_find_many_select_all() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).select(Select::All);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM"));
    }

    /// Task 28 regression test: a narrow `Select::fields` list must turn
    /// the emitted `SELECT *` into an explicit column list so wide models
    /// don't waste bandwidth. The projection still hydrates as the full
    /// struct, so callers are responsible for covering every non-`Option`
    /// field — see the CHANGELOG migration note.
    #[test]
    fn find_many_emits_explicit_column_list_when_select_narrows() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .select(Select::fields(["id", "email"]));
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);
        assert!(
            sql.contains("SELECT id, email FROM") && !sql.contains("SELECT *"),
            "expected narrow select list, got: {sql}"
        );
    }

    /// Counterpart to the narrowing test: with no `.select(...)` call,
    /// the default `Select::All` must still emit `SELECT *`. Guards
    /// against a regression where a future refactor of the default
    /// value silently drops back to an empty column list.
    #[test]
    fn find_many_emits_star_when_no_select() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);
        assert!(sql.contains("SELECT *"), "expected SELECT *, got: {sql}");
    }

    // ========== Distinct Tests ==========

    #[test]
    fn test_find_many_with_distinct() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).distinct(["category"]);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DISTINCT ON (category)"));
    }

    #[test]
    fn test_find_many_with_multiple_distinct() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .distinct(["category", "status"]);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DISTINCT ON (category, status)"));
    }
    #[test]
    fn test_find_many_without_distinct() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("DISTINCT"));
    }

    #[test]
    fn test_find_many_distinct_falls_back_without_distinct_on_support() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).distinct(["category"]);

        // MySQL has no `DISTINCT ON`; expect plain DISTINCT instead of
        // emitting syntax the backend would reject.
        let (sql, _) = op.build_sql(&crate::dialect::Mysql);

        assert!(sql.contains("SELECT DISTINCT "), "got: {sql}");
        assert!(!sql.contains("DISTINCT ON"), "got: {sql}");
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_find_many_sql_structure() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .order_by(OrderByField::desc("created_at"))
            .skip(10)
            .take(20)
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        // Check correct SQL clause ordering
        let select_pos = sql.find("SELECT").unwrap();
        let from_pos = sql.find("FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();
        let order_pos = sql.find("ORDER BY").unwrap();
        let limit_pos = sql.find("LIMIT").unwrap();
        let offset_pos = sql.find("OFFSET").unwrap();

        assert!(select_pos < from_pos);
        assert!(from_pos < where_pos);
        assert!(where_pos < order_pos);
        assert!(order_pos < limit_pos);
        assert!(limit_pos < offset_pos);
    }

    #[test]
    fn test_find_many_table_name() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Async Execution Tests ==========

    #[tokio::test]
    async fn test_find_many_exec() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).r#where(
            Filter::Equals("status".into(), FilterValue::String("active".to_string())),
        );

        let result = op.exec().await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty()); // MockEngine returns empty vec
    }

    #[tokio::test]
    async fn test_find_many_exec_no_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine);

        let result = op.exec().await;

        assert!(result.is_ok());
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_find_many_full_chain() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("active".to_string()),
            ))
            .order_by(OrderByField::desc("created_at"))
            .skip(10)
            .take(20)
            .select(Select::fields(["id", "name", "email"]))
            .distinct(["category"]);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DISTINCT ON (category)"));
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(sql.contains("LIMIT 20"));
        assert!(sql.contains("OFFSET 10"));
        assert_eq!(params.len(), 1);
    }

    // ========== Edge Cases ==========

    #[test]
    fn test_find_many_with_like_filter() {
        let op =
            FindManyOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::Contains(
                "email".into(),
                FilterValue::String("@example.com".to_string()),
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_many_with_null_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::IsNull("deleted_at".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IS NULL"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_find_many_with_not_filter() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::Not(
            Box::new(Filter::Equals(
                "status".into(),
                FilterValue::String("deleted".to_string()),
            )),
        ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("NOT"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_many_with_between_equivalent() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Gte("age".into(), FilterValue::Int(18)))
            .r#where(Filter::Lte("age".into(), FilterValue::Int(65)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    // ========== Cross-Dialect Tests ==========

    #[test]
    fn builds_mysql_placeholders() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()));
        let (sql, _) = op.build_sql(&crate::dialect::Mysql);
        assert!(
            sql.contains("?") && !sql.contains("$1"),
            "expected ? placeholders, got: {sql}"
        );
    }

    #[test]
    fn builds_mssql_placeholders() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()));
        let (sql, _) = op.build_sql(&crate::dialect::Mssql);
        assert!(sql.contains("@P1"), "expected @P1 placeholders, got: {sql}");
    }

    #[test]
    fn builds_sqlite_placeholders() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()));
        let (sql, _) = op.build_sql(&crate::dialect::Sqlite);
        assert!(sql.contains("?1"), "expected ?1 placeholders, got: {sql}");
    }
}
