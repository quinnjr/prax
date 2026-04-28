//! FindMany operation for querying multiple records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::Filter;
use crate::pagination::Pagination;
use crate::traits::{Model, QueryEngine};
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
            _model: PhantomData,
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
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<crate::filter::FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, dialect);

        let mut sql = String::new();

        // SELECT clause
        sql.push_str("SELECT ");
        if let Some(ref cols) = self.distinct {
            sql.push_str("DISTINCT ON (");
            sql.push_str(&cols.join(", "));
            sql.push_str(") ");
        }
        sql.push_str(&self.select.to_sql());

        // FROM clause
        sql.push_str(" FROM ");
        sql.push_str(M::TABLE_NAME);

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
    pub async fn exec(self) -> QueryResult<Vec<M>>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.query_many::<M>(&sql, params).await
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
        assert!(sql.contains("name = $1"));
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

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        // Cursor pagination should add some cursor-based filtering
        assert!(sql.contains("LIMIT 10"));
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
