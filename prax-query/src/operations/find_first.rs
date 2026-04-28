//! FindFirst operation for querying the first matching record.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::Filter;
use crate::traits::{Model, QueryEngine};
use crate::types::{OrderBy, Select};

/// A query operation that finds the first record matching the filter.
///
/// Unlike `FindUnique`, this doesn't require a unique constraint.
///
/// # Example
///
/// ```rust,ignore
/// let user = client
///     .user()
///     .find_first()
///     .r#where(user::email::contains("@example.com"))
///     .order_by(user::created_at::desc())
///     .exec()
///     .await?;
/// ```
pub struct FindFirstOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    order_by: OrderBy,
    select: Select,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> FindFirstOperation<E, M> {
    /// Create a new FindFirst operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            order_by: OrderBy::none(),
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

    /// Set the order by clause.
    pub fn order_by(mut self, order: impl Into<OrderBy>) -> Self {
        self.order_by = order.into();
        self
    }

    /// Select specific fields.
    pub fn select(mut self, select: impl Into<Select>) -> Self {
        self.select = select.into();
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

        // LIMIT 1
        sql.push_str(" LIMIT 1");

        (sql, params)
    }

    /// Execute the query and return an optional result.
    pub async fn exec(self) -> QueryResult<Option<M>>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.query_optional::<M>(&sql, params).await
    }

    /// Execute the query and error if not found.
    pub async fn exec_required(self) -> QueryResult<M>
    where
        M: Send + 'static,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        self.engine.query_one::<M>(&sql, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;
    use crate::filter::FilterValue;
    use crate::types::OrderByField;

    #[derive(Debug)]
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
    fn test_find_first_new() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM test_models"));
        assert!(sql.contains("LIMIT 1"));
        assert!(params.is_empty());
    }

    // ========== Filter Tests ==========

    #[test]
    fn test_find_first_with_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine).r#where(
            Filter::Equals("status".into(), FilterValue::String("active".to_string())),
        );

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""status" = $1"#));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_first_with_compound_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "department".into(),
                FilterValue::String("engineering".to_string()),
            ))
            .r#where(Filter::Gt("salary".into(), FilterValue::Int(50000)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_first_with_or_filter() {
        let op =
            FindFirstOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::or([
                Filter::Equals("role".into(), FilterValue::String("admin".to_string())),
                Filter::Equals("role".into(), FilterValue::String("superadmin".to_string())),
            ]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("OR"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_first_without_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
        assert!(params.is_empty());
    }

    // ========== Order By Tests ==========

    #[test]
    fn test_find_first_with_order() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Gt("age".into(), FilterValue::Int(18)))
            .order_by(OrderByField::desc("created_at"));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(sql.contains("LIMIT 1"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_first_with_asc_order() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .order_by(OrderByField::asc("name"));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY name ASC"));
    }

    #[test]
    fn test_find_first_without_order() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("ORDER BY"));
    }

    #[test]
    fn test_find_first_order_replaces() {
        // Later order_by should replace the previous one
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .order_by(OrderByField::asc("name"))
            .order_by(OrderByField::desc("created_at"));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(!sql.contains("ORDER BY name"));
    }

    // ========== Select Tests ==========

    #[test]
    fn test_find_first_with_select() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .select(Select::fields(["id", "email"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id, email FROM"));
        assert!(!sql.contains("SELECT *"));
    }

    #[test]
    fn test_find_first_select_single_field() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .select(Select::fields(["count"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT count FROM"));
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_find_first_sql_structure() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .order_by(OrderByField::desc("created_at"))
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        // Check correct SQL clause ordering
        let select_pos = sql.find("SELECT").unwrap();
        let from_pos = sql.find("FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();
        let order_pos = sql.find("ORDER BY").unwrap();
        let limit_pos = sql.find("LIMIT 1").unwrap();

        assert!(select_pos < from_pos);
        assert!(from_pos < where_pos);
        assert!(where_pos < order_pos);
        assert!(order_pos < limit_pos);
    }

    #[test]
    fn test_find_first_table_name() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Async Execution Tests ==========

    #[tokio::test]
    async fn test_find_first_exec() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec().await;

        // MockEngine returns Ok(None)
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_find_first_exec_required() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec_required().await;

        // MockEngine returns not_found error
        assert!(result.is_err());
        assert!(result.unwrap_err().is_not_found());
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_find_first_full_chain() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals(
                "status".into(),
                FilterValue::String("active".to_string()),
            ))
            .order_by(OrderByField::desc("created_at"))
            .select(Select::fields(["id", "name", "email"]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id, name, email FROM"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(sql.contains("LIMIT 1"));
        assert_eq!(params.len(), 1);
    }

    // ========== Edge Cases ==========

    #[test]
    fn test_find_first_with_like_filter() {
        let op =
            FindFirstOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::Contains(
                "email".into(),
                FilterValue::String("@example.com".to_string()),
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("LIKE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_first_with_null_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::IsNull("deleted_at".into()));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("IS NULL"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_find_first_with_not_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::Not(
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
    fn test_find_first_with_in_filter() {
        let op = FindFirstOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::In(
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
}
