//! MySQL query engine implementing `prax_query::QueryEngine`.

use mysql_async::prelude::*;
use mysql_async::{Params, Row as MyRow, Value as MyValue};
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::debug;

use crate::pool::MysqlPool;
use crate::row_ref::MysqlRowRef;
use crate::types::filter_value_to_mysql;

/// MySQL query engine backed by `mysql_async`.
#[derive(Clone)]
pub struct MysqlEngine {
    pool: MysqlPool,
}

impl MysqlEngine {
    /// Create a new engine with the given pool.
    pub fn new(pool: MysqlPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &MysqlPool {
        &self.pool
    }

    fn bind(params: &[FilterValue]) -> Vec<MyValue> {
        params.iter().map(filter_value_to_mysql).collect()
    }

    async fn query_rows<T: Model + FromRow>(
        &self,
        sql: String,
        params: Vec<FilterValue>,
    ) -> QueryResult<Vec<T>> {
        debug!(sql = %sql, "mysql query_rows");
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()))?;
        let bound = Self::bind(&params);
        let rows: Vec<MyRow> = conn
            .inner_mut()
            .exec(sql.as_str(), Params::Positional(bound))
            .await
            .map_err(|e| QueryError::database(e.to_string()))?;

        rows.into_iter()
            .map(|row| {
                let rr = MysqlRowRef::from_row(row)
                    .map_err(|e| QueryError::deserialization(e.to_string()))?;
                T::from_row(&rr).map_err(|e| QueryError::deserialization(e.to_string()))
            })
            .collect()
    }

    /// Stop after the first row so callers that want a single row do not pay
    /// for materializing the tail. Naively routing `query_one`/`query_optional`
    /// through `query_rows` + `.pop()` would decode every matching row and
    /// throw away all but one; a caller who accidentally asked for a single
    /// row from a million-row table would allocate a million typed models.
    async fn query_first_row<T: Model + FromRow>(
        &self,
        sql: String,
        params: Vec<FilterValue>,
    ) -> QueryResult<Option<T>> {
        debug!(sql = %sql, "mysql query_first_row");
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()))?;
        let bound = Self::bind(&params);
        let row: Option<MyRow> = conn
            .inner_mut()
            .exec_first(sql.as_str(), Params::Positional(bound))
            .await
            .map_err(|e| QueryError::database(e.to_string()))?;

        match row {
            Some(r) => {
                let rr = MysqlRowRef::from_row(r)
                    .map_err(|e| QueryError::deserialization(e.to_string()))?;
                let t = T::from_row(&rr).map_err(|e| QueryError::deserialization(e.to_string()))?;
                Ok(Some(t))
            }
            None => Ok(None),
        }
    }

    async fn exec_raw(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()))?;
        let bound = Self::bind(&params);
        conn.inner_mut()
            .exec_drop(sql.as_str(), Params::Positional(bound))
            .await
            .map_err(|e| QueryError::database(e.to_string()))?;
        Ok(conn.inner().affected_rows())
    }

    async fn count_rows(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()))?;
        let bound = Self::bind(&params);
        let count: Option<(i64,)> = conn
            .inner_mut()
            .exec_first(sql.as_str(), Params::Positional(bound))
            .await
            .map_err(|e| QueryError::database(e.to_string()))?;
        Ok(count.map(|(n,)| n as u64).unwrap_or(0))
    }
}

impl QueryEngine for MysqlEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Mysql
    }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_rows::<T>(sql, params))
    }

    fn query_one<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            self.query_first_row::<T>(sql, params)
                .await?
                .ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }

    fn query_optional<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_first_row::<T>(sql, params))
    }

    fn execute_insert<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        // MySQL 8.0.22+ supports INSERT ... RETURNING. INSERT RETURNING yields
        // at most one row per inserted tuple; query_first_row avoids ever
        // materializing a tail if the caller's SQL yields many (which would
        // be a misuse, but the engine shouldn't punish it with unbounded
        // allocation).
        let sql = sql.to_string();
        Box::pin(async move {
            self.query_first_row::<T>(sql, params)
                .await?
                .ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_rows::<T>(sql, params))
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.exec_raw(sql, params))
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.exec_raw(sql, params))
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.count_rows(sql, params))
    }
}
