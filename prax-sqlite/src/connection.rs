//! SQLite connection wrapper.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tokio::sync::OwnedSemaphorePermit;
use tokio_rusqlite::Connection;
use tracing::{debug, trace};

use crate::error::{SqliteError, SqliteResult};

/// A pooled connection for returning to the pool.
pub(crate) struct PooledConnection {
    /// The underlying connection.
    pub conn: Connection,
    /// When this connection was created.
    pub created_at: Instant,
    /// When this connection was last used.
    pub last_used: Instant,
}

impl PooledConnection {
    pub fn new(conn: Connection) -> Self {
        let now = Instant::now();
        Self {
            conn,
            created_at: now,
            last_used: now,
        }
    }
}

/// A wrapper around a SQLite connection.
pub struct SqliteConnection {
    conn: Option<Connection>,
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
    /// Channel to return the connection to the pool.
    return_to_pool: Option<Arc<Mutex<VecDeque<PooledConnection>>>>,
    /// When this connection was created (for pool tracking).
    created_at: Instant,
}

impl SqliteConnection {
    /// Create a new connection wrapper (non-pooled).
    pub fn new(conn: Connection, permit: OwnedSemaphorePermit) -> Self {
        Self {
            conn: Some(conn),
            permit,
            return_to_pool: None,
            created_at: Instant::now(),
        }
    }

    /// Create a new pooled connection wrapper.
    pub(crate) fn new_pooled(
        conn: Connection,
        permit: OwnedSemaphorePermit,
        return_to_pool: Option<Arc<Mutex<VecDeque<PooledConnection>>>>,
    ) -> Self {
        Self {
            conn: Some(conn),
            permit,
            return_to_pool,
            created_at: Instant::now(),
        }
    }

    /// Get the inner connection reference.
    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("Connection already taken")
    }

    /// Execute a query and return all rows as JSON values.
    pub async fn query(&self, sql: &str) -> SqliteResult<Vec<serde_json::Value>> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing query");

        self.conn()
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();

                let rows = stmt.query_map([], |row| {
                    let mut map = serde_json::Map::new();
                    for (i, col) in columns.iter().enumerate() {
                        let value = crate::types::get_value_at_index(row, i);
                        map.insert(col.clone(), value);
                    }
                    Ok(serde_json::Value::Object(map))
                })?;

                let results: Result<Vec<_>, _> = rows.collect();
                Ok(results?)
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a query with parameters and return all rows.
    pub async fn query_params(
        &self,
        sql: &str,
        params: Vec<rusqlite::types::Value>,
    ) -> SqliteResult<Vec<serde_json::Value>> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing parameterized query");

        self.conn()
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();

                let params_ref: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

                let rows = stmt.query_map(params_ref.as_slice(), |row| {
                    let mut map = serde_json::Map::new();
                    for (i, col) in columns.iter().enumerate() {
                        let value = crate::types::get_value_at_index(row, i);
                        map.insert(col.clone(), value);
                    }
                    Ok(serde_json::Value::Object(map))
                })?;

                let results: Result<Vec<_>, _> = rows.collect();
                Ok(results?)
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a query and return a single row.
    pub async fn query_one(&self, sql: &str) -> SqliteResult<serde_json::Value> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing query_one");

        self.conn()
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();

                Ok(stmt.query_row([], |row| {
                    let mut map = serde_json::Map::new();
                    for (i, col) in columns.iter().enumerate() {
                        let value = crate::types::get_value_at_index(row, i);
                        map.insert(col.clone(), value);
                    }
                    Ok(serde_json::Value::Object(map))
                })?)
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a query and return an optional row.
    pub async fn query_optional(&self, sql: &str) -> SqliteResult<Option<serde_json::Value>> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing query_optional");

        self.conn()
            .call(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let columns: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();

                let result = stmt.query_row([], |row| {
                    let mut map = serde_json::Map::new();
                    for (i, col) in columns.iter().enumerate() {
                        let value = crate::types::get_value_at_index(row, i);
                        map.insert(col.clone(), value);
                    }
                    Ok(serde_json::Value::Object(map))
                });

                match result {
                    Ok(row) => Ok(Some(row)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(tokio_rusqlite::Error::Rusqlite(e)),
                }
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a statement and return the number of affected rows.
    pub async fn execute(&self, sql: &str) -> SqliteResult<usize> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing statement");

        self.conn()
            .call(move |conn| Ok(conn.execute(&sql, [])?))
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a statement with parameters and return the number of affected rows.
    pub async fn execute_params(
        &self,
        sql: &str,
        params: Vec<rusqlite::types::Value>,
    ) -> SqliteResult<usize> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing parameterized statement");

        self.conn()
            .call(move |conn| {
                let params_ref: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                Ok(conn.execute(&sql, params_ref.as_slice())?)
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a statement and return the last insert rowid.
    pub async fn execute_insert(&self, sql: &str) -> SqliteResult<i64> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing insert");

        self.conn()
            .call(move |conn| {
                conn.execute(&sql, [])?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute a statement with parameters and return the last insert rowid.
    pub async fn execute_insert_params(
        &self,
        sql: &str,
        params: Vec<rusqlite::types::Value>,
    ) -> SqliteResult<i64> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing parameterized insert");

        self.conn()
            .call(move |conn| {
                let params_ref: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                conn.execute(&sql, params_ref.as_slice())?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .map_err(SqliteError::from)
    }

    /// Execute multiple statements in a batch.
    pub async fn execute_batch(&self, sql: &str) -> SqliteResult<()> {
        let sql = sql.to_string();
        trace!(sql = %sql, "Executing batch");

        self.conn()
            .call(move |conn| Ok(conn.execute_batch(&sql)?))
            .await
            .map_err(SqliteError::from)
    }

    /// Get the inner connection.
    pub fn inner(&self) -> &Connection {
        self.conn()
    }
}

impl Drop for SqliteConnection {
    fn drop(&mut self) {
        // Return the connection to the pool if possible
        if let Some(pool) = self.return_to_pool.take() {
            if let Some(conn) = self.conn.take() {
                trace!("Returning connection to pool");
                let mut idle: parking_lot::MutexGuard<'_, VecDeque<PooledConnection>> = pool.lock();
                idle.push_back(PooledConnection {
                    conn,
                    created_at: self.created_at,
                    last_used: Instant::now(),
                });
            }
        }
        // Otherwise, the connection is just dropped
    }
}
