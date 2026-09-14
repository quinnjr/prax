//! PostgreSQL connection wrapper.

use std::sync::Arc;

use deadpool_postgres::Object;
use tokio_postgres::Row;
use tracing::{debug, trace};

use prax_query::sql::is_valid_sql_identifier;

use crate::error::{PgError, PgResult};
use crate::statement::PreparedStatementCache;

/// A wrapper around a PostgreSQL connection with statement caching.
pub struct PgConnection {
    client: Object,
    statement_cache: Arc<PreparedStatementCache>,
}

/// Whether a driver error is PostgreSQL's `0A000 "cached plan must not change
/// result type"`.
///
/// This is raised when a server-side prepared statement is executed after DDL
/// altered the result columns of a table it references (e.g. a pooled
/// connection that prepared the statement before an `ALTER TABLE … ADD
/// COLUMN`). It is transient: re-preparing against the current schema resolves
/// it. `0A000` is the shared `FEATURE_NOT_SUPPORTED` class, so the specific
/// message is required — other `0A000` conditions (genuinely unsupported
/// features) are terminal and must not trigger recovery.
fn is_stale_cached_plan(err: &tokio_postgres::Error) -> bool {
    // The human-readable message lives in the DbError, not in `Display`, which
    // renders a DB error as just "db error". Reading `to_string()` here would
    // never match the cached-plan text.
    match err.as_db_error() {
        Some(db) => {
            db.code() == &tokio_postgres::error::SqlState::FEATURE_NOT_SUPPORTED
                && is_stale_cached_plan_message(db.message())
        }
        None => false,
    }
}

