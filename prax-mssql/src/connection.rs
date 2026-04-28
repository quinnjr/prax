//! Microsoft SQL Server connection wrapper.

use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use tiberius::Row;
use tracing::{debug, trace};

use crate::error::{MssqlError, MssqlResult};

/// A wrapper around a SQL Server connection.
pub struct MssqlConnection<'a> {
    client: PooledConnection<'a, ConnectionManager>,
}

impl<'a> MssqlConnection<'a> {
    /// Create a new connection wrapper.
    pub(crate) fn new(client: PooledConnection<'a, ConnectionManager>) -> Self {
        Self { client }
    }

    /// Execute a query and return all rows.
    pub async fn query(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Vec<Row>> {
        trace!(sql = %sql, "Executing query");

        let stream = self.client.query(sql, params).await?;
        let rows = stream.into_first_result().await?;
        Ok(rows)
    }

    /// Execute a query and return exactly one row.
    pub async fn query_one(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Row> {
        trace!(sql = %sql, "Executing query_one");

        let mut rows = self.query(sql, params).await?;

        if rows.is_empty() {
            return Err(MssqlError::query("query returned no rows"));
        }

        Ok(rows.remove(0))
    }

    /// Execute a query and return zero or one row.
    pub async fn query_opt(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Option<Row>> {
        trace!(sql = %sql, "Executing query_opt");

        let mut rows = self.query(sql, params).await?;

        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(rows.remove(0)))
        }
    }

    /// Execute a statement and return the number of affected rows.
    pub async fn execute(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<u64> {
        trace!(sql = %sql, "Executing statement");

        let result = self.client.execute(sql, params).await?;
        Ok(result.total())
    }

    /// Execute a batch of statements (separated by GO or semicolons).
    pub async fn batch_execute(&mut self, sql: &str) -> MssqlResult<()> {
        trace!(sql = %sql, "Executing batch");
        self.client.simple_query(sql).await?.into_results().await?;
        Ok(())
    }

    /// Begin a transaction.
    pub async fn begin_transaction(&mut self) -> MssqlResult<()> {
        debug!("Beginning transaction");
        self.client
            .simple_query("BEGIN TRANSACTION")
            .await?
            .into_results()
            .await?;
        Ok(())
    }

    /// Commit the current transaction.
    pub async fn commit(&mut self) -> MssqlResult<()> {
        debug!("Committing transaction");
        self.client
            .simple_query("COMMIT")
            .await?
            .into_results()
            .await?;
        Ok(())
    }

    /// Rollback the current transaction.
    pub async fn rollback(&mut self) -> MssqlResult<()> {
        debug!("Rolling back transaction");
        self.client
            .simple_query("ROLLBACK")
            .await?
            .into_results()
            .await?;
        Ok(())
    }

    /// Create a savepoint.
    pub async fn savepoint(&mut self, name: &str) -> MssqlResult<()> {
        debug!(name = %name, "Creating savepoint");
        self.client
            .simple_query(&format!("SAVE TRANSACTION {}", name))
            .await?
            .into_results()
            .await?;
        Ok(())
    }

    /// Rollback to a savepoint.
    pub async fn rollback_to(&mut self, name: &str) -> MssqlResult<()> {
        debug!(name = %name, "Rolling back to savepoint");
        self.client
            .simple_query(&format!("ROLLBACK TRANSACTION {}", name))
            .await?
            .into_results()
            .await?;
        Ok(())
    }

    /// Set a session context value (for RLS).
    ///
    /// This sets a value that can be read by RLS predicate functions using
    /// `SESSION_CONTEXT(N'key')`.
    pub async fn set_session_context(&mut self, key: &str, value: &str) -> MssqlResult<()> {
        debug!(key = %key, "Setting session context");
        let sql = format!(
            "EXEC sp_set_session_context @key = N'{}', @value = N'{}'",
            key.replace('\'', "''"),
            value.replace('\'', "''")
        );
        self.client.simple_query(&sql).await?.into_results().await?;
        Ok(())
    }

    /// Set a session context value with read-only option.
    pub async fn set_session_context_readonly(
        &mut self,
        key: &str,
        value: &str,
        read_only: bool,
    ) -> MssqlResult<()> {
        debug!(key = %key, read_only = %read_only, "Setting session context");
        let sql = format!(
            "EXEC sp_set_session_context @key = N'{}', @value = N'{}', @read_only = {}",
            key.replace('\'', "''"),
            value.replace('\'', "''"),
            if read_only { 1 } else { 0 }
        );
        self.client.simple_query(&sql).await?.into_results().await?;
        Ok(())
    }

    /// Get a session context value.
    pub async fn get_session_context(&mut self, key: &str) -> MssqlResult<Option<String>> {
        let sql = format!("SELECT CAST(SESSION_CONTEXT(N'{}') AS NVARCHAR(MAX))", key);
        let rows = self.query(&sql, &[]).await?;

        if let Some(row) = rows.first() {
            let value: Option<&str> = row.get(0);
            Ok(value.map(String::from))
        } else {
            Ok(None)
        }
    }

    /// Check if RLS is enabled on a table.
    pub async fn is_rls_enabled(&mut self, schema: &str, table: &str) -> MssqlResult<bool> {
        let sql = r#"
            SELECT COUNT(*)
            FROM sys.security_policies sp
            JOIN sys.security_predicates pred ON sp.object_id = pred.object_id
            JOIN sys.objects o ON pred.target_object_id = o.object_id
            JOIN sys.schemas s ON o.schema_id = s.schema_id
            WHERE s.name = @P1 AND o.name = @P2 AND sp.is_enabled = 1
        "#;

        let rows = self.query(sql, &[&schema, &table]).await?;

        if let Some(row) = rows.first() {
            let count: i32 = row.get(0).unwrap_or(0);
            Ok(count > 0)
        } else {
            Ok(false)
        }
    }

    /// Get the underlying client reference.
    pub fn inner(&mut self) -> &mut PooledConnection<'a, ConnectionManager> {
        &mut self.client
    }
}

/// A SQL Server transaction wrapper.
pub struct MssqlTransaction<'a> {
    conn: &'a mut MssqlConnection<'a>,
    committed: bool,
}

