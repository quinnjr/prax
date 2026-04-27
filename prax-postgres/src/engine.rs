//! PostgreSQL query engine implementation.

use std::marker::PhantomData;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::debug;

use crate::pool::PgPool;
use crate::types::filter_value_to_sql;

/// PostgreSQL query engine that implements the Prax QueryEngine trait.
#[derive(Clone)]
pub struct PgEngine {
    pool: PgPool,
}

impl PgEngine {
    /// Create a new PostgreSQL engine with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Convert filter values to PostgreSQL parameters.
    #[allow(clippy::result_large_err)]
    fn to_params(
        values: &[FilterValue],
    ) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>, prax_query::QueryError>
    {
        values
            .iter()
            .map(|v| {
                filter_value_to_sql(v).map_err(|e| prax_query::QueryError::database(e.to_string()))
            })
            .collect()
    }
}

impl QueryEngine for PgEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Postgres
    }

    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_many");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            crate::deserialize::rows_into::<T>(rows)
        })
    }

    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_one");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = conn.query_one(&sql, &param_refs).await.map_err(|e| {
                if e.to_string().contains("no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(e.to_string())
                }
            })?;

            crate::deserialize::row_into::<T>(row)
        })
    }

    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_optional");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = conn
                .query_opt(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            row.map(crate::deserialize::row_into::<T>).transpose()
        })
    }

    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing insert");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            crate::deserialize::row_into::<T>(row)
        })
    }

    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing update");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let rows = conn
                .query(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            crate::deserialize::rows_into::<T>(rows)
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing delete");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(count)
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing raw SQL");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let count = conn
                .execute(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(count)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing count");

            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| prax_query::QueryError::connection(e.to_string()))?;

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = conn
                .query_one(&sql, &param_refs)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let count: i64 = row.get(0);
            Ok(count as u64)
        })
    }
}

/// A typed query builder that uses the PostgreSQL engine.
pub struct PgQueryBuilder<T: Model> {
    engine: PgEngine,
    _marker: PhantomData<T>,
}

impl<T: Model> PgQueryBuilder<T> {
    /// Create a new query builder.
    pub fn new(engine: PgEngine) -> Self {
        Self {
            engine,
            _marker: PhantomData,
        }
    }

    /// Get the underlying engine.
    pub fn engine(&self) -> &PgEngine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require a real PostgreSQL database
}