/// The message half of [`is_stale_cached_plan`], split out so the gate can be
/// unit-tested without constructing a `tokio_postgres::Error` (which cannot be
/// built with a chosen SQLSTATE via the public API).
fn is_stale_cached_plan_message(msg: &str) -> bool {
    msg.contains("cached plan must not change result type")
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

        match self.client.query(&stmt, params).await {
            Ok(rows) => Ok(rows),
            Err(e) if is_stale_cached_plan(&e) => {
                let stmt = self.reprepare_after_stale_plan(sql).await?;
                let rows = self.client.query(&stmt, params).await?;
                Ok(rows)
            }
            Err(e) => Err(e.into()),
        }
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

        match self.client.query_one(&stmt, params).await {
            Ok(row) => Ok(row),
            Err(e) if is_stale_cached_plan(&e) => {
                let stmt = self.reprepare_after_stale_plan(sql).await?;
                let row = self.client.query_one(&stmt, params).await?;
                Ok(row)
            }
            Err(e) => Err(e.into()),
        }
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

        match self.client.query_opt(&stmt, params).await {
            Ok(row) => Ok(row),
            Err(e) if is_stale_cached_plan(&e) => {
                let stmt = self.reprepare_after_stale_plan(sql).await?;
                let row = self.client.query_opt(&stmt, params).await?;
                Ok(row)
            }
            Err(e) => Err(e.into()),
        }
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

        match self.client.execute(&stmt, params).await {
            Ok(count) => Ok(count),
            Err(e) if is_stale_cached_plan(&e) => {
                let stmt = self.reprepare_after_stale_plan(sql).await?;
                let count = self.client.execute(&stmt, params).await?;
                Ok(count)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Recover from a stale cached plan (`0A000`): drop the SQL from the
    /// statement cache and prepare it afresh, bypassing deadpool's per-
    /// connection cache so the new plan is built against the current schema.
    ///
    /// `prepare_cached` would hand back the same invalidated statement, so the
    /// retry must use the uncached `prepare`. The freshly prepared statement is
    /// what the caller re-executes; the cache is left empty for this SQL so the
    /// next ordinary call re-primes it via `get_or_prepare`.
    async fn reprepare_after_stale_plan(&self, sql: &str) -> PgResult<tokio_postgres::Statement> {
        debug!(
            sql = %sql,
            "Recovering from stale cached plan (0A000): re-preparing statement"
        );
        self.statement_cache.evict(sql);
        let stmt = self.client.prepare(sql).await?;
        Ok(stmt)
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

/// Maximum allowed savepoint name length (matches PostgreSQL's `NAMEDATALEN - 1`).
const MAX_SAVEPOINT_NAME_LEN: usize = 63;

/// Validate a savepoint name before it is interpolated into SQL.
///
/// Savepoint identifiers cannot be parameterized, so they must match the
/// whitelist pattern `^[A-Za-z_][A-Za-z0-9_]*$` to prevent SQL injection.
fn validate_savepoint_name(name: &str) -> PgResult<()> {
    let valid = name.len() <= MAX_SAVEPOINT_NAME_LEN && is_valid_sql_identifier(name);
    if !valid {
        return Err(PgError::query(format!("invalid savepoint name: {name:?}")));
    }
    Ok(())
}

/// A PostgreSQL transaction.
pub struct PgTransaction<'a> {
    txn: deadpool_postgres::Transaction<'a>,
    statement_cache: Arc<PreparedStatementCache>,
}

impl<'a> PgTransaction<'a> {
    // A stale cached plan (`0A000`) is NOT transparently retried inside a
    // transaction: the error aborts the transaction, so any subsequent
    // statement on it fails with `25P02 in_failed_sql_transaction`. Re-running
    // the one statement cannot succeed here. The error is still classified
    // retryable (via `classify_sqlstate`), so a caller that retries the whole
    // transaction recovers on a fresh statement. Only the non-transactional
    // `PgConnection` methods above self-heal in place.

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
        validate_savepoint_name(name)?;
        debug!(name = %name, "Creating savepoint");
        self.txn
            .batch_execute(&format!("SAVEPOINT {}", name))
            .await?;
        Ok(())
    }

    /// Rollback to a savepoint.
    pub async fn rollback_to(&mut self, name: &str) -> PgResult<()> {
        validate_savepoint_name(name)?;
        debug!(name = %name, "Rolling back to savepoint");
        self.txn
            .batch_execute(&format!("ROLLBACK TO SAVEPOINT {}", name))
            .await?;
        Ok(())
    }

    /// Release a savepoint.
    pub async fn release_savepoint(&mut self, name: &str) -> PgResult<()> {
        validate_savepoint_name(name)?;
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
    use super::*;

    // Integration tests would require a real PostgreSQL connection
    // Unit tests for connection wrapper are limited without mocking

    #[test]
    fn test_stale_cached_plan_message_gate() {
        // The exact PostgreSQL wording is recognized.
        assert!(is_stale_cached_plan_message(
            "db error: ERROR: cached plan must not change result type"
        ));
        // Other 0A000 (FEATURE_NOT_SUPPORTED) messages are not the stale-plan
        // case and must not trigger recovery.
        assert!(!is_stale_cached_plan_message(
            "ERROR: cannot insert into view \"v\""
        ));
        assert!(!is_stale_cached_plan_message("some unrelated error"));
    }

    #[test]
    fn test_validate_savepoint_name_accepts_valid_names() {
        assert!(validate_savepoint_name("sp1").is_ok());
        assert!(validate_savepoint_name("my_savepoint").is_ok());
        assert!(validate_savepoint_name("_private").is_ok());
        assert!(validate_savepoint_name("SP_2").is_ok());
        assert!(validate_savepoint_name("a").is_ok());
        // 63 chars (the max) is accepted
        let max_name = "a".repeat(MAX_SAVEPOINT_NAME_LEN);
        assert!(validate_savepoint_name(&max_name).is_ok());
    }

    #[test]
    fn test_validate_savepoint_name_rejects_invalid_names() {
        assert!(validate_savepoint_name("sp1; DROP TABLE").is_err());
        assert!(validate_savepoint_name("my savepoint").is_err());
        assert!(validate_savepoint_name("\"quoted\"").is_err());
        assert!(validate_savepoint_name("").is_err());
        assert!(validate_savepoint_name("1leading_digit").is_err());
        assert!(validate_savepoint_name("has-dash").is_err());
        assert!(validate_savepoint_name("has.dot").is_err());
        // 64 chars exceeds the limit
        let too_long = "a".repeat(MAX_SAVEPOINT_NAME_LEN + 1);
        assert!(validate_savepoint_name(&too_long).is_err());
    }
}
