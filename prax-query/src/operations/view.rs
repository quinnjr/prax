//! View operations for querying database views (read-only).

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::Filter;
use crate::pagination::Pagination;
use crate::traits::{MaterializedView, QueryEngine, View};
use crate::types::{OrderBy, Select};

/// A query operation that finds multiple records from a view.
///
/// Views are read-only, so only SELECT operations are supported.
///
/// # Example
///
/// ```rust,ignore
/// let stats = client
///     .user_stats()
///     .find_many()
///     .r#where(user_stats::post_count::gte(10))
///     .order_by(user_stats::post_count::desc())
///     .take(100)
///     .exec()
///     .await?;
/// ```
#[allow(dead_code)]
pub struct ViewFindManyOperation<E: QueryEngine, V: View> {
    engine: E,
    filter: Filter,
    order_by: OrderBy,
    pagination: Pagination,
    select: Select,
    distinct: Option<Vec<String>>,
    _view: PhantomData<V>,
}

impl<E: QueryEngine, V: View> ViewFindManyOperation<E, V> {
    /// Create a new ViewFindMany operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            order_by: OrderBy::none(),
            pagination: Pagination::new(),
            select: Select::All,
            distinct: None,
            _view: PhantomData,
        }
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
    pub fn distinct(mut self, columns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.distinct = Some(columns.into_iter().map(Into::into).collect());
        self
    }

    /// Set cursor for cursor-based pagination.
    pub fn cursor(mut self, cursor: crate::pagination::Cursor) -> Self {
        self.pagination = self.pagination.cursor(cursor);
        self
    }

    /// Build the SQL query.
    ///
    /// View operations do not yet accept a dialect. They emit Postgres
    /// placeholders and `DISTINCT ON`; behaviour against other backends is
    /// undefined until view ops inherit the dialect-threaded build_sql shape
    /// that `FindManyOperation` and friends use.
    pub fn build_sql(&self) -> (String, Vec<crate::filter::FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, &crate::dialect::Postgres);

        let mut sql = String::new();

        // SELECT clause
        sql.push_str("SELECT ");
        if let Some(ref cols) = self.distinct {
            sql.push_str("DISTINCT ON (");
            sql.push_str(&cols.join(", "));
            sql.push_str(") ");
        }
        sql.push_str(&self.select.to_sql());

        // FROM clause - use the view name
        sql.push_str(" FROM ");
        sql.push_str(V::DB_VIEW_NAME);

        // WHERE clause
        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        // ORDER BY clause
        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.order_by.to_sql());
        }

        // LIMIT/OFFSET clause
        let pagination_sql = self.pagination.to_sql();
        if !pagination_sql.is_empty() {
            sql.push(' ');
            sql.push_str(&pagination_sql);
        }

        (sql, params)
    }

    /// Execute the query.
    ///
    /// Note: This requires the query engine to support view queries.
    /// For now, we use raw query execution.
    pub async fn exec(self) -> QueryResult<Vec<V>>
    where
        V: Send + 'static + serde::de::DeserializeOwned,
    {
        let (sql, _params) = self.build_sql();
        // Views return the same data structure - use raw query
        // The actual implementation would depend on the QueryEngine
        let _ = sql;
        Ok(Vec::new()) // Placeholder - actual implementation in database-specific crates
    }
}

/// A query operation that finds a single record from a view.
pub struct ViewFindFirstOperation<E: QueryEngine, V: View> {
    inner: ViewFindManyOperation<E, V>,
}

impl<E: QueryEngine, V: View> ViewFindFirstOperation<E, V> {
    /// Create a new ViewFindFirst operation.
    pub fn new(engine: E) -> Self {
        Self {
            inner: ViewFindManyOperation::new(engine).take(1),
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.inner = self.inner.r#where(filter);
        self
    }

    /// Set the order by clause.
    pub fn order_by(mut self, order: impl Into<OrderBy>) -> Self {
        self.inner = self.inner.order_by(order);
        self
    }

    /// Build the SQL query.
    pub fn build_sql(&self) -> (String, Vec<crate::filter::FilterValue>) {
        self.inner.build_sql()
    }

    /// Execute the query and return the first result.
    pub async fn exec(self) -> QueryResult<Option<V>>
    where
        V: Send + 'static + serde::de::DeserializeOwned,
    {
        let results = self.inner.exec().await?;
        Ok(results.into_iter().next())
    }
}

/// A count operation for views.
pub struct ViewCountOperation<E: QueryEngine, V: View> {
    engine: E,
    filter: Filter,
    _view: PhantomData<V>,
}

impl<E: QueryEngine, V: View> ViewCountOperation<E, V> {
    /// Create a new ViewCount operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            _view: PhantomData,
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.filter = self.filter.and_then(filter.into());
        self
    }

