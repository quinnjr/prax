//! FindUnique operation for querying a single record by unique constraint.

use std::marker::PhantomData;

use crate::error::QueryResult;
use crate::filter::Filter;
use crate::relations::IncludeSpec;
use crate::traits::{Model, ModelRelationLoader, QueryEngine};
use crate::types::Select;

/// A query operation that finds a single record by unique constraint.
///
/// # Example
///
/// ```rust,ignore
/// let user = client
///     .user()
///     .find_unique()
///     .r#where(user::id::equals(1))
///     .exec()
///     .await?;
/// ```
pub struct FindUniqueOperation<E: QueryEngine, M: Model> {
    engine: E,
    filter: Filter,
    select: Select,
    /// Relations to eager-load alongside the unique lookup. Mirrors
    /// the `find_many` include list — even though the result is a
    /// single row, the loader operates on a 1-element slice.
    includes: Vec<IncludeSpec>,
    _model: PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> FindUniqueOperation<E, M> {
    /// Create a new FindUnique operation.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            filter: Filter::None,
            select: Select::All,
            includes: Vec::new(),
            _model: PhantomData,
        }
    }

    /// Add a filter condition (should be on unique fields).
    pub fn r#where(mut self, filter: impl Into<Filter>) -> Self {
        self.filter = filter.into();
        self
    }

    /// Select specific fields.
    pub fn select(mut self, select: impl Into<Select>) -> Self {
        self.select = select.into();
        self
    }

    /// Eager-load a relation alongside the unique lookup.
    ///
    /// Queued includes dispatch through the model's
    /// [`ModelRelationLoader`] after the main SELECT returns.
    pub fn include(mut self, spec: IncludeSpec) -> Self {
        self.includes.push(spec);
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

        // LIMIT 1 for unique query
        sql.push_str(" LIMIT 1");

        (sql, params)
    }

    /// Execute the query and return the result (errors if not found).
    pub async fn exec(self) -> QueryResult<M>
    where
        M: Send + 'static + ModelRelationLoader<E>,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        let row = self.engine.query_one::<M>(&sql, params).await?;
        // Wrap the single row in a 1-element slice for the loader.
        // `into_iter().next()` below reads it back out without any
        // extra clone.
        let mut parents = vec![row];
        for spec in &self.includes {
            <M as ModelRelationLoader<E>>::load_relation(&self.engine, &mut parents, spec).await?;
        }
        Ok(parents.into_iter().next().expect("1-element vec"))
    }

    /// Execute the query and return an optional result.
    pub async fn exec_optional(self) -> QueryResult<Option<M>>
    where
        M: Send + 'static + ModelRelationLoader<E>,
    {
        let dialect = self.engine.dialect();
        let (sql, params) = self.build_sql(dialect);
        match self.engine.query_optional::<M>(&sql, params).await? {
            None => Ok(None),
            Some(row) => {
                let mut parents = vec![row];
                for spec in &self.includes {
                    <M as ModelRelationLoader<E>>::load_relation(&self.engine, &mut parents, spec)
                        .await?;
                }
                Ok(parents.into_iter().next())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::QueryError;
    use crate::filter::FilterValue;

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

    // ========== Construction and Basic Tests ==========

    #[test]
    fn test_find_unique_new() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM test_models"));
        assert!(sql.contains("LIMIT 1"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_find_unique_basic() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM test_models"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""id" = $1"#));
        assert!(sql.contains("LIMIT 1"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_unique_by_email() {
        let op =
            FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::Equals(
                "email".into(),
                FilterValue::String("test@example.com".to_string()),
            ));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains(r#""email" = $1"#));
        assert_eq!(params.len(), 1);
    }

    // ========== Select Tests ==========

    #[test]
    fn test_find_unique_with_select() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id, name FROM"));
        assert!(!sql.contains("SELECT *"));
    }

    #[test]
    fn test_find_unique_select_single_field() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id FROM"));
    }

    #[test]
    fn test_find_unique_select_all_explicitly() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::All);

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT * FROM"));
    }

    // ========== Filter Tests ==========

    #[test]
    fn test_find_unique_with_compound_filter() {
        let op =
            FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::and([
                Filter::Equals(
                    "email".into(),
                    FilterValue::String("test@example.com".to_string()),
                ),
                Filter::Equals("tenant_id".into(), FilterValue::Int(1)),
            ]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("WHERE"));
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_find_unique_without_filter() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(!sql.contains("WHERE"));
        assert!(params.is_empty());
    }

    #[test]
    fn test_find_unique_with_none_filter() {
        let op =
            FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine).r#where(Filter::None);

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        // Filter::None should not produce WHERE clause
        assert!(!sql.contains("WHERE"));
        assert!(params.is_empty());
    }

    // ========== SQL Structure Tests ==========

    #[test]
    fn test_find_unique_sql_order() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id", "name"]));

        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        // Check SQL structure order
        let select_pos = sql.find("SELECT").unwrap();
        let from_pos = sql.find("FROM").unwrap();
        let where_pos = sql.find("WHERE").unwrap();
        let limit_pos = sql.find("LIMIT 1").unwrap();

        assert!(select_pos < from_pos);
        assert!(from_pos < where_pos);
        assert!(where_pos < limit_pos);
    }

    #[test]
    fn test_find_unique_table_name() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine);
        let (sql, _) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("test_models"));
    }

    // ========== Async Execution Tests ==========

    #[tokio::test]
    async fn test_find_unique_exec() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec().await;

        // MockEngine returns not_found error
        assert!(result.is_err());
        assert!(result.unwrap_err().is_not_found());
    }

    #[tokio::test]
    async fn test_find_unique_exec_optional() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)));

        let result = op.exec_optional().await;

        // MockEngine returns Ok(None)
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // ========== Parameter Tests ==========

    #[test]
    fn test_find_unique_with_string_param() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine).r#where(
            Filter::Equals("name".into(), FilterValue::String("Alice".to_string())),
        );

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::String("Alice".to_string()));
    }

    #[test]
    fn test_find_unique_with_int_param() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(42)));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::Int(42));
    }

    #[test]
    fn test_find_unique_with_boolean_param() {
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::Bool(true));
    }

    // ========== Method Chaining Tests ==========

    #[test]
    fn test_find_unique_method_chaining() {
        // Test that methods return Self and can be chained
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .select(Select::fields(["id", "name"]));

        let (sql, params) = op.build_sql(&crate::dialect::Postgres);

        assert!(sql.contains("SELECT id, name"));
        assert!(sql.contains("WHERE"));
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_find_unique_replace_filter() {
        // Later where_ calls should replace the filter
        let op = FindUniqueOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
            .r#where(Filter::Equals("id".into(), FilterValue::Int(2)));

        let (_, params) = op.build_sql(&crate::dialect::Postgres);

        // Should only have the second filter's param
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], FilterValue::Int(2));
    }
}
