//! DuckDB connection management.

use std::sync::Arc;

use duckdb::Connection;
use parking_lot::Mutex;
use serde_json::Value as JsonValue;
use tracing::{debug, instrument};

use crate::config::DuckDbConfig;
use crate::error::{DuckDbError, DuckDbResult};
use crate::types::{duckdb_value_ref_to_json, DuckDbParam};
use prax_query::filter::FilterValue;

/// A DuckDB connection wrapper.
///
/// DuckDB connections are not thread-safe by default, so we wrap them
/// in a Mutex for safe concurrent access.
#[derive(Clone)]
pub struct DuckDbConnection {
    conn: Arc<Mutex<Connection>>,
}

impl DuckDbConnection {
    /// Create a new connection from configuration.
    pub fn new(config: &DuckDbConfig) -> DuckDbResult<Self> {
        let conn = if config.is_in_memory() {
            Connection::open_in_memory()?
        } else {
            Connection::open(config.path_str())?
        };

        // Apply configuration settings
        Self::apply_config(&conn, config)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory connection.
    pub fn open_in_memory() -> DuckDbResult<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open a connection to a file.
    pub fn open(path: &str) -> DuckDbResult<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Apply configuration settings to the connection.
    fn apply_config(conn: &Connection, config: &DuckDbConfig) -> DuckDbResult<()> {
        // Set threads
        if let Some(threads) = config.threads {
            conn.execute(&format!("SET threads = {}", threads), [])?;
        }

        // Set memory limit
        if let Some(ref limit) = config.memory_limit {
            conn.execute(&format!("SET memory_limit = '{}'", limit), [])?;
        }

        // Set max memory
        if let Some(ref max) = config.max_memory {
            conn.execute(&format!("SET max_memory = '{}'", max), [])?;
        }

        // Set temp directory
        if let Some(ref temp_dir) = config.temp_directory {
            let path = temp_dir.to_string_lossy();
            conn.execute(&format!("SET temp_directory = '{}'", path), [])?;
        }

        // Enable/disable external access
        if !config.enable_external_access {
            conn.execute("SET enable_external_access = false", [])?;
        }

        // Enable/disable object cache
        if !config.enable_object_cache {
            conn.execute("SET enable_object_cache = false", [])?;
        }

        // Set default null order
        if let Some(ref order) = config.default_null_order {
            conn.execute(&format!("SET default_null_order = '{}'", order), [])?;
        }

        // Set default order
        if let Some(ref order) = config.default_order {
            conn.execute(&format!("SET default_order = '{}'", order), [])?;
        }

        // Enable/disable progress bar
        if config.enable_progress_bar {
            conn.execute("SET enable_progress_bar = true", [])?;
        }

        Ok(())
    }

    /// Execute a query and return all rows as JSON.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn query(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<Vec<JsonValue>> {
        debug!("Executing query");

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(sql)?;

        // Bind parameters
        let duckdb_params: Vec<DuckDbParam<'_>> = params.iter().map(DuckDbParam).collect();
        let param_refs: Vec<&dyn duckdb::ToSql> = duckdb_params
            .iter()
            .map(|p| p as &dyn duckdb::ToSql)
            .collect();

        let mut rows = stmt.query(param_refs.as_slice())?;

        // Get column names from the statement via rows.as_ref()
        let column_names: Vec<String> = rows
            .as_ref()
            .map(|stmt| stmt.column_names())
            .unwrap_or_default();

        let mut results = Vec::new();
        while let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();

            for (i, name) in column_names.iter().enumerate() {
                let value = row.get_ref(i)?;
                obj.insert(name.clone(), duckdb_value_ref_to_json(value));
            }

            results.push(JsonValue::Object(obj));
        }

