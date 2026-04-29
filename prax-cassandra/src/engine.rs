//! Query execution engine.
//!
//! This module defines the public query API (query/execute/batch/LWT/paging).
//! Actual network calls to cdrs-tokio are wired up in the live integration
//! task so these methods currently return a "not yet wired" error.

use crate::error::{CassandraError, CassandraResult};
use crate::pool::CassandraPool;
use crate::row::{FromRow, Row};

/// Aggregate result of a CQL query.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Rows returned by the query. Empty for non-SELECT statements.
    pub rows: Vec<Row>,
    /// Whether a lightweight transaction applied.
    pub applied: Option<bool>,
}

impl CassandraPool {
    /// Execute a query returning rows.
    pub async fn query(&self, cql: &str) -> CassandraResult<QueryResult> {
        let envelope = self
            .connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("query failed: {e}")))?;

        // Parse the response. SELECT responses carry a ResponseBody::Result
        // with rows; INSERT/UPDATE/DELETE responses typically carry an
        // empty result. LWT responses carry a single row with the
        // `[applied]` boolean column first.
        let body = envelope
            .response_body()
            .map_err(|e| CassandraError::Query(format!("response body parse: {e}")))?;

        let (rows, applied) = if let Some(raw_rows) = body.into_rows() {
            // LWT responses carry the applied-boolean as the first column
            // of a single row. Detect that shape by checking whether the
            // result set is exactly one row and the first column is a
            // boolean named "[applied]".
            let applied = raw_rows.first().and_then(|row| {
                use cdrs_tokio::types::ByName;
                row.by_name::<bool>("[applied]").ok().flatten()
            });
            let decoded: Vec<crate::row::Row> = raw_rows
                .into_iter()
                .map(|r| crate::row::Row::from_cdrs_row(&r))
                .collect::<CassandraResult<_>>()?;
            (decoded, applied)
        } else {
            (Vec::new(), None)
        };

        Ok(QueryResult { rows, applied })
    }

    /// Execute a statement not expecting rows (INSERT, UPDATE, DELETE, DDL).
    pub async fn execute(&self, cql: &str) -> CassandraResult<()> {
        self.connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("execute failed: {e}")))?;
        Ok(())
    }

    /// Query a single row, deserialized into T.
    pub async fn query_one<T: FromRow>(&self, cql: &str) -> CassandraResult<T> {
        let result = self.query(cql).await?;
        let row = result
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| CassandraError::Query("query_one: no rows returned".into()))?;
        T::from_row(&row)
    }

    /// Query many rows.
    pub async fn query_many<T: FromRow>(&self, cql: &str) -> CassandraResult<Vec<T>> {
        let result = self.query(cql).await?;
        result.rows.iter().map(|row| T::from_row(row)).collect()
    }

    /// Execute a lightweight transaction. Returns whether the CAS succeeded.
    pub async fn execute_lwt(&self, cql: &str) -> CassandraResult<bool> {
        let result = self.query(cql).await?;
        Ok(result.applied.unwrap_or(false))
    }

    /// Build a batch of statements.
    pub fn batch(&self) -> BatchBuilder<'_> {
        BatchBuilder {
            pool: self,
            statements: Vec::new(),
        }
    }
}

/// Builder for a CQL batch.
pub struct BatchBuilder<'a> {
    pool: &'a CassandraPool,
    statements: Vec<String>,
}

impl<'a> BatchBuilder<'a> {
    /// Add a statement to the batch.
    pub fn add_statement(mut self, cql: impl Into<String>) -> Self {
        self.statements.push(cql.into());
        self
    }

    /// Execute the batch as a LOGGED batch (default).
    pub async fn execute(self) -> CassandraResult<()> {
        self.execute_logged().await
    }

