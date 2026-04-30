//! Delete operation for removing records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::traits::{Model, QueryEngine};
use crate::types::Select;

/// A delete operation for removing records.
///
/// # Example
///
/// ```rust,ignore
/// let deleted = client
///     .user()
///     .delete()
///     .r#where(user::id::equals(1))
///     .exec()
///     .await?;
/// ```
pub struct DeleteOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    select: Select,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> DeleteOperation<E, M> {
    /// Create a new Delete operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            select: Select::All,
            _model: PhantomData,
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        let new_filter = filter.into();
        self.filter = self.filter.and_then(new_filter);
        self
    }

    /// Select specific fields to return from deleted records.
    pub fn select(mut self, select: impl Into<Select>) -> Self {
        self.select = select.into();
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, dialect);

        let mut sql = String::new();

        // DELETE FROM clause
        sql.push_str("DELETE FROM ");
        sql.push_str(M::TABLE_NAME);

        // WHERE clause
        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        // RETURNING clause
        sql.push_str(&dialect.returning_clause(&self.select.to_sql()));

        (sql, params)
    }

    /// Build SQL without RETURNING (for count).
    fn build_sql_count(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, dialect);

        let mut sql = String::new();

        sql.push_str("DELETE FROM ");
        sql.push_str(M::TABLE_NAME);

        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        (sql, params)
    }

    /// Execute the delete and return deleted records.
    pub async fn exec(self) -> QueryResult<Vec<M>>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_update::<M>(&sql, params).await
    }

    /// Execute the delete and return the count of deleted records.
    pub async fn exec_count(self) -> QueryResult<u64> {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql_count(dialect);
        self.engine.execute_delete(&sql, params).await
    }
}

/// Delete many records at once.
pub struct DeleteManyOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model> DeleteManyOperation<E, M> {
    /// Create a new DeleteMany operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            _model: PhantomData,
        }
    }

    /// Add a filter condition.
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        let new_filter = filter.into();
        self.filter = self.filter.and_then(new_filter);
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, dialect);

        let mut sql = String::new();

        sql.push_str("DELETE FROM ");
        sql.push_str(M::TABLE_NAME);

        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        (sql, params)
    }

    /// Execute the delete and return the count of deleted records.
    pub async fn exec(self) -> QueryResult<u64> {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.execute_delete(&sql, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;

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
    struct MockEngine {
        delete_count: u64,
    }

    impl MockEngine {
        fn new() -> Self {
            Self { delete_count: 0 }
        }

        fn with_count(count: u64) -> Self {
            Self {
                delete_count: count,
            }
        }
    }

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
            let count = self.delete_count;
            Box::pin(async move { Ok(count) })
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

    // ========== DeleteOperation Tests ==========

    #[test]
    fn test_delete_new() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DELETE FROM test_models"));
        assert!(sql.contains("RETURNING *"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_delete_with_filter() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DELETE FROM test_models"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""id" = $1"#));
        assert!(sql.contains("RETURNING *"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_delete_with_select() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("RETURNING id, name"));
        assert!(!sql.contains("RETURNING *"));
    }

    #[test]
    fn test_delete_with_compound_filter() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("deleted".to_string()),
            ))
            .r#where(Filter::Lt(
                "updated_at".into(),
                FilterValue::String("2024-01-01".to_string()),
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_delete_without_filter() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
        assert!(sql.contains("DELETE FROM test_models"));
    }

    #[test]
    fn test_delete_build_sql_count() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql_count(&crate::dialect::Postgres);

        assert!(sql.contains("DELETE FROM test_models"));
        assert!(sql.contains("WHERE"));
        assert!(!sql.contains("RETURNING")); // No RETURNING for count
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_delete_with_or_filter() {
        let op =
            DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(Filter::or([
                Filter::Equals("status".into(), FilterValue::String("deleted".to_string())),
                Filter::Equals("status".into(), FilterValue::String("archived".to_string())),
            ]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("OR"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_delete_with_in_filter() {
        let op =
            DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(Filter::In(
                "id".into(),
                vec![
                    FilterValue::Int(1),
                    FilterValue::Int(2),
                    FilterValue::Int(3),
                ],
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 3);
    }

    #[tokio::test]
    async fn test_delete_exec() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_exec_count() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::with_count(5))
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec_count().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5);
    }

    // ========== DeleteManyOperation Tests ==========

    #[test]
    fn test_delete_many_new() {
        let op = DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DELETE FROM test_models"));
        assert!(!sql.contains("RETURNING"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_delete_many() {
        let op = DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(
            Filter::In("id".into(), vec![FilterValue::Int(1), FilterValue::Int(2)]),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("DELETE FROM test_models"));
        assert!(sql.contains("IN"));
        assert!(!sql.contains("RETURNING"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_delete_many_with_compound_filter() {
        let op = DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("tenant_id".into(), FilterValue::Int(1)))
            .r#where(Filter::Equals("deleted".into(), FilterValue::Bool(true)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_delete_many_without_filter() {
        let op = DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
    }

    #[test]
    fn test_delete_many_with_not_in_filter() {
        let op = DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(
            Filter::NotIn(
                "status".into(),
                vec![
                    FilterValue::String("active".to_string()),
                    FilterValue::String("pending".to_string()),
                ],
            ),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("NOT IN"));
        assert_eq!(params.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_many_exec() {
        let op =
            DeleteManyOperation::<MockEngine, TestModel>::new(MockEngine::with_count(10)).r#where(
                Filter::Equals("status".into(), FilterValue::String("deleted".to_string())),
            );

        let result = op.exec().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_delete_sql_structure() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        let delete_pos = sql.find("DELETE FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();
        let returning_pos = sql.find("RETURNING").unwrap();

        assert!(delete_pos < where_pos);
        assert!(where_pos < returning_pos);
    }

    #[test]
    fn test_delete_with_null_check() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::IsNull("deleted_at".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IS NULL"));
        assert!(params.is_empty()); // IS NULL doesn't have params
    }

    #[test]
    fn test_delete_with_not_null_check() {
        let op = DeleteOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::IsNotNull("email".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IS NOT NULL"));
        assert!(params.is_empty());
    }
}