        Ok(results)
    }

    /// Execute a query and return the first row.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn query_one(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<JsonValue> {
        let results = self.query(sql, params)?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| DuckDbError::query("Query returned no rows"))
    }

    /// Execute a query and return the first row or None.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn query_optional(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> DuckDbResult<Option<JsonValue>> {
        let results = self.query(sql, params)?;
        Ok(results.into_iter().next())
    }

    /// Execute a statement and return the number of affected rows.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn execute(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<usize> {
        debug!("Executing statement");

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(sql)?;

        let duckdb_params: Vec<DuckDbParam<'_>> = params.iter().map(DuckDbParam).collect();
        let param_refs: Vec<&dyn duckdb::ToSql> = duckdb_params
            .iter()
            .map(|p| p as &dyn duckdb::ToSql)
            .collect();

        let affected = stmt.execute(param_refs.as_slice())?;
        Ok(affected)
    }

    /// Execute a batch of SQL statements.
    #[instrument(skip(self), fields(sql_len = %sql.len()))]
    pub fn execute_batch(&self, sql: &str) -> DuckDbResult<()> {
        debug!("Executing batch");

        let conn = self.conn.lock();
        conn.execute_batch(sql)?;
        Ok(())
    }

    /// Execute an INSERT statement.
    ///
    /// Note: DuckDB doesn't have `last_insert_rowid()` like SQLite.
    /// Use `INSERT ... RETURNING id` if you need the inserted ID.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn insert(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<usize> {
        debug!("Executing insert");
        self.execute(sql, params)
    }

    /// Execute an INSERT with RETURNING clause to get inserted values.
    ///
    /// Example: `INSERT INTO users (name) VALUES (?) RETURNING id`
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub fn insert_returning(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> DuckDbResult<Vec<JsonValue>> {
        debug!("Executing insert with returning");
        self.query(sql, params)
    }

    /// Begin a transaction.
    pub fn begin_transaction(&self) -> DuckDbResult<()> {
        self.execute_batch("BEGIN TRANSACTION")
    }

    /// Commit a transaction.
    pub fn commit(&self) -> DuckDbResult<()> {
        self.execute_batch("COMMIT")
    }

    /// Rollback a transaction.
    pub fn rollback(&self) -> DuckDbResult<()> {
        self.execute_batch("ROLLBACK")
    }

    /// Create a savepoint.
    pub fn savepoint(&self, name: &str) -> DuckDbResult<()> {
        self.execute_batch(&format!("SAVEPOINT {}", name))
    }

    /// Release a savepoint.
    pub fn release_savepoint(&self, name: &str) -> DuckDbResult<()> {
        self.execute_batch(&format!("RELEASE SAVEPOINT {}", name))
    }

    /// Rollback to a savepoint.
    pub fn rollback_to_savepoint(&self, name: &str) -> DuckDbResult<()> {
        self.execute_batch(&format!("ROLLBACK TO SAVEPOINT {}", name))
    }

    // =========================================================================
    // DuckDB-specific analytical operations
    // =========================================================================

    /// Copy data to a Parquet file.
    #[instrument(skip(self), fields(query_len = %query.len()))]
    pub fn copy_to_parquet(&self, query: &str, path: &str) -> DuckDbResult<()> {
        let sql = format!("COPY ({}) TO '{}' (FORMAT PARQUET)", query, path);
        self.execute_batch(&sql)
    }

    /// Copy data to a CSV file.
    #[instrument(skip(self), fields(query_len = %query.len()))]
    pub fn copy_to_csv(&self, query: &str, path: &str, header: bool) -> DuckDbResult<()> {
        let sql = format!(
            "COPY ({}) TO '{}' (FORMAT CSV, HEADER {})",
            query,
            path,
            if header { "TRUE" } else { "FALSE" }
        );
        self.execute_batch(&sql)
    }

    /// Query a Parquet file.
    pub fn query_parquet(&self, path: &str) -> DuckDbResult<Vec<JsonValue>> {
        let sql = format!("SELECT * FROM read_parquet('{}')", path);
        self.query(&sql, &[])
    }

    /// Query a CSV file.
    pub fn query_csv(&self, path: &str, header: bool) -> DuckDbResult<Vec<JsonValue>> {
        let sql = format!(
            "SELECT * FROM read_csv('{}', header = {})",
            path,
            if header { "true" } else { "false" }
        );
        self.query(&sql, &[])
    }

    /// Query a JSON file.
    pub fn query_json(&self, path: &str) -> DuckDbResult<Vec<JsonValue>> {
        let sql = format!("SELECT * FROM read_json_auto('{}')", path);
        self.query(&sql, &[])
    }

    /// Get DuckDB version.
    pub fn version(&self) -> DuckDbResult<String> {
        let result = self.query_one("SELECT version()", &[])?;
        result
            .as_object()
            .and_then(|obj| obj.values().next())
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| DuckDbError::query("Failed to get version"))
    }

    /// Explain a query plan.
    pub fn explain(&self, query: &str) -> DuckDbResult<String> {
        let sql = format!("EXPLAIN {}", query);
        let results = self.query(&sql, &[])?;

        let mut plan = String::new();
        for row in results {
            if let Some(obj) = row.as_object() {
                for value in obj.values() {
                    if let Some(s) = value.as_str() {
                        plan.push_str(s);
                        plan.push('\n');
                    }
                }
            }
        }

        Ok(plan)
    }

    /// Explain a query plan with analysis.
    pub fn explain_analyze(&self, query: &str) -> DuckDbResult<String> {
        let sql = format!("EXPLAIN ANALYZE {}", query);
        let results = self.query(&sql, &[])?;

        let mut plan = String::new();
        for row in results {
            if let Some(obj) = row.as_object() {
                for value in obj.values() {
                    if let Some(s) = value.as_str() {
                        plan.push_str(s);
                        plan.push('\n');
                    }
                }
            }
        }

        Ok(plan)
    }
}

impl std::fmt::Debug for DuckDbConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbConnection").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let conn = DuckDbConnection::open_in_memory().unwrap();
        let version = conn.version().unwrap();
        assert!(!version.is_empty());
    }

    #[test]
    fn test_execute_batch() {
        let conn = DuckDbConnection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE test (id INTEGER, name VARCHAR);
             INSERT INTO test VALUES (1, 'Alice'), (2, 'Bob');",
        )
        .unwrap();

        let results = conn.query("SELECT * FROM test ORDER BY id", &[]).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_query_with_params() {
        let conn = DuckDbConnection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE users (id INTEGER, name VARCHAR);")
            .unwrap();
        conn.execute(
            "INSERT INTO users VALUES (?, ?)",
            &[FilterValue::Int(1), FilterValue::String("Alice".to_string())],
        )
        .unwrap();

        let results = conn
            .query(
                "SELECT * FROM users WHERE id = ?",
                &[FilterValue::Int(1)],
            )
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_transaction() {
        let conn = DuckDbConnection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE test (id INTEGER);")
            .unwrap();

        conn.begin_transaction().unwrap();
        conn.execute("INSERT INTO test VALUES (?)", &[FilterValue::Int(1)])
            .unwrap();
        conn.rollback().unwrap();

        let results = conn.query("SELECT * FROM test", &[]).unwrap();
        assert!(results.is_empty());
    }
}