    /// Build the SQL query.
    ///
    /// Same caveat as `ViewFindManyOperation::build_sql`: view ops still emit
    /// Postgres placeholders unconditionally.
    pub fn build_sql(&self) -> (String, Vec<crate::filter::FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, &crate::dialect::Postgres);

        let mut sql = format!("SELECT COUNT(*) FROM {}", V::DB_VIEW_NAME);

        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        (sql, params)
    }

    /// Execute the count query.
    pub async fn exec(self) -> QueryResult<u64> {
        let (sql, params) = self.build_sql();
        self.engine.count(&sql, params).await
    }
}

/// A refresh operation for materialized views.
///
/// This operation refreshes the data in a materialized view.
/// For PostgreSQL, this executes `REFRESH MATERIALIZED VIEW`.
///
/// # Example
///
/// ```rust,ignore
/// // Refresh a materialized view
/// client
///     .user_stats()
///     .refresh()
///     .exec()
///     .await?;
///
/// // Refresh concurrently (allows reads during refresh)
/// client
///     .user_stats()
///     .refresh()
///     .concurrently()
///     .exec()
///     .await?;
/// ```
pub struct RefreshMaterializedViewOperation<E: QueryEngine, V: MaterializedView> {
    engine: E,
    concurrently: bool,
    _view: PhantomData<V>,
}

impl<E: QueryEngine, V: MaterializedView> RefreshMaterializedViewOperation<E, V> {
    /// Create a new refresh operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            concurrently: false,
            _view: PhantomData,
        }
    }

    /// Refresh concurrently (allows reads during refresh).
    ///
    /// Note: Concurrent refresh requires a unique index on the view.
    /// Not all databases support concurrent refresh.
    pub fn concurrently(mut self) -> Self {
        self.concurrently = V::SUPPORTS_CONCURRENT_REFRESH;
        self
    }

    /// Execute the refresh operation.
    pub async fn exec(self) -> QueryResult<()> {
        self.engine
            .refresh_materialized_view(V::DB_VIEW_NAME, self.concurrently)
            .await
    }
}

/// The view query builder that provides access to view query operations.
///
/// Views are read-only, so only SELECT operations are available.
/// No create, update, or delete operations are provided.
pub struct ViewQueryBuilder<E: QueryEngine, V: View> {
    engine: E,
    _view: PhantomData<V>,
}

impl<E: QueryEngine, V: View> ViewQueryBuilder<E, V> {
    /// Create a new view query builder.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            _view: PhantomData,
        }
    }

    /// Start a find_many query on the view.
    pub fn find_many(&self) -> ViewFindManyOperation<E, V> {
        ViewFindManyOperation::new(self.engine.clone())
    }

    /// Start a find_first query on the view.
    pub fn find_first(&self) -> ViewFindFirstOperation<E, V> {
        ViewFindFirstOperation::new(self.engine.clone())
    }

    /// Start a count query on the view.
    pub fn count(&self) -> ViewCountOperation<E, V> {
        ViewCountOperation::new(self.engine.clone())
    }
}

impl<E: QueryEngine, V: MaterializedView> ViewQueryBuilder<E, V> {
    /// Start a refresh operation for a materialized view.
    ///
    /// This is only available for materialized views.
    pub fn refresh(&self) -> RefreshMaterializedViewOperation<E, V> {
        RefreshMaterializedViewOperation::new(self.engine.clone())
    }
}

impl<E: QueryEngine, V: View> Clone for ViewQueryBuilder<E, V> {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            _view: PhantomData,
        }
    }
}

