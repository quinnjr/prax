//! Count operation for counting records.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::{Filter, FilterValue};
use crate::traits::{Model, QueryEngine};

/// A count operation for counting records.
///
/// # Example
///
/// ```rust,ignore
/// let count = client
///     .user()
///     .count()
///     .r#where(user::active::equals(true))
///     .exec()
///     .await?;
/// ```
pub struct CountOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    distinct: Option<String>,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model> CountOperation<E, M> {
    /// Create a new Count operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
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

    /// Count distinct values of a column.
    pub fn distinct(mut self, column: impl Into<String>) -> Self {
        self.distinct = Some(column.into());
        self
    }

    /// Build the SQL query.
    pub fn build_sql(
        &self,
        dialect: &dyn crate::dialect::SqlDialect,
    ) -> (String, Vec<FilterValue>) {
        let (where_sql, params) = self.filter.to_sql(0, dialect);

        let mut sql = String::new();

        // SELECT COUNT clause
        sql.push_str("SELECT COUNT(");
        match &self.distinct {
            Some(col) => {
                sql.push_str("DISTINCT ");
                sql.push_str(col);
            }
            None => sql.push('*'),
        }
        sql.push(')');

        // FROM clause
        sql.push_str(" FROM ");
        sql.push_str(M::TABLE_NAME);

        // WHERE clause
        if !self.filter.is_none() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_sql);
        }

        (sql, params)
    }

    /// Execute the count query.
    pub async fn exec(self) -> QueryResult<u64> {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.count(&sql, params).await
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
        count_result: u64,
    }

    impl MockEngine {
        fn new() -> Self {
            Self { count_result: 0 }
        }

        fn with_count(count: u64) -> Self {
            Self {
                count_result: count,
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
            let count = self.count_result;
            Box::pin(async move { Ok(count) })
        }
    }

    // ========== Construction Tests ==========

    #[test]
    fn test_count_new() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT COUNT(*)"));
        assert!(sql.contains("FROM test_models"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_count_basic() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(sql, "SELECT COUNT(*) FROM test_models");
        assert!(params.is_empty());
    }

    // ========== Filter Tests ==========

    #[test]
    fn test_count_with_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""active" = $1"#));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_count_with_compound_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
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
    fn test_count_with_or_filter() {
        let op =
            CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(Filter::or([
                Filter::Equals("role".into(), FilterValue::String("admin".to_string())),
                Filter::Equals("role".into(), FilterValue::String("moderator".to_string())),
            ]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("OR"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_count_with_in_filter() {
        let op =
            CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(Filter::In(
                "status".into(),
                vec![
                    FilterValue::String("pending".to_string()),
                    FilterValue::String("processing".to_string()),
                    FilterValue::String("completed".to_string()),
                ],
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IN"));
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_count_without_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_count_with_null_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::IsNull("deleted_at".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("IS NULL"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_count_with_not_null_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::IsNotNull("verified_at".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IS NOT NULL"));
        assert!(params.is_empty());
    }

    // ========== Distinct Tests ==========

    #[test]
    fn test_count_distinct() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).distinct("email");

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("COUNT(DISTINCT email)"));
        assert!(!sql.contains("COUNT(*)"));
    }

    #[test]
    fn test_count_distinct_with_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)))
            .distinct("user_id");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("COUNT(DISTINCT user_id)"));
        assert!(sql.contains("WHERE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_count_distinct_replaces() {
        // Later distinct should replace the previous one
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .distinct("email")
            .distinct("user_id");

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("COUNT(DISTINCT user_id)"));
        assert!(!sql.contains("COUNT(DISTINCT email)"));
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_count_sql_structure() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        let count_pos = sql.find("COUNT").unwrap();
        let from_pos = sql.find("FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();

        assert!(count_pos < from_pos);
        assert!(from_pos < where_pos);
    }

    #[test]
    fn test_count_table_name() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new());
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Async Execution Tests ==========

    #[tokio::test]
    async fn test_count_exec() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::with_count(42));

        let result = op.exec().await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_count_exec_with_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::with_count(10))
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)));

        let result = op.exec().await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 10);
    }

    #[tokio::test]
    async fn test_count_exec_zero() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new());

        let result = op.exec().await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_count_method_chaining() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new())
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("active".to_string()),
            ))
            .distinct("user_id");

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("COUNT(DISTINCT user_id)"));
        assert!(sql.contains("WHERE"));
        assert_eq!(params.len(), 1);
    }

    // ========== Edge Cases ==========

    #[test]
    fn test_count_with_like_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(
            Filter::Contains(
                "email".into(),
                FilterValue::String("@example.com".to_string()),
            ),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_count_with_starts_with() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(
            Filter::StartsWith("name".into(), FilterValue::String("A".to_string())),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_count_with_not_filter() {
        let op = CountOperation::<MockEngine, TestModel>::new(MockEngine::new()).r#where(
            Filter::Not(Box::new(Filter::Equals(
                "status".into(),
                FilterValue::String("deleted".to_string()),
            ))),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("NOT"));
        assert_eq!(params.len(), 1);
    }
}