impl<'a> MssqlTransaction<'a> {
    /// Create a new transaction.
    pub async fn new(conn: &'a mut MssqlConnection<'a>) -> MssqlResult<Self> {
        conn.begin_transaction().await?;
        Ok(Self {
            conn,
            committed: false,
        })
    }

    /// Execute a query and return all rows.
    pub async fn query(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Vec<Row>> {
        self.conn.query(sql, params).await
    }

    /// Execute a query and return exactly one row.
    pub async fn query_one(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Row> {
        self.conn.query_one(sql, params).await
    }

    /// Execute a query and return zero or one row.
    pub async fn query_opt(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<Option<Row>> {
        self.conn.query_opt(sql, params).await
    }

    /// Execute a statement and return the number of affected rows.
    pub async fn execute(
        &mut self,
        sql: &str,
        params: &[&dyn tiberius::ToSql],
    ) -> MssqlResult<u64> {
        self.conn.execute(sql, params).await
    }

    /// Create a savepoint.
    pub async fn savepoint(&mut self, name: &str) -> MssqlResult<()> {
        self.conn.savepoint(name).await
    }

    /// Rollback to a savepoint.
    pub async fn rollback_to(&mut self, name: &str) -> MssqlResult<()> {
        self.conn.rollback_to(name).await
    }

    /// Commit the transaction.
    pub async fn commit(mut self) -> MssqlResult<()> {
        self.conn.commit().await?;
        self.committed = true;
        Ok(())
    }

    /// Rollback the transaction.
    pub async fn rollback(mut self) -> MssqlResult<()> {
        self.conn.rollback().await?;
        self.committed = true; // Mark as handled
        Ok(())
    }
}

impl<'a> Drop for MssqlTransaction<'a> {
    fn drop(&mut self) {
        if !self.committed {
            // Transaction was not committed or explicitly rolled back
            // We can't do async rollback in drop, so log a warning
            tracing::warn!("Transaction dropped without commit or rollback");
        }
    }
}

#[cfg(test)]
mod tests {
    // Connection tests require integration testing with a real SQL Server
}