/// A view accessor that provides query operations for a specific view.
///
/// This is typically generated by the proc-macro for each view.
pub trait ViewAccessor<E: QueryEngine>: Send + Sync {
    /// The view type.
    type View: View;

    /// Get the query engine.
    fn engine(&self) -> &E;

    /// Start a find_many query.
    fn find_many(&self) -> ViewFindManyOperation<E, Self::View>;

    /// Start a find_first query.
    fn find_first(&self) -> ViewFindFirstOperation<E, Self::View>;

    /// Count records in the view.
    fn count(&self) -> ViewCountOperation<E, Self::View>;
}

/// A materialized view accessor with refresh capabilities.
pub trait MaterializedViewAccessor<E: QueryEngine>: ViewAccessor<E>
where
    Self::View: MaterializedView,
{
    /// Refresh the materialized view.
    fn refresh(&self) -> RefreshMaterializedViewOperation<E, Self::View>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;
    use crate::filter::FilterValue;
    use crate::traits::BoxFuture;

    struct TestView;

    impl View for TestView {
        const VIEW_NAME: &'static str = "TestView";
        const DB_VIEW_NAME: &'static str = "test_view";
        const COLUMNS: &'static [&'static str] = &["id", "user_id", "post_count"];
        const IS_MATERIALIZED: bool = false;
    }

    struct TestMaterializedView;

    impl View for TestMaterializedView {
        const VIEW_NAME: &'static str = "TestMaterializedView";
        const DB_VIEW_NAME: &'static str = "test_materialized_view";
        const COLUMNS: &'static [&'static str] = &["id", "stats"];
        const IS_MATERIALIZED: bool = true;
    }

    impl MaterializedView for TestMaterializedView {
        const SUPPORTS_CONCURRENT_REFRESH: bool = true;
    }

    #[derive(Clone)]
    struct MockEngine;

    impl QueryEngine for MockEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

        fn query_many<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn query_one<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn query_optional<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }

        fn execute_insert<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn execute_update<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn count(&self, _sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
            Box::pin(async { Ok(42) })
        }

        fn refresh_materialized_view(
            &self,
            view_name: &str,
            concurrently: bool,
        ) -> BoxFuture<'_, QueryResult<()>> {
            let view_name = view_name.to_string();
            Box::pin(async move {
                let _ = (view_name, concurrently);
                Ok(())
            })
        }
    }

    // ========== ViewFindManyOperation Tests ==========

    #[test]
    fn test_view_find_many_basic() {
        let op = ViewFindManyOperation::<MockEngine, TestView>::new(MockEngine);
        let (sql, params) = op.build_sql();

        assert_eq!(sql, "SELECT * FROM test_view");
        assert!(params.is_empty());
    }

    #[test]
    fn test_view_find_many_with_filter() {
        let op = ViewFindManyOperation::<MockEngine, TestView>::new(MockEngine)
            .r#where(Filter::Gte("post_count".into(), FilterValue::Int(10)));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("post_count"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_view_find_many_with_pagination() {
        let op = ViewFindManyOperation::<MockEngine, TestView>::new(MockEngine)
            .skip(10)
            .take(20);

        let (sql, _) = op.build_sql();

        assert!(sql.contains("LIMIT 20"));
        assert!(sql.contains("OFFSET 10"));
    }

    #[test]
    fn test_view_find_many_with_order() {
        use crate::types::OrderByField;

        let op = ViewFindManyOperation::<MockEngine, TestView>::new(MockEngine)
            .order_by(OrderByField::desc("post_count"));

        let (sql, _) = op.build_sql();

        assert!(sql.contains("ORDER BY post_count DESC"));
    }

    #[test]
    fn test_view_find_many_with_distinct() {
        let op =
            ViewFindManyOperation::<MockEngine, TestView>::new(MockEngine).distinct(["user_id"]);

        let (sql, _) = op.build_sql();

        assert!(sql.contains("DISTINCT ON (user_id)"));
    }

    // ========== ViewFindFirstOperation Tests ==========

    #[test]
    fn test_view_find_first_has_limit_1() {
        let op = ViewFindFirstOperation::<MockEngine, TestView>::new(MockEngine);
        let (sql, _) = op.build_sql();

        assert!(sql.contains("LIMIT 1"));
    }

    #[test]
    fn test_view_find_first_with_filter() {
        let op = ViewFindFirstOperation::<MockEngine, TestView>::new(MockEngine)
            .r#where(Filter::Equals("user_id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("user_id"));
        assert!(sql.contains("LIMIT 1"));
        assert_eq!(params.len(), 1);
    }

    // ========== ViewCountOperation Tests ==========

    #[test]
    fn test_view_count_basic() {
        let op = ViewCountOperation::<MockEngine, TestView>::new(MockEngine);
        let (sql, params) = op.build_sql();

        assert_eq!(sql, "SELECT COUNT(*) FROM test_view");
        assert!(params.is_empty());
    }

    #[test]
    fn test_view_count_with_filter() {
        let op = ViewCountOperation::<MockEngine, TestView>::new(MockEngine)
            .r#where(Filter::Gte("post_count".into(), FilterValue::Int(5)));

        let (sql, params) = op.build_sql();

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("post_count"));
        assert_eq!(params.len(), 1);
    }

    #[tokio::test]
    async fn test_view_count_exec() {
        let op = ViewCountOperation::<MockEngine, TestView>::new(MockEngine);
        let result = op.exec().await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42); // Mock returns 42
    }

    // ========== RefreshMaterializedViewOperation Tests ==========

    #[test]
    fn test_refresh_materialized_view_default() {
        let op =
            RefreshMaterializedViewOperation::<MockEngine, TestMaterializedView>::new(MockEngine);

        assert!(!op.concurrently);
    }

    #[test]
    fn test_refresh_materialized_view_concurrently() {
        let op =
            RefreshMaterializedViewOperation::<MockEngine, TestMaterializedView>::new(MockEngine)
                .concurrently();

        assert!(op.concurrently);
    }

    #[tokio::test]
    async fn test_refresh_materialized_view_exec() {
        let op =
            RefreshMaterializedViewOperation::<MockEngine, TestMaterializedView>::new(MockEngine);
        let result = op.exec().await;

        assert!(result.is_ok());
    }

    // ========== ViewQueryBuilder Tests ==========

    #[test]
    fn test_view_query_builder_find_many() {
        let builder = ViewQueryBuilder::<MockEngine, TestView>::new(MockEngine);
        let op = builder.find_many();
        let (sql, _) = op.build_sql();

        assert!(sql.contains("SELECT * FROM test_view"));
    }

    #[test]
    fn test_view_query_builder_find_first() {
        let builder = ViewQueryBuilder::<MockEngine, TestView>::new(MockEngine);
        let op = builder.find_first();
        let (sql, _) = op.build_sql();

        assert!(sql.contains("LIMIT 1"));
    }

    #[test]
    fn test_view_query_builder_count() {
        let builder = ViewQueryBuilder::<MockEngine, TestView>::new(MockEngine);
        let op = builder.count();
        let (sql, _) = op.build_sql();

        assert!(sql.contains("COUNT(*)"));
    }

    #[test]
    fn test_materialized_view_query_builder_refresh() {
        let builder = ViewQueryBuilder::<MockEngine, TestMaterializedView>::new(MockEngine);
        let _op = builder.refresh();
        // Just verify we can call refresh on materialized views
    }

    #[test]
    fn test_view_query_builder_clone() {
        let builder = ViewQueryBuilder::<MockEngine, TestView>::new(MockEngine);
        let _cloned = builder.clone();
    }

    // ========== View Trait Tests ==========

    #[test]
    fn test_view_trait_constants() {
        assert_eq!(TestView::VIEW_NAME, "TestView");
        assert_eq!(TestView::DB_VIEW_NAME, "test_view");
        assert_eq!(TestView::COLUMNS, &["id", "user_id", "post_count"]);
        assert!(!TestView::IS_MATERIALIZED);
    }

    #[test]
    fn test_materialized_view_trait_constants() {
        assert_eq!(TestMaterializedView::VIEW_NAME, "TestMaterializedView");
        assert!(TestMaterializedView::IS_MATERIALIZED);
        assert!(TestMaterializedView::SUPPORTS_CONCURRENT_REFRESH);
    }
}
