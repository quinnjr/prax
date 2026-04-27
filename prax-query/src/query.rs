//! Query builder entry point.

use crate::filter::{Filter, FilterValue};
use crate::operations::*;
use crate::traits::{Model, QueryEngine};

/// The main query builder that provides access to all query operations.
///
/// This is typically not used directly - instead, use the generated
/// model accessors (e.g., `client.user()`).
pub struct QueryBuilder<E: QueryEngine, M: Model> {
    engine: E,
    _model: std::marker::PhantomData<M>,
}

impl<E: QueryEngine, M: Model + crate::row::FromRow> QueryBuilder<E, M> {
    /// Create a new query builder.
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            _model: std::marker::PhantomData,
        }
    }

    /// Start a find_many query.
    pub fn find_many(&self) -> FindManyOperation<E, M> {
        FindManyOperation::new(self.engine.clone())
    }

    /// Start a find_unique query.
    pub fn find_unique(&self) -> FindUniqueOperation<E, M> {
        FindUniqueOperation::new(self.engine.clone())
    }

    /// Start a find_first query.
    pub fn find_first(&self) -> FindFirstOperation<E, M> {
        FindFirstOperation::new(self.engine.clone())
    }

    /// Start a create operation.
    pub fn create(&self) -> CreateOperation<E, M> {
        CreateOperation::new(self.engine.clone())
    }

    /// Start an update operation.
    pub fn update(&self) -> UpdateOperation<E, M> {
        UpdateOperation::new(self.engine.clone())
    }

    /// Start a delete operation.
    pub fn delete(&self) -> DeleteOperation<E, M> {
        DeleteOperation::new(self.engine.clone())
    }

    /// Start an upsert operation.
    pub fn upsert(&self) -> UpsertOperation<E, M> {
        UpsertOperation::new(self.engine.clone())
    }

    /// Start a count operation.
    pub fn count(&self) -> CountOperation<E, M> {
        CountOperation::new(self.engine.clone())
    }

    /// Execute a raw SQL query.
    pub async fn raw(&self, sql: &str, params: Vec<FilterValue>) -> crate::error::QueryResult<u64> {
        self.engine.execute_raw(sql, params).await
    }

    /// Find a record by ID.
    ///
    /// This is a convenience method for `find_unique().r#where(id::equals(id))`.
    pub fn find_by_id(&self, id: impl Into<FilterValue>) -> FindUniqueOperation<E, M> {
        let pk = *M::PRIMARY_KEY.first().unwrap_or(&"id");
        self.find_unique()
            .r#where(Filter::Equals(pk.into(), id.into()))
    }
}

impl<E: QueryEngine, M: Model> Clone for QueryBuilder<E, M> {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            _model: std::marker::PhantomData,
        }
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
    struct MockEngine;

    impl QueryEngine for MockEngine {
        fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
            &crate::dialect::Postgres
        }

        fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }

        fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<T>> {
            Box::pin(async { Err(QueryError::not_found("test")) })
        }

        fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }

        fn count(
            &self,
            _sql: &str,
            _params: Vec<FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    #[test]
    fn test_query_builder_find_many() {
        let qb = QueryBuilder::<MockEngine, TestModel>::new(MockEngine);
        let op = qb.find_many();
        let (sql, _) = op.build_sql();
        assert!(sql.contains("SELECT * FROM test_models"));
    }

    #[test]
    fn test_query_builder_find_by_id() {
        let qb = QueryBuilder::<MockEngine, TestModel>::new(MockEngine);
        let op = qb.find_by_id(1i32);
        let (sql, params) = op.build_sql();
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("id = $1"));
        assert_eq!(params.len(), 1);
    }
}
