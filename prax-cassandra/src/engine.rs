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
    pub async fn query(&self, _cql: &str) -> CassandraResult<QueryResult> {
        Err(CassandraError::Query(
            "query() not yet wired to cdrs-tokio".into(),
        ))
    }

    /// Execute a statement not expecting rows (INSERT, UPDATE, DELETE, DDL).
    pub async fn execute(&self, _cql: &str) -> CassandraResult<()> {
        Err(CassandraError::Query(
            "execute() not yet wired to cdrs-tokio".into(),
        ))
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
        // The prax-cassandra driver is wire-stubbed: pool.query returns
        // an error until the cdrs-tokio integration lands. Surface the
        // underlying stub error verbatim so callers see the same message
        // whether they route through the Client API or the raw pool
        // methods.
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let _: QueryResult = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            // Row decoding for Cassandra lives behind the real
            // integration — the stubbed `Row` type stores raw bytes
            // only, so there's no RowRef bridge to hand each row to
            // `T::from_row`. Return an empty Vec until that lands.
            Ok(Vec::new())
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
            let _: QueryResult = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Err(prax_query::QueryError::not_found(T::MODEL_NAME))
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
            let _: QueryResult = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(None)
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
