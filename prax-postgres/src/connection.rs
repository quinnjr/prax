//! PostgreSQL connection wrapper.

use std::sync::Arc;

use deadpool_postgres::Object;
use tokio_postgres::Row;
use tracing::{debug, trace};

use crate::error::PgResult;
use crate::statement::PreparedStatementCache;

/// A wrapper around a PostgreSQL connection with statement caching.
pub struct PgConnection {
    client: Object,
    statement_cache: Arc<PreparedStatementCache>,
}

impl PgConnection {
    /// Create a new connection wrapper.
    pub(crate) fn new(client: Object, statement_cache: Arc<PreparedStatementCache>) -> Self {
        Self {
            client,
            statement_cache,
        }
    }

    /// Execute a query and return all rows.
    pub async fn query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Vec<Row>> {
        trace!(sql = %sql, "Executing query");

        // Try to get a cached prepared statement
        let stmt = self
            .statement_cache
            .get_or_prepare(&self.client, sql)
            .await?;

        let rows = self.client.query(&stmt, params).await?;
        Ok(rows)
    }

    /// Execute a query and return exactly one row.
    pub async fn query_one(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Row> {
        trace!(sql = %sql, "Executing query_one");

        let stmt = self
            .statement_cache
            .get_or_prepare(&self.client, sql)
            .await?;

        let row = self.client.query_one(&stmt, params).await?;
        Ok(row)
    }

    /// Execute a query and return zero or one row.
    pub async fn query_opt(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Option<Row>> {
        trace!(sql = %sql, "Executing query_opt");

        let stmt = self
            .statement_cache
            .get_or_prepare(&self.client, sql)
            .await?;

        let row = self.client.query_opt(&stmt, params).await?;
        Ok(row)
    }

    /// Execute a statement and return the number of affected rows.
    pub async fn execute(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<u64> {
        trace!(sql = %sql, "Executing statement");

        let stmt = self
            .statement_cache
            .get_or_prepare(&self.client, sql)
            .await?;

        let count = self.client.execute(&stmt, params).await?;
        Ok(count)
    }

    /// Execute a batch of statements in a single round-trip.
    pub async fn batch_execute(&self, sql: &str) -> PgResult<()> {
        trace!(sql = %sql, "Executing batch");
        self.client.batch_execute(sql).await?;
        Ok(())
    }

    /// Begin a transaction.
    pub async fn transaction(&mut self) -> PgResult<PgTransaction<'_>> {
        debug!("Beginning transaction");
        let txn = self.client.transaction().await?;
        Ok(PgTransaction {
            txn,
            statement_cache: self.statement_cache.clone(),
        })
    }

    /// Get the underlying tokio-postgres client.
    ///
    /// This is useful for advanced operations not covered by this wrapper.
    pub fn inner(&self) -> &Object {
        &self.client
    }

    /// Execute a query using the prepared statement cache.
    ///
    /// This is an alias for `query` that makes it explicit that statement caching
    /// is being used. All query methods already use prepared statement caching,
    /// but this method name makes it more explicit for benchmark comparisons.
    #[inline]
    pub async fn query_cached(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Vec<Row>> {
        self.query(sql, params).await
    }

    /// Execute a raw query without using the prepared statement cache.
    ///
    /// This is useful for one-off queries where the overhead of preparing
    /// a statement isn't worth it.
    pub async fn query_raw(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Vec<Row>> {
        trace!(sql = %sql, "Executing raw query (no statement cache)");
        let rows = self.client.query(sql, params).await?;
        Ok(rows)
    }

    /// Execute a raw query and return zero or one row without using statement cache.
    pub async fn query_opt_raw(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Option<Row>> {
        trace!(sql = %sql, "Executing raw query_opt (no statement cache)");
        let row = self.client.query_opt(sql, params).await?;
        Ok(row)
    }
}

/// A PostgreSQL transaction.
pub struct PgTransaction<'a> {
    txn: deadpool_postgres::Transaction<'a>,
    statement_cache: Arc<PreparedStatementCache>,
}

impl<'a> PgTransaction<'a> {
    /// Execute a query and return all rows.
    pub async fn query(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Vec<Row>> {
        trace!(sql = %sql, "Executing query in transaction");

        let stmt = self
            .statement_cache
            .get_or_prepare_in_txn(&self.txn, sql)
            .await?;

        let rows = self.txn.query(&stmt, params).await?;
        Ok(rows)
    }

    /// Execute a query and return exactly one row.
    pub async fn query_one(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Row> {
        let stmt = self
            .statement_cache
            .get_or_prepare_in_txn(&self.txn, sql)
            .await?;

        let row = self.txn.query_one(&stmt, params).await?;
        Ok(row)
    }

    /// Execute a query and return zero or one row.
    pub async fn query_opt(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<Option<Row>> {
        let stmt = self
            .statement_cache
            .get_or_prepare_in_txn(&self.txn, sql)
            .await?;

        let row = self.txn.query_opt(&stmt, params).await?;
        Ok(row)
    }

    /// Execute a statement and return the number of affected rows.
    pub async fn execute(
        &self,
        sql: &str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> PgResult<u64> {
        let stmt = self
            .statement_cache
            .get_or_prepare_in_txn(&self.txn, sql)
            .await?;

        let count = self.txn.execute(&stmt, params).await?;
        Ok(count)
    }

    /// Create a savepoint.
    pub async fn savepoint(&mut self, name: &str) -> PgResult<()> {
        debug!(name = %name, "Creating savepoint");
        self.txn
            .batch_execute(&format!("SAVEPOINT {}", name))
            .await?;
        Ok(())
    }

    /// Rollback to a savepoint.
    pub async fn rollback_to(&mut self, name: &str) -> PgResult<()> {
        debug!(name = %name, "Rolling back to savepoint");
        self.txn
            .batch_execute(&format!("ROLLBACK TO SAVEPOINT {}", name))
            .await?;
        Ok(())
    }

    /// Release a savepoint.
    pub async fn release_savepoint(&mut self, name: &str) -> PgResult<()> {
        debug!(name = %name, "Releasing savepoint");
        self.txn
            .batch_execute(&format!("RELEASE SAVEPOINT {}", name))
            .await?;
        Ok(())
    }

    /// Commit the transaction.
    pub async fn commit(self) -> PgResult<()> {
        debug!("Committing transaction");
        self.txn.commit().await?;
        Ok(())
    }

    /// Rollback the transaction.
    pub async fn rollback(self) -> PgResult<()> {
        debug!("Rolling back transaction");
        self.txn.rollback().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require a real PostgreSQL connection
    // Unit tests for connection wrapper are limited without mocking
}