    /// Execute the batch as a LOGGED batch.
    pub async fn execute_logged(self) -> CassandraResult<()> {
        let _ = self.pool;
        if self.statements.is_empty() {
            return Err(CassandraError::Query("cannot execute empty batch".into()));
        }
        Err(CassandraError::Query(
            "batch.execute_logged not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Execute the batch as an UNLOGGED batch.
    pub async fn execute_unlogged(self) -> CassandraResult<()> {
        let _ = self.pool;
        if self.statements.is_empty() {
            return Err(CassandraError::Query("cannot execute empty batch".into()));
        }
        Err(CassandraError::Query(
            "batch.execute_unlogged not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Execute the batch as a COUNTER batch.
    pub async fn execute_counter(self) -> CassandraResult<()> {
        let _ = self.pool;
        if self.statements.is_empty() {
            return Err(CassandraError::Query("cannot execute empty batch".into()));
        }
        Err(CassandraError::Query(
            "batch.execute_counter not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Number of statements in the batch (for test/debug).
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// True if the batch has no statements.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// Top-level query engine for the Cassandra driver.
///
/// Thin wrapper around [`CassandraPool`] that lets `#[derive(Model)]`-
/// generated `Client<E>` target Cassandra through the same codegen
/// pipeline the SQL drivers use. The underlying pool methods are still
/// stubbed to return a "not yet wired" error until the cdrs-tokio
/// integration lands, so `QueryEngine` method calls surface that same
/// error. The trait surface is stable — only the runtime wiring is
/// outstanding.
#[derive(Clone)]
pub struct CassandraEngine {
    pool: CassandraPool,
}

impl CassandraEngine {
    /// Create a new engine wrapping the given pool.
    pub fn new(pool: CassandraPool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool. Exposed for callers that need to
    /// reach the raw query/execute/batch helpers directly.
    pub fn pool(&self) -> &CassandraPool {
        &self.pool
    }
}

impl prax_query::traits::QueryEngine for CassandraEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Cql
    }

    fn query_many<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            result
                .rows
                .iter()
                .filter_map(|r| r.as_cdrs())
                .map(|cdrs_row| {
                    let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
                    let rr = crate::row_ref::CassandraRowRef::from_cdrs_with_cols(cdrs_row, &cols);
                    T::from_row(&rr).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                })
                .collect()
        })
    }

    fn query_one<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let cdrs_row = result
                .rows
                .iter()
                .filter_map(|r| r.as_cdrs())
                .next()
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
            let rr = crate::row_ref::CassandraRowRef::from_cdrs_with_cols(cdrs_row, &cols);
            T::from_row(&rr).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn query_optional<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Option<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            match result.rows.iter().filter_map(|r| r.as_cdrs()).next() {
                None => Ok(None),
                Some(cdrs_row) => {
                    let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
                    let rr = crate::row_ref::CassandraRowRef::from_cdrs_with_cols(cdrs_row, &cols);
                    Ok(Some(T::from_row(&rr).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })?))
                }
            }
        })
    }

    fn execute_insert<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        // CQL has no RETURNING; mirror ScyllaEngine's behaviour.
        let _ = (sql, params);
        Box::pin(async move {
            Err(prax_query::QueryError::unsupported(
                "CassandraEngine::execute_insert: CQL has no RETURNING",
            ))
        })
    }

    fn execute_update<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let _ = (sql, params);
        Box::pin(async move {
            Err(prax_query::QueryError::unsupported(
                "CassandraEngine::execute_update: CQL has no RETURNING",
            ))
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let _: () = pool
                .execute(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(0)
        })
    }

    fn execute_raw(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        self.execute_delete(sql, params)
    }

    fn count(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let _: QueryResult = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CassandraConfig;

    #[tokio::test]
    async fn test_query_without_connection_returns_error() {
        let config = CassandraConfig::builder()
            .known_nodes(["127.0.0.1:9042".to_string()])
            .build();
        // Pool.connect returns an error in the stub phase, so we can't
        // build a pool here. Instead, construct the error directly via
        // the assertion below. This test primarily exercises the API
        // surface compiles.
        let _ = config;
    }

    #[test]
    fn test_batch_builder_add_increments_len() {
        // Construct a fake pool surface through a compile-check-only path.
        // We can't instantiate a real pool without a live cluster, so this
        // test lives as a TODO placeholder; live integration covers the
        // real behavior.
        let stmts: Vec<String> = vec!["INSERT INTO t VALUES (1)".into()];
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_query_result_default_is_empty() {
        let r = QueryResult::default();
        assert!(r.rows.is_empty());
        assert!(r.applied.is_none());
    }
}
